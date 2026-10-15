use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use zectrix_eink_calendar::{
    CalendarConfig, CalendarState, DEFAULT_DEVICE_ID, DEFAULT_PAGE_ID, FramePublisher,
    PublishDecision, ZectrixPublisher, render_calendar, render_calendar_for_shanghai_epoch,
    seconds_until_next_shanghai_midnight, shanghai_date_from_epoch, validate_calendar_page_id,
};

/// 守护进程运行与健康监控共享状态（供外部 HTTP 控制探针与 CLI 探活实时读取）
struct RuntimeControl {
    started_at: u64,
    device_id: String,
    allow_page_3: bool,
    page_id: AtomicU8,
    last_ok_at: AtomicU64,
    push_success_count: AtomicU64,
    push_fail_streak: AtomicU64,
    max_fail_streak: u64,
    force_refresh: AtomicBool,
    last_pushed_date: Mutex<String>,
    last_frame_hash: Mutex<Option<String>>,
    wake_signal: (Mutex<()>, Condvar),
}

impl RuntimeControl {
    fn new(device_id: String, page_id: u8, allow_page_3: bool, max_fail_streak: u64) -> Arc<Self> {
        let now = unix_time_u64();
        Arc::new(Self {
            started_at: now,
            device_id,
            allow_page_3,
            page_id: AtomicU8::new(page_id),
            last_ok_at: AtomicU64::new(0),
            push_success_count: AtomicU64::new(0),
            push_fail_streak: AtomicU64::new(0),
            max_fail_streak,
            force_refresh: AtomicBool::new(false),
            last_pushed_date: Mutex::new(String::new()),
            last_frame_hash: Mutex::new(None),
            wake_signal: (Mutex::new(()), Condvar::new()),
        })
    }

    fn trigger_immediate_refresh(&self) {
        self.force_refresh.store(true, Ordering::SeqCst);
        let (lock, cvar) = &self.wake_signal;
        if let Ok(_guard) = lock.lock() {
            cvar.notify_all();
        }
    }
}

fn main() {
    if let Err(error) = dispatch_cli() {
        eprintln!("ZECTRIX 月历服务异常退出：{error}");
        std::process::exit(1);
    }
}

fn dispatch_cli() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();

    // 1. 本地预览模式：zectrix_calendar_companion --preview [YYYY-MM-DD] [output.png]
    if let Some(pos) = args.iter().position(|a| a == "--preview") {
        let date_arg = args.get(pos + 1).map(String::as_str);
        let out_path = args
            .get(pos + 2)
            .map(String::as_str)
            .unwrap_or("calendar_preview.png");
        let output = if let Some(d_str) = date_arg
            && !d_str.ends_with(".png")
        {
            let date = NaiveDate::parse_from_str(d_str, "%Y-%m-%d")?;
            render_calendar(
                CalendarState::from_naive_date(date, DEFAULT_PAGE_ID),
                CalendarConfig::default(),
            )?
        } else {
            let target_file = date_arg.unwrap_or(out_path);
            let now = unix_time_seconds()?;
            let out =
                render_calendar_for_shanghai_epoch(now, DEFAULT_PAGE_ID, CalendarConfig::default())?;
            out.frame.write_png(target_file)?;
            println!("已生成 400x300 墨水屏月历预览图：{target_file}");
            return Ok(());
        };
        output.frame.write_png(out_path)?;
        println!("已生成 400x300 墨水屏月历预览图：{out_path}");
        return Ok(());
    }

    // 2. 外部健康检查命令：zectrix_calendar_companion --check
    if args.iter().any(|a| a == "--check") {
        return query_local_control("GET /healthz");
    }

    // 3. 外部立即刷新控制命令：zectrix_calendar_companion --trigger-refresh
    if args.iter().any(|a| a == "--trigger-refresh") {
        return query_local_control("POST /refresh");
    }

    let run_once = args.iter().any(|a| a == "--once");
    run_daemon(run_once)
}

/// 通过本机控制端口发送外部探活或即时重绘指令
fn query_local_control(request_line: &str) -> Result<(), Box<dyn Error>> {
    let addr = env_value("CONTROL_LISTEN", "127.0.0.1:9596");
    let mut stream = TcpStream::connect_timeout(&addr.parse()?, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "{request_line} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status_line = response.lines().next().unwrap_or("");
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
    println!("{body}");
    if status_line.contains("200 OK") {
        Ok(())
    } else {
        Err(format!("服务状态异常：{status_line}").into())
    }
}

/// 启动外部控制与健康监控 HTTP 服务（默认 127.0.0.1:9596；设为空则关闭）：
/// - `GET /healthz` 或 `GET /status`：返回运行状态、最近推送日期、距离零点秒数与连续失败次数（正常 200，异常 503）
/// - `POST /refresh` 或 `GET /refresh`：外部立即触发重绘并推送上屏
/// - `POST /page?id=2`：动态切换推送页面通道（1/2/4/5，保护第3页）并立即刷新
/// - `GET /preview.png`：实时返回当前 400x300 PNG 图像流供远程查看
fn spawn_control_server(ctrl: Arc<RuntimeControl>) -> Option<thread::JoinHandle<()>> {
    let addr = match std::env::var("CONTROL_LISTEN") {
        Ok(v) if v.trim().is_empty() => return None,
        Ok(v) => v,
        Err(_) => "127.0.0.1:9596".to_owned(),
    };
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("控制与健康探针=绑定失败 {addr}: {e}");
            return None;
        }
    };
    eprintln!("控制与健康探针=监听 http://{addr} (/healthz, /refresh, /page?id=N, /preview.png)");
    Some(thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle_control_client(stream, &ctrl);
        }
    }))
}

fn handle_control_client(mut stream: TcpStream, ctrl: &Arc<RuntimeControl>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0_u8; 1024];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

    if path.starts_with("/healthz") || path.starts_with("/status") {
        let now_u64 = unix_time_u64();
        let now_i64 = now_u64 as i64;
        let cur_date = shanghai_date_from_epoch(now_i64)
            .map(|d| d.to_string())
            .unwrap_or_default();
        let pushed_date = ctrl
            .last_pushed_date
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let last_hash = ctrl
            .last_frame_hash
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        let last_ok = ctrl.last_ok_at.load(Ordering::Relaxed);
        let fails = ctrl.push_fail_streak.load(Ordering::Relaxed);
        let ok_cnt = ctrl.push_success_count.load(Ordering::Relaxed);
        let page_id = ctrl.page_id.load(Ordering::Relaxed);
        let until_midnight = seconds_until_next_shanghai_midnight(now_i64);

        let healthy = fails < ctrl.max_fail_streak && !pushed_date.is_empty();
        let code = if healthy { 200 } else { 503 };
        let status_str = if healthy { "ok" } else { "degraded" };
        let body = format!(
            "{{\"status\":\"{status_str}\",\"healthy\":{healthy},\"device_id\":\"{}\",\"page_id\":{page_id},\"timezone\":\"Asia/Shanghai (UTC+8)\",\"current_shanghai_date\":\"{cur_date}\",\"last_pushed_date\":\"{pushed_date}\",\"last_frame_sha256\":\"{last_hash}\",\"uptime_seconds\":{},\"last_ok_age_seconds\":{},\"next_midnight_refresh_seconds\":{until_midnight},\"push_success_count\":{ok_cnt},\"push_fail_streak\":{fails}}}",
            ctrl.device_id,
            now_u64.saturating_sub(ctrl.started_at),
            if last_ok == 0 {
                u64::MAX
            } else {
                now_u64.saturating_sub(last_ok)
            },
        );
        write_http_response(&mut stream, code, "application/json", body.as_bytes());
    } else if path.starts_with("/refresh") {
        ctrl.trigger_immediate_refresh();
        let body = br#"{"status":"triggered","message":"immediate calendar refresh scheduled"}"#;
        write_http_response(&mut stream, 200, "application/json", body);
    } else if let Some(query) = path.strip_prefix("/page?id=") {
        let parsed = query
            .split('&')
            .next()
            .unwrap_or("")
            .parse::<u8>()
            .ok()
            .and_then(|pid| validate_calendar_page_id(pid, ctrl.allow_page_3).ok());
        if let Some(new_page) = parsed {
            ctrl.page_id.store(new_page, Ordering::SeqCst);
            if let Ok(mut guard) = ctrl.last_frame_hash.lock() {
                *guard = None;
            }
            ctrl.trigger_immediate_refresh();
            let body = format!("{{\"status\":\"ok\",\"page_id\":{new_page}}}");
            write_http_response(&mut stream, 200, "application/json", body.as_bytes());
        } else {
            let body = br#"{"status":"error","message":"invalid or reserved page_id (allowed: 1,2,4,5)"}"#;
            write_http_response(&mut stream, 400, "application/json", body);
        }
    } else if path.starts_with("/preview.png") {
        let now_i64 = unix_time_u64() as i64;
        let page_id = ctrl.page_id.load(Ordering::Relaxed);
        if let Ok(out) = render_calendar_for_shanghai_epoch(now_i64, page_id, CalendarConfig::default())
            && let Ok(png) = out.frame.png_bytes()
        {
            write_http_response(&mut stream, 200, "image/png", &png);
        } else {
            write_http_response(&mut stream, 500, "text/plain", b"render error");
        }
    } else {
        write_http_response(
            &mut stream,
            404,
            "application/json",
            br#"{"status":"not_found"}"#,
        );
    }
}

fn write_http_response(stream: &mut TcpStream, code: u16, content_type: &str, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

fn run_daemon(run_once: bool) -> Result<(), Box<dyn Error>> {
    let api_key = read_api_key()?;
    let device_id = env_value("ZECTRIX_DEVICE_ID", DEFAULT_DEVICE_ID);
    let initial_page_id: u8 = env_value("ZECTRIX_PAGE_ID", &DEFAULT_PAGE_ID.to_string()).parse()?;
    let allow_page_3 = env_value("ALLOW_PAGE_3_OVERRIDE", "false").eq_ignore_ascii_case("true");
    validate_calendar_page_id(initial_page_id, allow_page_3)?;

    let base_url = env_value("ZECTRIX_API_BASE", "https://cloud.zectrix.com");
    let preview_output = std::env::var("CALENDAR_PREVIEW_PATH").ok();
    let max_fail_streak: u64 = env_value("CALENDAR_MAX_FAIL_STREAK", "5")
        .parse()
        .unwrap_or(5)
        .max(1);

    let ctrl = RuntimeControl::new(
        device_id.clone(),
        initial_page_id,
        allow_page_3,
        max_fail_streak,
    );
    let _control_handle = if !run_once {
        spawn_control_server(Arc::clone(&ctrl))
    } else {
        None
    };

    eprintln!(
        "ZECTRIX 月历守护服务已启动：设备={device_id} 通道=Page {initial_page_id} 时区=Asia/Shanghai(UTC+8)"
    );

    loop {
        let now = unix_time_seconds()?;
        let shanghai_date = shanghai_date_from_epoch(now)?;
        let active_page_id = ctrl.page_id.load(Ordering::SeqCst);
        let forced = ctrl.force_refresh.swap(false, Ordering::SeqCst);
        let prev_hash = if forced {
            None
        } else {
            ctrl.last_frame_hash.lock().ok().and_then(|g| g.clone())
        };

        let output = render_calendar_for_shanghai_epoch(
            now,
            active_page_id,
            CalendarConfig {
                previous_frame_hash: prev_hash,
            },
        )?;

        if let Some(path) = preview_output.as_deref()
            && !path.trim().is_empty()
        {
            let _ = output.frame.write_png(path);
        }

        let mut push_succeeded = true;
        if output.publish_decision == PublishDecision::Publish || forced {
            let mut publisher = ZectrixPublisher::new(
                &api_key,
                &base_url,
                &device_id,
                active_page_id,
                allow_page_3,
            )?;
            match publisher.publish(&output) {
                Ok(()) => {
                    eprintln!(
                        "月历推送=成功 日期={shanghai_date} 通道=Page {active_page_id} sha256={}",
                        &output.frame.sha256[..12]
                    );
                    ctrl.last_ok_at.store(unix_time_u64(), Ordering::Relaxed);
                    ctrl.push_success_count.fetch_add(1, Ordering::Relaxed);
                    ctrl.push_fail_streak.store(0, Ordering::Relaxed);
                    if let Ok(mut d_guard) = ctrl.last_pushed_date.lock() {
                        *d_guard = shanghai_date.to_string();
                    }
                    if let Ok(mut h_guard) = ctrl.last_frame_hash.lock() {
                        *h_guard = Some(output.frame.sha256);
                    }
                }
                Err(err) => {
                    push_succeeded = false;
                    let streak = ctrl.push_fail_streak.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!(
                        "月历推送=失败({streak}/{max_fail_streak}) 日期={shanghai_date} 原因={err}"
                    );
                    if streak >= max_fail_streak && !run_once {
                        return Err(
                            format!("连续推送失败达 {streak} 次，触发进程自愈重启").into()
                        );
                    }
                }
            }
        } else {
            ctrl.last_ok_at.store(unix_time_u64(), Ordering::Relaxed);
            eprintln!("月历推送=跳过（画面未变化） 日期={shanghai_date}");
        }

        if run_once {
            if !push_succeeded {
                return Err("单次执行模式下月历推送失败".into());
            }
            return Ok(());
        }

        let sleep_seconds = if push_succeeded {
            let until_midnight = seconds_until_next_shanghai_midnight(unix_time_seconds()?);
            eprintln!("下次自动重绘将在北京时间 24:00（{until_midnight} 秒后）触发");
            until_midnight
        } else {
            60
        };

        // 支持外部即时唤醒（/refresh 或 /page）与 30 秒分段跨天时钟校验
        let mut remaining = sleep_seconds;
        while remaining > 0 && !ctrl.force_refresh.load(Ordering::SeqCst) {
            let step = remaining.min(30);
            let (lock, cvar) = &ctrl.wake_signal;
            if let Ok(guard) = lock.lock() {
                let _ = cvar.wait_timeout(guard, Duration::from_secs(step));
            }
            remaining = remaining.saturating_sub(step);
            if push_succeeded
                && let Ok(current_now) = unix_time_seconds()
                && let Ok(current_date) = shanghai_date_from_epoch(current_now)
                && current_date != shanghai_date
            {
                break;
            }
        }
    }
}

fn read_api_key() -> Result<String, Box<dyn Error>> {
    if let Ok(key) = std::env::var("ZECTRIX_API_KEY")
        && !key.trim().is_empty()
    {
        return Ok(key.trim().to_owned());
    }
    if let Ok(path) = std::env::var("ZECTRIX_API_KEY_FILE")
        && !path.trim().is_empty()
    {
        let content = fs::read_to_string(path)?;
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }
    }
    Err("请设置 ZECTRIX_API_KEY 或 ZECTRIX_API_KEY_FILE 环境变量".into())
}

fn env_value(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn unix_time_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn unix_time_seconds() -> Result<i64, Box<dyn Error>> {
    Ok(unix_time_u64().try_into()?)
}