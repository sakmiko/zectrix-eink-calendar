pub mod publisher;

use std::io::Cursor;
use std::path::Path;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc};
use image::GrayImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use publisher::{
    CODEX_RESERVED_PAGE_ID, DEFAULT_DEVICE_ID, DEFAULT_PAGE_ID, FramePublisher,
    ZectrixPublishError, ZectrixPublisher, validate_calendar_page_id,
};

pub const DISPLAY_WIDTH: u32 = 400;
pub const DISPLAY_HEIGHT: u32 = 300;
pub const SHANGHAI_UTC_OFFSET_SECONDS: i32 = 8 * 3600;

/// 1900-2100 年农历数据表（共 201 年）
const LUNAR_INFO: [u32; 201] = [
    0x04bd8, 0x04ae0, 0x0a570, 0x054d5, 0x0d260, 0x0d950, 0x16554, 0x056a0, 0x09ad0, 0x055d2,
    0x04ae0, 0x0a5b6, 0x0a4d0, 0x0d250, 0x1d255, 0x0b540, 0x0d6a0, 0x0ada2, 0x095b0, 0x14977,
    0x04970, 0x0a4b0, 0x0b4b5, 0x06a50, 0x06d40, 0x1ab54, 0x02b60, 0x09570, 0x052f2, 0x04970,
    0x06566, 0x0d4a0, 0x0ea50, 0x06e95, 0x05ad0, 0x02b60, 0x186e3, 0x092e0, 0x1c8d7, 0x0c950,
    0x0d4a0, 0x1d8a6, 0x0b550, 0x056a0, 0x1a5b4, 0x025d0, 0x092d0, 0x0d2b2, 0x0a950, 0x0b557,
    0x06ca0, 0x0b550, 0x15355, 0x04da0, 0x0a5b0, 0x14573, 0x052b0, 0x0a9a8, 0x0e950, 0x06aa0,
    0x0aea6, 0x0ab50, 0x04b60, 0x0aae4, 0x0a570, 0x05260, 0x0f263, 0x0d950, 0x05b57, 0x056a0,
    0x096d0, 0x04dd5, 0x04ad0, 0x0a4d0, 0x0d4d4, 0x0d250, 0x0d558, 0x0b540, 0x0b6a0, 0x195a6,
    0x095b0, 0x049b0, 0x0a974, 0x0a4b0, 0x0b27a, 0x06a50, 0x06d40, 0x0af46, 0x0ab60, 0x09570,
    0x04af5, 0x04970, 0x064b0, 0x074a3, 0x0ea50, 0x06b58, 0x05ac0, 0x0ab60, 0x096d5, 0x092e0,
    0x0c960, 0x0d954, 0x0d4a0, 0x0da50, 0x07552, 0x056a0, 0x0abb7, 0x025d0, 0x092d0, 0x0cab5,
    0x0a950, 0x0b4a0, 0x0baa4, 0x0ad50, 0x055d9, 0x04ba0, 0x0a5b0, 0x15176, 0x052b0, 0x0a930,
    0x07954, 0x06aa0, 0x0ad50, 0x05b52, 0x04b60, 0x0a6e6, 0x0a4e0, 0x0d260, 0x0ea65, 0x0d530,
    0x05aa0, 0x076a3, 0x096d0, 0x04afb, 0x04ad0, 0x0a4d0, 0x1d0b6, 0x0d250, 0x0d520, 0x0dd45,
    0x0b5a0, 0x056d0, 0x055b2, 0x049b0, 0x0a577, 0x0a4b0, 0x0aa50, 0x1b255, 0x06d20, 0x0ada0,
    0x14b63, 0x09370, 0x049f8, 0x04970, 0x064b0, 0x168a6, 0x0ea50, 0x06b20, 0x1a6c4, 0x0aae0,
    0x092e0, 0x0d2e3, 0x0c960, 0x0d557, 0x0d4a0, 0x0da50, 0x05d55, 0x056a0, 0x0a6d0, 0x055d4,
    0x052d0, 0x0a9b8, 0x0a950, 0x0b4a0, 0x0b6a6, 0x0ad50, 0x055a0, 0x0aba4, 0x0a5b0, 0x052b0,
    0x0b273, 0x06930, 0x07337, 0x06aa0, 0x0ad50, 0x14b55, 0x04b60, 0x0a570, 0x054e4, 0x0d160,
    0x0e968, 0x0d520, 0x0daa0, 0x16aa6, 0x056d0, 0x04ae0, 0x0a9d4, 0x0a2d0, 0x0d150, 0x0f252,
    0x0d520,
];

const TIAN_GAN: [&str; 10] = ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"];
const DI_ZHI: [&str; 12] = [
    "子", "丑", "寅", "卯", "辰", "巳", "午", "未", "申", "酉", "戌", "亥",
];
const ZODIAC: [&str; 12] = [
    "鼠", "牛", "虎", "兔", "龙", "蛇", "马", "羊", "猴", "鸡", "狗", "猪",
];
const LUNAR_MONTH_NAMES: [&str; 12] = [
    "正", "二", "三", "四", "五", "六", "七", "八", "九", "十", "冬", "腊",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LunarDateInfo {
    pub lunar_year: i32,
    pub lunar_month: u32,
    pub lunar_day: u32,
    pub is_leap_month: bool,
    pub is_big_month: bool,
    pub ganzhi_year: String,
    pub zodiac: &'static str,
    pub ganzhi_day: String,
    pub month_text: String,
    pub day_text: String,
    pub moon_phase: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CalendarState {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub page_id: u8,
}

impl CalendarState {
    pub fn from_naive_date(date: NaiveDate, page_id: u8) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
            day: date.day(),
            page_id,
        }
    }

    pub fn naive_date(&self) -> Result<NaiveDate, CalendarError> {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day).ok_or_else(|| {
            CalendarError::InvalidDate(format!("{}-{:02}-{:02}", self.year, self.month, self.day))
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CalendarConfig {
    pub previous_frame_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishDecision {
    Publish,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonochromeFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub sha256: String,
}

impl MonochromeFrame {
    pub fn png_bytes(&self) -> Result<Vec<u8>, CalendarError> {
        let image = GrayImage::from_raw(self.width, self.height, self.pixels.clone())
            .ok_or(CalendarError::InvalidFrame)?;
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image)
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(CalendarError::Image)?;
        Ok(output.into_inner())
    }

    pub fn write_png(&self, path: impl AsRef<Path>) -> Result<(), CalendarError> {
        std::fs::write(path, self.png_bytes()?)
            .map_err(image::ImageError::IoError)
            .map_err(CalendarError::Image)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarOutput {
    pub state: CalendarState,
    pub frame: MonochromeFrame,
    pub visible_text: Vec<String>,
    pub publish_decision: PublishDecision,
}

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("画布尺寸无效")]
    InvalidFrame,
    #[error("写入或编码 PNG 失败：{0}")]
    Image(#[source] image::ImageError),
    #[error("日期无效：{0}")]
    InvalidDate(String),
}

/// 将 Unix 秒时间戳严格转换为中国标准时间（UTC+8 / Asia/Shanghai）的公历日期
pub fn shanghai_date_from_epoch(epoch_seconds: i64) -> Result<NaiveDate, CalendarError> {
    let tz = FixedOffset::east_opt(SHANGHAI_UTC_OFFSET_SECONDS).unwrap();
    let utc = DateTime::<Utc>::from_timestamp(epoch_seconds, 0)
        .ok_or_else(|| CalendarError::InvalidDate(epoch_seconds.to_string()))?;
    Ok(utc.with_timezone(&tz).date_naive())
}

/// 计算距离下一个北京时间 24:00（次日 00:00:01 UTC+8）的精确秒数
pub fn seconds_until_next_shanghai_midnight(now_epoch_seconds: i64) -> u64 {
    let tz = FixedOffset::east_opt(SHANGHAI_UTC_OFFSET_SECONDS).unwrap();
    let Some(utc) = DateTime::<Utc>::from_timestamp(now_epoch_seconds, 0) else {
        return 60;
    };
    let shanghai_now = utc.with_timezone(&tz);
    let elapsed_today = u64::from(shanghai_now.hour()) * 3600
        + u64::from(shanghai_now.minute()) * 60
        + u64::from(shanghai_now.second());
    86_400_u64.saturating_sub(elapsed_today).saturating_add(1)
}

pub fn render_calendar_for_shanghai_epoch(
    now_epoch_seconds: i64,
    page_id: u8,
    config: CalendarConfig,
) -> Result<CalendarOutput, CalendarError> {
    let date = shanghai_date_from_epoch(now_epoch_seconds)?;
    render_calendar(CalendarState::from_naive_date(date, page_id), config)
}

pub fn render_calendar(
    state: CalendarState,
    config: CalendarConfig,
) -> Result<CalendarOutput, CalendarError> {
    let date = state.naive_date()?;
    let (pixels, visible_text) = draw_calendar_dashboard(date, state.page_id);
    let sha256 = format!("{:x}", Sha256::digest(&pixels));
    let publish_decision = if config.previous_frame_hash.as_deref() == Some(&sha256) {
        PublishDecision::Unchanged
    } else {
        PublishDecision::Publish
    };

    Ok(CalendarOutput {
        state,
        frame: MonochromeFrame {
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
            pixels,
            sha256,
        },
        visible_text,
        publish_decision,
    })
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn lunar_leap_month(year: i32) -> u32 {
    let idx = (year - 1900).clamp(0, 200) as usize;
    LUNAR_INFO[idx] & 0xf
}

fn lunar_leap_days(year: i32) -> u32 {
    if lunar_leap_month(year) != 0 {
        let idx = (year - 1900).clamp(0, 200) as usize;
        if (LUNAR_INFO[idx] & 0x10000) != 0 { 30 } else { 29 }
    } else {
        0
    }
}

fn lunar_month_days(year: i32, month: u32) -> u32 {
    let idx = (year - 1900).clamp(0, 200) as usize;
    if (LUNAR_INFO[idx] & (0x10000 >> month)) != 0 { 30 } else { 29 }
}

fn lunar_year_days(year: i32) -> u32 {
    let idx = (year - 1900).clamp(0, 200) as usize;
    let info = LUNAR_INFO[idx];
    let mut total = 348_u32;
    let mut mask = 0x8000_u32;
    while mask > 0x8 {
        if (info & mask) != 0 {
            total += 1;
        }
        mask >>= 1;
    }
    total + lunar_leap_days(year)
}

fn format_lunar_day(lday: u32) -> String {
    const DIGITS: [&str; 11] = ["", "一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    match lday {
        10 => "初十".into(),
        20 => "二十".into(),
        30 => "三十".into(),
        _ => {
            let prefix = ["初", "十", "廿", "三"][(lday / 10).min(3) as usize];
            let suffix = DIGITS[(lday % 10) as usize];
            format!("{prefix}{suffix}")
        }
    }
}

fn format_lunar_month(lmonth: u32, is_leap: bool) -> String {
    let base = LUNAR_MONTH_NAMES[lmonth.saturating_sub(1).min(11) as usize];
    if is_leap {
        format!("闰{base}月")
    } else {
        format!("{base}月")
    }
}

fn moon_phase_name(lday: u32) -> &'static str {
    match lday {
        1 => "朔月",
        2..=6 => "蛾眉月",
        7..=9 => "上弦月",
        10..=14 => "盈凸月",
        15..=16 => "望月",
        17..=21 => "亏凸月",
        22..=24 => "下弦月",
        25..=29 => "残月",
        _ => "晦月",
    }
}

pub fn solar_to_lunar(date: NaiveDate) -> LunarDateInfo {
    let base = NaiveDate::from_ymd_opt(1900, 1, 31).unwrap();
    let total_offset = date.signed_duration_since(base).num_days().max(0);
    let mut offset = total_offset as u32;
    let mut lunar_year = 1900_i32;

    while lunar_year <= 2100 {
        let days_in_ly = lunar_year_days(lunar_year);
        if offset < days_in_ly {
            break;
        }
        offset -= days_in_ly;
        lunar_year += 1;
    }

    let leap_m = lunar_leap_month(lunar_year);
    let mut is_leap = false;
    let mut lunar_month = 1_u32;

    while lunar_month <= 12 {
        let mdays = if leap_m > 0 && lunar_month == (leap_m + 1) && !is_leap {
            lunar_month -= 1;
            is_leap = true;
            lunar_leap_days(lunar_year)
        } else {
            lunar_month_days(lunar_year, lunar_month)
        };
        if offset < mdays {
            break;
        }
        offset -= mdays;
        if is_leap && lunar_month == leap_m {
            is_leap = false;
        }
        lunar_month += 1;
    }

    let lunar_day = offset + 1;
    let cur_mdays = if is_leap {
        lunar_leap_days(lunar_year)
    } else {
        lunar_month_days(lunar_year, lunar_month)
    };

    let yr_idx = (lunar_year - 1900 + 36).rem_euclid(60) as usize;
    let ganzhi_year = format!("{}{}", TIAN_GAN[yr_idx % 10], DI_ZHI[yr_idx % 12]);
    let zodiac = ZODIAC[(lunar_year - 1900).rem_euclid(12) as usize];

    let day_idx = (total_offset + 40).rem_euclid(60) as usize;
    let ganzhi_day = format!("{}{}日", TIAN_GAN[day_idx % 10], DI_ZHI[day_idx % 12]);

    LunarDateInfo {
        lunar_year,
        lunar_month,
        lunar_day,
        is_leap_month: is_leap,
        is_big_month: cur_mdays == 30,
        ganzhi_year,
        zodiac,
        ganzhi_day,
        month_text: format_lunar_month(lunar_month, is_leap),
        day_text: format_lunar_day(lunar_day),
        moon_phase: moon_phase_name(lunar_day),
    }
}

pub fn month_solar_term_days(year: i32, month: u32) -> ((u32, &'static str), (u32, &'static str)) {
    const TERMS: [(&str, f64, &str, f64); 12] = [
        ("小寒", 5.4055, "大寒", 20.12),
        ("立春", 3.87, "雨水", 18.73),
        ("惊蛰", 5.63, "春分", 20.646),
        ("清明", 4.81, "谷雨", 20.1),
        ("立夏", 5.52, "小满", 21.04),
        ("芒种", 5.678, "夏至", 21.37),
        ("小暑", 7.108, "大暑", 22.83),
        ("立秋", 7.5, "处暑", 23.13),
        ("白露", 7.646, "秋分", 23.042),
        ("寒露", 8.318, "霜降", 23.438),
        ("立冬", 7.438, "小雪", 22.36),
        ("大雪", 7.18, "冬至", 21.94),
    ];
    let idx = month.saturating_sub(1).min(11) as usize;
    let (t1_name, c1, t2_name, c2) = TERMS[idx];
    let y = (year % 100).max(0);
    let leap = if month <= 2 { (y - 1) / 4 } else { y / 4 };
    let d1 = (((y as f64) * 0.2422 + c1) as i32 - leap).max(1) as u32;
    let d2 = (((y as f64) * 0.2422 + c2) as i32 - leap).max(1) as u32;
    ((d1, t1_name), (d2, t2_name))
}

pub fn cell_lunar_subtitle(date: NaiveDate) -> (String, bool) {
    let info = solar_to_lunar(date);
    let ((d1, t1), (d2, t2)) = month_solar_term_days(date.year(), date.month());

    if !info.is_leap_month {
        let lunar_fest = match (info.lunar_month, info.lunar_day) {
            (1, 1) => Some("春节"),
            (1, 15) => Some("元宵"),
            (2, 2) => Some("龙头"),
            (5, 5) => Some("端午"),
            (7, 7) => Some("七夕"),
            (7, 15) => Some("中元"),
            (8, 15) => Some("中秋"),
            (9, 9) => Some("重阳"),
            (12, 8) => Some("腊八"),
            (12, 23) => Some("小年"),
            (12, d)
                if d == lunar_month_days(info.lunar_year, 12)
                    && lunar_leap_month(info.lunar_year) != 12 =>
            {
                Some("除夕")
            }
            _ => None,
        };
        if let Some(fest) = lunar_fest {
            return (fest.into(), true);
        }
    }

    if date.day() == d1 {
        return (t1.into(), true);
    }
    if date.day() == d2 {
        return (t2.into(), true);
    }

    let solar_fest = match (date.month(), date.day()) {
        (1, 1) => Some("元旦"),
        (5, 1) => Some("五一"),
        (10, 1) => Some("国庆"),
        _ => None,
    };
    if let Some(fest) = solar_fest {
        return (fest.into(), true);
    }

    if info.lunar_day == 1 {
        let short_m: String = info.month_text.chars().take(2).collect();
        return (short_m, true);
    }

    (info.day_text, false)
}

fn collect_month_highlights(year: i32, month: u32, total_days: u32) -> Vec<(u32, String)> {
    let mut items = Vec::new();
    for d in 1..=total_days {
        if let Some(cur) = NaiveDate::from_ymd_opt(year, month, d) {
            let (sub, is_special) = cell_lunar_subtitle(cur);
            if is_special && !sub.ends_with('月') {
                items.push((d, sub));
            }
        }
    }
    if items.len() < 4 {
        for d in 1..=total_days {
            if let Some(cur) = NaiveDate::from_ymd_opt(year, month, d) {
                let (sub, is_special) = cell_lunar_subtitle(cur);
                if is_special
                    && sub.ends_with('月')
                    && !items.iter().any(|(ed, _)| *ed == d)
                {
                    items.push((d, sub));
                    if items.len() >= 4 {
                        break;
                    }
                }
            }
        }
    }
    items.sort_by_key(|(d, _)| *d);
    items.truncate(4);
    items
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TextAlign {
    Left,
    Center,
    Right,
}

fn draw_calendar_dashboard(target_date: NaiveDate, page_id: u8) -> (Vec<u8>, Vec<String>) {
    let mut disp = FrameDisplay::new();
    let mut visible = Vec::new();

    let year = target_date.year();
    let month = target_date.month();
    let day = target_date.day();
    let weekday_idx = target_date.weekday().num_days_from_monday() as usize;
    let total_month_days = days_in_month(year, month);
    let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let first_weekday = first_day.weekday().num_days_from_monday() as i32;
    let day_of_year = target_date.ordinal();
    let total_year_days = if is_leap_year(year) { 366 } else { 365 };
    let iso_week = target_date.iso_week().week();
    let quarter = (month - 1) / 3 + 1;
    let lunar = solar_to_lunar(target_date);

    const WEEKDAY_CN: [&str; 7] = [
        "星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日",
    ];

    // 1. 极窄边距顶栏 (X: 3..396, Y: 3..27, W: 393, H: 24)
    disp.fill_rect(3, 3, 393, 24, true);
    for (cx, cy) in [(3, 3), (395, 3), (3, 26), (395, 26)] {
        disp.draw_pixel(cx, cy, false);
    }

    disp.fill_rect(7, 6, 62, 18, false);
    draw_micro_pixel_str(&mut disp, "ZECTRIX", 11, 12, 1, 1, true, 2);

    draw_cjk_12x12(&mut disp, "月度日历", 78, 9, false, TextAlign::Left, 3);
    disp.fill_rect(138, 14, 2, 2, false);
    draw_cjk_12x12(&mut disp, "农历黄历", 145, 9, false, TextAlign::Left, 3);
    visible.push("月度日历 · 农历黄历".into());

    disp.stroke_rect(294, 6, 98, 18, 1, false);
    disp.fill_rect(300, 12, 6, 6, false);
    let page_badge = format!("P{page_id} UTC+8");
    draw_micro_pixel_str(&mut disp, page_badge.as_str(), 312, 12, 1, 1, false, 2);
    visible.push(page_badge);

    // 2. 左侧今日公历 + 农历黄历详情卡 (X: 3, Y: 31, W: 130, H: 265)
    let (lx, ly_top, lw, lh) = (3_i32, 31_i32, 130_u32, 265_u32);
    let lw_i32 = lw as i32;
    disp.draw_chamfer_card(lx, ly_top, lw, lh, 1);

    // Zone 1: 年月 + 季度
    disp.fill_rect(lx + 2, ly_top + 2, lw - 4, 23, true);
    let yr_str = format!("{year:04}");
    let w_yr = draw_block_number_str(&mut disp, yr_str.as_str(), lx + 6, ly_top + 8, 6, 11, 2, 1, false, false);
    draw_cjk_12x12(&mut disp, "年", lx + 8 + w_yr, ly_top + 8, false, TextAlign::Left, 2);
    let mo_str = format!("{month:02}");
    let w_mo = draw_block_number_str(&mut disp, mo_str.as_str(), lx + 23 + w_yr, ly_top + 8, 6, 11, 2, 1, false, false);
    draw_cjk_12x12(&mut disp, "月", lx + 25 + w_yr + w_mo, ly_top + 8, false, TextAlign::Left, 2);
    disp.stroke_rect(lx + lw_i32 - 26, ly_top + 6, 20, 15, 1, false);
    let q_str = format!("Q{quarter}");
    draw_micro_pixel_str(&mut disp, q_str.as_str(), lx + lw_i32 - 21, ly_top + 10, 1, 1, false, 1);
    visible.push(format!("{year}年{month:02}月"));

    // Zone 2: 干支生肖 + 农历大小月
    disp.fill_rect(lx + 2, ly_top + 25, lw - 4, 19, false);
    disp.fill_rect(lx + 2, ly_top + 44, lw - 4, 1, true);
    let gz_zodiac = format!("{}{}年", lunar.ganzhi_year, lunar.zodiac);
    let lm_size = format!("{}{}", lunar.month_text, if lunar.is_big_month { "大" } else { "小" });
    draw_cjk_12x12(&mut disp, gz_zodiac.as_str(), lx + 6, ly_top + 29, true, TextAlign::Left, 2);
    draw_cjk_12x12(&mut disp, lm_size.as_str(), lx + lw_i32 - 6, ly_top + 29, true, TextAlign::Right, 2);
    visible.push(format!("{gz_zodiac} {lm_size}"));

    // Zone 3: 巨型 TODAY 公历号数框
    disp.stroke_rect(lx + 7, ly_top + 50, lw - 14, 65, 1, true);
    for (cx, cy) in [
        (lx + 9, ly_top + 52),
        (lx + lw_i32 - 12, ly_top + 52),
        (lx + 9, ly_top + 111),
        (lx + lw_i32 - 12, ly_top + 111),
    ] {
        disp.fill_rect(cx, cy, 3, 3, true);
    }
    disp.fill_rect(lx + 33, ly_top + 46, 64, 10, true);
    draw_micro_pixel_str(&mut disp, "TODAY", lx + 48, ly_top + 48, 1, 1, false, 2);

    let day_str = format!("{day:02}");
    visible.push(format!("今日 {day_str}日"));
    let (dw_hero, dh_hero, dt_hero, gap_hero) = (28_u32, 44_u32, 6_u32, 8_i32);
    let total_dw = (day_str.len() as i32) * (dw_hero as i32) + ((day_str.len() as i32) - 1) * gap_hero;
    let mut start_dx = lx + (lw_i32 - total_dw) / 2;
    for ch in day_str.chars() {
        draw_block_digit(&mut disp, ch, start_dx, ly_top + 62, dw_hero, dh_hero, dt_hero, true, true);
        start_dx += (dw_hero as i32) + gap_hero;
    }

    // Zone 4: 黑底农历日期与月相横幅
    disp.fill_rect(lx + 7, ly_top + 119, lw - 14, 20, true);
    let lunar_md = format!("{}{}", lunar.month_text, lunar.day_text);
    draw_cjk_12x12(&mut disp, lunar_md.as_str(), lx + 12, ly_top + 123, false, TextAlign::Left, 2);
    draw_cjk_12x12(&mut disp, lunar.moon_phase, lx + lw_i32 - 12, ly_top + 123, false, TextAlign::Right, 2);
    visible.push(format!("{lunar_md} {}", lunar.moon_phase));

    // Zone 5: 星期 | 干支日柱
    disp.stroke_rect(lx + 7, ly_top + 143, lw - 14, 20, 1, true);
    disp.draw_dotted_vline(lx + 65, ly_top + 145, ly_top + 160, 2);
    draw_cjk_12x12(&mut disp, WEEKDAY_CN[weekday_idx], lx + 36, ly_top + 147, true, TextAlign::Center, 2);
    draw_cjk_12x12(&mut disp, lunar.ganzhi_day.as_str(), lx + 94, ly_top + 147, true, TextAlign::Center, 2);
    visible.push(WEEKDAY_CN[weekday_idx].into());

    // Zone 6: 第XX周 | 余XX天
    disp.stroke_rect(lx + 7, ly_top + 166, lw - 14, 20, 1, true);
    disp.draw_dotted_vline(lx + 65, ly_top + 168, ly_top + 183, 2);
    draw_cjk_12x12(&mut disp, "第", lx + 11, ly_top + 170, true, TextAlign::Left, 2);
    let wk_str = format!("{iso_week:02}");
    let w_wk = draw_block_number_str(&mut disp, wk_str.as_str(), lx + 24, ly_top + 171, 6, 10, 2, 1, false, true);
    draw_cjk_12x12(&mut disp, "周", lx + 26 + w_wk, ly_top + 170, true, TextAlign::Left, 2);

    let rem_days = total_month_days.saturating_sub(day);
    draw_cjk_12x12(&mut disp, "余", lx + 70, ly_top + 170, true, TextAlign::Left, 2);
    let rm_str = format!("{rem_days:02}");
    let w_rm = draw_block_number_str(&mut disp, rm_str.as_str(), lx + 83, ly_top + 171, 6, 10, 2, 1, false, true);
    draw_cjk_12x12(&mut disp, "天", lx + 85 + w_rm, ly_top + 170, true, TextAlign::Left, 2);

    disp.draw_dotted_hline(lx + 7, lx + lw_i32 - 8, ly_top + 191, 2);

    // Zone 7: 本月流转 & 全年进度
    let month_pct = ((day * 100) / total_month_days.max(1)) as u8;
    draw_cjk_12x12(&mut disp, "本月流转", lx + 7, ly_top + 196, true, TextAlign::Left, 2);
    let mp_str = format!("{day}/{total_month_days}");
    let w_mp = block_number_str_width(mp_str.as_str(), 5, 10, 1, 2);
    draw_block_number_str(&mut disp, mp_str.as_str(), lx + lw_i32 - 7 - w_mp, ly_top + 197, 5, 10, 1, 2, false, true);
    draw_segmented_bar(&mut disp, month_pct, lx + 7, ly_top + 211, lw - 14, 10, 10);

    let year_pct = ((day_of_year * 100) / total_year_days.max(1)) as u8;
    draw_cjk_12x12(&mut disp, "全年进度", lx + 7, ly_top + 227, true, TextAlign::Left, 2);
    let yp_str = format!("{year_pct}%");
    let w_yp = block_number_str_width(yp_str.as_str(), 5, 10, 1, 2);
    draw_block_number_str(&mut disp, yp_str.as_str(), lx + lw_i32 - 7 - w_yp, ly_top + 228, 5, 10, 1, 2, false, true);
    draw_segmented_bar(&mut disp, year_pct, lx + 7, ly_top + 242, lw - 14, 10, 10);

    // 3. 右侧当月完整 7x6 公历+农历双层矩阵卡 (X: 137, Y: 31, W: 259, H: 265)
    let (rx, ry, rw, rh) = (137_i32, 31_i32, 259_u32, 265_u32);
    disp.draw_chamfer_card(rx, ry, rw, rh, 1);

    let grid_x = rx + 4;
    let col_w = 36_i32;
    let grid_w = (col_w * 7) as u32; // 252px

    let hdr_y = ry + 4;
    let hdr_h = 20_u32;
    disp.stroke_rect(grid_x, hdr_y, grid_w, hdr_h, 1, true);
    const WK_HEADERS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
    for (col, label) in WK_HEADERS.iter().enumerate() {
        let cx_cell = grid_x + (col as i32) * col_w;
        if col >= 5 {
            disp.fill_rect(cx_cell, hdr_y, col_w as u32, hdr_h, true);
            draw_cjk_12x12(&mut disp, label, cx_cell + col_w / 2, hdr_y + 4, false, TextAlign::Center, 2);
        } else {
            if col > 0 {
                disp.draw_dotted_vline(cx_cell, hdr_y + 2, hdr_y + (hdr_h as i32) - 3, 2);
            }
            draw_cjk_12x12(&mut disp, label, cx_cell + col_w / 2, hdr_y + 4, true, TextAlign::Center, 2);
        }
    }

    let cells_y = ry + 26;
    let row_h = 35_i32;
    let cells_h = (row_h * 6) as u32; // 210px
    disp.stroke_rect(grid_x, cells_y, grid_w, cells_h, 1, true);

    for col in 1..7_i32 {
        let gx = grid_x + col * col_w;
        disp.draw_dotted_vline(gx, cells_y + 1, cells_y + (cells_h as i32) - 2, 2);
    }
    for row in 1..6_i32 {
        let gy = cells_y + row * row_h;
        disp.draw_dotted_hline(grid_x + 1, grid_x + (grid_w as i32) - 2, gy, 2);
    }

    for slot in 0..42_i32 {
        let row = slot / 7;
        let col = slot % 7;
        let cx = grid_x + col * col_w;
        let cy = cells_y + row * row_h;
        let d_i32 = slot - first_weekday + 1;
        if d_i32 < 1 || d_i32 > (total_month_days as i32) {
            disp.fill_rect(cx + col_w / 2 - 1, cy + row_h / 2, 3, 1, true);
            continue;
        }
        let d = d_i32 as u32;
        let cur_date = NaiveDate::from_ymd_opt(year, month, d).unwrap();
        let (lunar_sub, is_special) = cell_lunar_subtitle(cur_date);

        let d_str = d.to_string();
        let (dw_cell, dh_cell, dt_cell, gap_cell) = (7_u32, 11_u32, 2_u32, 2_i32);
        let num_w = (d_str.len() as i32) * (dw_cell as i32) + ((d_str.len() as i32) - 1) * gap_cell;
        let nx = cx + (col_w - num_w) / 2;
        let ny = cy + 4;

        if d == day {
            disp.fill_rect(cx + 1, cy + 1, (col_w - 1) as u32, (row_h - 1) as u32, true);
            for (px_c, py_c) in [
                (cx + 2, cy + 2),
                (cx + col_w - 3, cy + 2),
                (cx + 2, cy + row_h - 3),
                (cx + col_w - 3, cy + row_h - 3),
            ] {
                disp.draw_pixel(px_c, py_c, false);
            }
            draw_block_number_str(&mut disp, d_str.as_str(), nx, ny, dw_cell, dh_cell, dt_cell, gap_cell, false, false);
            draw_cjk_12x12(&mut disp, lunar_sub.as_str(), cx + col_w / 2, cy + 18, false, TextAlign::Center, 2);
        } else {
            if d < day {
                for dy_t in 0..4_i32 {
                    disp.fill_rect(cx + 2, cy + 2 + dy_t, (4 - dy_t) as u32, 1, true);
                }
            }
            if is_special {
                disp.fill_rect(cx + col_w - 5, cy + 2, 3, 3, true);
                disp.fill_rect(cx + 5, cy + row_h - 2, (col_w - 9) as u32, 1, true);
            }
            draw_block_number_str(&mut disp, d_str.as_str(), nx, ny, dw_cell, dh_cell, dt_cell, gap_cell, false, true);
            draw_cjk_12x12(&mut disp, lunar_sub.as_str(), cx + col_w / 2, cy + 18, true, TextAlign::Center, 2);
        }
    }

    // 4. 底部 [节候] 当月节气与节庆速览栏 (Y: ry+239 .. ry+259, H: 20px)
    let leg_y = ry + 239;
    let leg_h = 20_u32;
    disp.stroke_rect(grid_x, leg_y, grid_w, leg_h, 1, true);
    disp.fill_rect(grid_x, leg_y, 36, leg_h, true);
    draw_cjk_12x12(&mut disp, "节候", grid_x + 18, leg_y + 4, false, TextAlign::Center, 2);

    let highlights = collect_month_highlights(year, month, total_month_days);
    let mut hx = grid_x + 43;
    for (idx_h, (hd, hname)) in highlights.iter().enumerate() {
        if idx_h > 0 {
            disp.fill_rect(hx, leg_y + 9, 2, 2, true);
            hx += 6;
        }
        let hd_str = format!("{hd:02}");
        let w_hd = draw_block_number_str(&mut disp, hd_str.as_str(), hx, leg_y + 5, 5, 10, 1, 1, false, true);
        hx += w_hd + 3;
        let w_hn = draw_cjk_12x12(&mut disp, hname.as_str(), hx, leg_y + 4, true, TextAlign::Left, 2);
        hx += w_hn + 6;
    }

    (disp.pixels, visible)
}

/// 逐字手工调优的 12x12 像素点阵字库（覆盖全部 24 节气、农历月日、干支、生肖、月相及界面文案）
fn cjk_glyph_12x12(ch: char) -> Option<[u16; 12]> {
    match ch {
        '一' => Some([0x000, 0x000, 0x000, 0x000, 0x000, 0xffe, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000]),
        '丁' => Some([0xffe, 0x040, 0x040, 0x040, 0x040, 0x040, 0x040, 0x040, 0x040, 0x040, 0x0c0, 0x000]),
        '七' => Some([0x080, 0x080, 0x080, 0x09e, 0x7e0, 0x080, 0x080, 0x080, 0x082, 0x082, 0x07e, 0x000]),
        '三' => Some([0x000, 0x7fc, 0x000, 0x000, 0x000, 0x3f8, 0x000, 0x000, 0x000, 0xffe, 0x000, 0x000]),
        '上' => Some([0x080, 0x080, 0x080, 0x080, 0x0fc, 0x080, 0x080, 0x080, 0x080, 0x080, 0xffe, 0x000]),
        '下' => Some([0xffe, 0x080, 0x080, 0x080, 0x090, 0x088, 0x084, 0x080, 0x080, 0x080, 0x080, 0x000]),
        '丑' => Some([0x7fc, 0x084, 0x084, 0x084, 0x104, 0x7fc, 0x104, 0x104, 0x104, 0x104, 0xffe, 0x000]),
        '丙' => Some([0xffe, 0x040, 0x040, 0x7fc, 0x444, 0x4a4, 0x4a4, 0x514, 0x60c, 0x404, 0x40c, 0x000]),
        '中' => Some([0x040, 0x040, 0x7fc, 0x444, 0x444, 0x444, 0x7fc, 0x040, 0x040, 0x040, 0x040, 0x000]),
        '乙' => Some([0x7fc, 0x008, 0x010, 0x020, 0x040, 0x080, 0x100, 0x200, 0x202, 0x202, 0x1fe, 0x000]),
        '九' => Some([0x100, 0x100, 0x100, 0x7f8, 0x108, 0x108, 0x108, 0x208, 0x20a, 0x40a, 0x806, 0x000]),
        '二' => Some([0x000, 0x000, 0x3fc, 0x000, 0x000, 0x000, 0x000, 0x000, 0xffe, 0x000, 0x000, 0x000]),
        '亏' => Some([0x7fc, 0x000, 0x000, 0xffe, 0x100, 0x100, 0x1f8, 0x008, 0x008, 0x008, 0x030, 0x000]),
        '五' => Some([0x7fc, 0x080, 0x080, 0x080, 0x3f8, 0x088, 0x108, 0x108, 0x108, 0x108, 0xffe, 0x000]),
        '亥' => Some([0x040, 0xffe, 0x080, 0x108, 0x3f0, 0x040, 0x090, 0x120, 0x240, 0x490, 0x30c, 0x000]),
        '今' => Some([0x040, 0x0a0, 0x110, 0x248, 0x424, 0x802, 0x3fc, 0x008, 0x010, 0x020, 0x040, 0x000]),
        '余' => Some([0x040, 0x0a0, 0x110, 0x208, 0x5f4, 0x842, 0x040, 0x3f8, 0x148, 0x244, 0x4c2, 0x000]),
        '候' => Some([0x47c, 0x404, 0xdbe, 0xd10, 0x528, 0x57c, 0x510, 0x57e, 0x510, 0x428, 0x446, 0x000]),
        '元' => Some([0x3fc, 0x000, 0x000, 0xffe, 0x0a0, 0x0a0, 0x120, 0x120, 0x222, 0x222, 0x41e, 0x000]),
        '兔' => Some([0x100, 0x3f0, 0x410, 0x7fc, 0x444, 0x444, 0x7fc, 0x0a0, 0x128, 0x225, 0x41f, 0x000]),
        '全' => Some([0x040, 0x0a0, 0x110, 0x208, 0x5f4, 0x842, 0x040, 0x3f8, 0x040, 0x040, 0x7fc, 0x000]),
        '八' => Some([0x000, 0x120, 0x120, 0x110, 0x110, 0x210, 0x208, 0x208, 0x404, 0x404, 0x802, 0x000]),
        '六' => Some([0x080, 0x040, 0x000, 0xffe, 0x000, 0x110, 0x108, 0x208, 0x204, 0x404, 0x802, 0x000]),
        '农' => Some([0x080, 0x080, 0x7fe, 0x482, 0x108, 0x350, 0x560, 0x950, 0x108, 0x146, 0x180, 0x000]),
        '冬' => Some([0x080, 0x1f0, 0x210, 0x520, 0x0c0, 0x1b0, 0x60e, 0x0c0, 0x020, 0x0c0, 0x030, 0x000]),
        '凸' => Some([0x1f0, 0x110, 0x110, 0x110, 0xf1e, 0x802, 0x802, 0x802, 0x802, 0xffe, 0x000, 0x000]),
        '分' => Some([0x110, 0x108, 0x208, 0x404, 0xbfa, 0x088, 0x088, 0x108, 0x108, 0x208, 0x430, 0x000]),
        '初' => Some([0x200, 0x13e, 0x78a, 0x10a, 0x20a, 0x70a, 0xa92, 0x212, 0x222, 0x242, 0x286, 0x000]),
        '十' => Some([0x040, 0x040, 0x040, 0x040, 0x040, 0xffe, 0x040, 0x040, 0x040, 0x040, 0x040, 0x000]),
        '午' => Some([0x100, 0x100, 0x3fc, 0x240, 0x440, 0x040, 0xffe, 0x040, 0x040, 0x040, 0x040, 0x000]),
        '卯' => Some([0x19e, 0x612, 0x492, 0x492, 0x492, 0x492, 0x592, 0x696, 0x110, 0x210, 0x410, 0x000]),
        '历' => Some([0x7fe, 0x400, 0x440, 0x440, 0x5fc, 0x444, 0x444, 0x484, 0x484, 0x514, 0xa0c, 0x000]),
        '周' => Some([0x7fc, 0x444, 0x5f4, 0x444, 0x5f4, 0x404, 0x5f4, 0x514, 0x5f4, 0x404, 0x80c, 0x000]),
        '四' => Some([0x7fe, 0x492, 0x492, 0x492, 0x492, 0x512, 0x61e, 0x402, 0x402, 0x7fe, 0x402, 0x000]),
        '国' => Some([0x7fe, 0x402, 0x5fa, 0x442, 0x442, 0x5fa, 0x452, 0x44a, 0x5fa, 0x402, 0x7fe, 0x000]),
        '壬' => Some([0x1fc, 0x640, 0x040, 0x040, 0x040, 0xffe, 0x040, 0x040, 0x040, 0x040, 0x7fe, 0x000]),
        '处' => Some([0x110, 0x110, 0x3d0, 0x25e, 0x652, 0x552, 0x992, 0x196, 0x250, 0x430, 0x80f, 0x000]),
        '夏' => Some([0xffe, 0x040, 0x3fc, 0x204, 0x3fc, 0x204, 0x3fc, 0x080, 0x1f0, 0x250, 0x68e, 0x000]),
        '夕' => Some([0x100, 0x1f8, 0x208, 0x408, 0x910, 0x090, 0x060, 0x020, 0x040, 0x180, 0x600, 0x000]),
        '大' => Some([0x040, 0x040, 0x040, 0xffe, 0x040, 0x0a0, 0x0a0, 0x110, 0x110, 0x208, 0xc06, 0x000]),
        '天' => Some([0x7fc, 0x040, 0x040, 0x040, 0xffe, 0x040, 0x0a0, 0x0a0, 0x110, 0x208, 0xc06, 0x000]),
        '头' => Some([0x200, 0x100, 0x480, 0x240, 0x040, 0xffe, 0x040, 0x040, 0x0a0, 0x110, 0x60c, 0x000]),
        '子' => Some([0x7fc, 0x008, 0x010, 0x020, 0x040, 0xffe, 0x040, 0x040, 0x040, 0x040, 0x0c0, 0x000]),
        '宵' => Some([0x040, 0x7fe, 0x442, 0x248, 0x444, 0x3fc, 0x204, 0x3fc, 0x204, 0x3fc, 0x204, 0x20c]),
        '寅' => Some([0x040, 0x7fe, 0x402, 0x3fc, 0x040, 0x3fc, 0x244, 0x3fc, 0x244, 0x3fc, 0x110, 0x60c]),
        '寒' => Some([0x040, 0x7fe, 0x492, 0x3fc, 0x090, 0x3fc, 0x090, 0x7fe, 0x108, 0x666, 0x060, 0x198]),
        '小' => Some([0x040, 0x040, 0x040, 0x148, 0x148, 0x244, 0x244, 0x442, 0x040, 0x040, 0x0c0, 0x000]),
        '己' => Some([0x7fc, 0x004, 0x004, 0x004, 0x7fc, 0x400, 0x400, 0x400, 0x402, 0x402, 0x3fe, 0x000]),
        '巳' => Some([0x7fc, 0x404, 0x404, 0x404, 0x7fc, 0x400, 0x400, 0x400, 0x402, 0x402, 0x3fe, 0x000]),
        '年' => Some([0x100, 0x1fc, 0x240, 0x440, 0x3fc, 0x240, 0x240, 0xffe, 0x040, 0x040, 0x040, 0x000]),
        '庆' => Some([0x040, 0x7fe, 0x400, 0x420, 0x420, 0x5fe, 0x420, 0x450, 0x450, 0x488, 0x906, 0x000]),
        '庚' => Some([0x040, 0x7fe, 0x440, 0x5f8, 0x448, 0x7fe, 0x448, 0x5f8, 0x440, 0x4a0, 0x91c, 0x000]),
        '度' => Some([0x040, 0x7fe, 0x490, 0x7fe, 0x490, 0x4f0, 0x400, 0x5f8, 0x490, 0x460, 0x99e, 0x000]),
        '廿' => Some([0x110, 0x110, 0x110, 0xffe, 0x110, 0x110, 0x110, 0x110, 0x110, 0x1f0, 0x000, 0x000]),
        '弦' => Some([0xf08, 0x108, 0x17e, 0xf10, 0x824, 0x878, 0xf08, 0x112, 0x122, 0x17f, 0x601, 0x000]),
        '惊' => Some([0x210, 0x27e, 0xa00, 0xb78, 0x248, 0x278, 0x210, 0x294, 0x252, 0x210, 0x230, 0x000]),
        '戊' => Some([0x014, 0x012, 0x7fe, 0x410, 0x410, 0x414, 0x414, 0x438, 0x449, 0x485, 0x903, 0x000]),
        '戌' => Some([0x014, 0x012, 0x7fe, 0x410, 0x410, 0x5d4, 0x414, 0x438, 0x449, 0x485, 0x903, 0x000]),
        '日' => Some([0x3fc, 0x204, 0x204, 0x204, 0x204, 0x3fc, 0x204, 0x204, 0x204, 0x204, 0x3fc, 0x000]),
        '旦' => Some([0x3fc, 0x204, 0x204, 0x3fc, 0x204, 0x204, 0x3fc, 0x000, 0x000, 0xffe, 0x000, 0x000]),
        '明' => Some([0x7be, 0x4a2, 0x4a2, 0x7be, 0x4a2, 0x4a2, 0x7be, 0x4a2, 0x022, 0x042, 0x086, 0x000]),
        '星' => Some([0x3f8, 0x208, 0x3f8, 0x208, 0x3f8, 0x120, 0x2fc, 0x420, 0x1f8, 0x020, 0x7fe, 0x000]),
        '春' => Some([0x040, 0x7fc, 0x080, 0x7fc, 0x120, 0xe1e, 0x3fc, 0x204, 0x3fc, 0x204, 0x3fc, 0x000]),
        '晦' => Some([0x010, 0xe3e, 0xa40, 0xa3e, 0xe2a, 0xa7f, 0xa2a, 0xa22, 0xe7f, 0x004, 0x01c, 0x000]),
        '暑' => Some([0x3fc, 0x204, 0x3fc, 0x204, 0x3fc, 0x048, 0x7fe, 0x1fc, 0x600, 0x1fc, 0x104, 0x1fc]),
        '月' => Some([0x3fc, 0x204, 0x204, 0x3fc, 0x204, 0x204, 0x3fc, 0x204, 0x204, 0x404, 0x80c, 0x000]),
        '朔' => Some([0x49e, 0x312, 0xfd2, 0x49e, 0x492, 0x792, 0x11e, 0x7d2, 0x112, 0x212, 0x446, 0x000]),
        '望' => Some([0x21e, 0xf92, 0x41e, 0x492, 0x79e, 0x022, 0x7fc, 0x040, 0x3f8, 0x040, 0xffe, 0x000]),
        '期' => Some([0x25e, 0xfd2, 0x252, 0x3de, 0x252, 0x3d2, 0x25e, 0xfd2, 0x252, 0x492, 0x866, 0x000]),
        '未' => Some([0x040, 0x040, 0x3f8, 0x040, 0x040, 0xffe, 0x0e0, 0x150, 0x248, 0x444, 0x842, 0x000]),
        '本' => Some([0x040, 0x040, 0xffe, 0x0e0, 0x150, 0x248, 0x444, 0x842, 0x3f8, 0x040, 0x040, 0x000]),
        '板' => Some([0x20e, 0x230, 0x220, 0xf20, 0x23e, 0x72a, 0x6aa, 0xa24, 0x224, 0x24a, 0x291, 0x000]),
        '正' => Some([0x7fe, 0x040, 0x040, 0x040, 0x240, 0x27c, 0x240, 0x240, 0x240, 0x240, 0xffe, 0x000]),
        '残' => Some([0x00a, 0xf88, 0x23e, 0x208, 0x7be, 0x488, 0xabf, 0x105, 0x10c, 0x212, 0x463, 0x000]),
        '水' => Some([0x040, 0x040, 0x044, 0x748, 0x150, 0x160, 0x250, 0x248, 0x444, 0x842, 0x0c0, 0x000]),
        '流' => Some([0x010, 0x4fe, 0x220, 0x044, 0x4fe, 0x202, 0x054, 0x254, 0x454, 0x495, 0x913, 0x000]),
        '清' => Some([0x010, 0x4fe, 0x210, 0x07c, 0x410, 0x2fe, 0x07c, 0x244, 0x47c, 0x444, 0x84c, 0x000]),
        '满' => Some([0x048, 0x5fe, 0x248, 0x07c, 0x430, 0x2fe, 0x0b2, 0x2ca, 0x4aa, 0x482, 0x886, 0x000]),
        '牛' => Some([0x100, 0x140, 0x240, 0x3fc, 0x440, 0x040, 0xffe, 0x040, 0x040, 0x040, 0x040, 0x000]),
        '狗' => Some([0x520, 0x220, 0x67e, 0x942, 0x33a, 0x52a, 0x92a, 0x13a, 0x102, 0x102, 0x60c, 0x000]),
        '猪' => Some([0x512, 0x27c, 0x614, 0x97e, 0x308, 0x51e, 0x92a, 0x14e, 0x10a, 0x10e, 0x60a, 0x000]),
        '猴' => Some([0x55e, 0x242, 0x6de, 0x948, 0x350, 0x55e, 0x944, 0x15e, 0x144, 0x14a, 0x651, 0x000]),
        '甲' => Some([0x7fc, 0x444, 0x444, 0x7fc, 0x444, 0x444, 0x7fc, 0x040, 0x040, 0x040, 0x040, 0x000]),
        '申' => Some([0x040, 0x040, 0x7fc, 0x444, 0x444, 0x7fc, 0x444, 0x444, 0x7fc, 0x040, 0x040, 0x000]),
        '癸' => Some([0x794, 0x128, 0x530, 0x24c, 0x482, 0xbf8, 0x040, 0x7fe, 0x0a0, 0x110, 0x60e, 0x000]),
        '白' => Some([0x080, 0x100, 0x7fc, 0x404, 0x404, 0x404, 0x7fc, 0x404, 0x404, 0x404, 0x7fc, 0x000]),
        '盈' => Some([0x7f8, 0x208, 0x2e8, 0x4ae, 0x542, 0xaa6, 0x000, 0x7fc, 0x494, 0x494, 0xffe, 0x000]),
        '眉' => Some([0x7fc, 0x444, 0x7fc, 0x400, 0x5fc, 0x504, 0x5fc, 0x504, 0x5fc, 0x504, 0x9fc, 0x000]),
        '看' => Some([0x0fc, 0x740, 0x040, 0xffe, 0x100, 0x3fc, 0x504, 0x9fc, 0x104, 0x1fc, 0x104, 0x1fc]),
        '秋' => Some([0x384, 0xd04, 0x125, 0xf26, 0x104, 0x304, 0x58a, 0x90a, 0x111, 0x121, 0x140, 0x000]),
        '种' => Some([0x310, 0xc10, 0x27e, 0xfaa, 0x2aa, 0x7aa, 0x6be, 0xa10, 0x210, 0x210, 0x210, 0x000]),
        '立' => Some([0x080, 0x040, 0x7fe, 0x000, 0x208, 0x108, 0x110, 0x110, 0x090, 0x0a0, 0xffe, 0x000]),
        '端' => Some([0x208, 0x02a, 0xf2a, 0x53e, 0x500, 0x57e, 0x508, 0x5be, 0xe2a, 0x02a, 0x02b, 0x000]),
        '第' => Some([0x210, 0x7be, 0x948, 0x7fc, 0x044, 0x7fc, 0x440, 0x7fe, 0x0c2, 0x146, 0x640, 0x000]),
        '羊' => Some([0x108, 0x090, 0x7fc, 0x040, 0x040, 0x3f8, 0x040, 0x040, 0xffe, 0x040, 0x040, 0x000]),
        '腊' => Some([0x712, 0x57f, 0x512, 0x712, 0x57f, 0x500, 0x73e, 0x522, 0x53e, 0x522, 0xb3e, 0x000]),
        '至' => Some([0x7fc, 0x080, 0x110, 0x208, 0x7fc, 0x042, 0x040, 0x3f8, 0x040, 0x040, 0xffe, 0x000]),
        '节' => Some([0x110, 0xffe, 0x110, 0x000, 0x7fc, 0x044, 0x044, 0x044, 0x04c, 0x040, 0x040, 0x000]),
        '芒' => Some([0x110, 0x7fe, 0x110, 0x040, 0x040, 0x7fe, 0x200, 0x200, 0x200, 0x200, 0x3fe, 0x000]),
        '虎' => Some([0x040, 0x078, 0x7c0, 0x444, 0x5f8, 0x440, 0x478, 0x490, 0x492, 0x512, 0xa0e, 0x000]),
        '蛇' => Some([0x208, 0x208, 0xfbe, 0xaa2, 0xa84, 0xf94, 0x218, 0x210, 0x291, 0xf91, 0x08f, 0x000]),
        '蛰' => Some([0x228, 0x778, 0x250, 0x76a, 0x292, 0x020, 0x3fc, 0x248, 0x3fc, 0x020, 0x7fa, 0x006]),
        '蛾' => Some([0x216, 0x264, 0xf25, 0xa94, 0xffe, 0x224, 0x235, 0x2e6, 0x724, 0x8aa, 0x073, 0x000]),
        '谷' => Some([0x110, 0x208, 0x444, 0x0a0, 0x110, 0x208, 0x5f4, 0x912, 0x110, 0x110, 0x1f0, 0x000]),
        '转' => Some([0x208, 0xf88, 0x43e, 0x508, 0xfbf, 0x210, 0x29e, 0xf02, 0x214, 0x20c, 0x202, 0x000]),
        '辛' => Some([0x040, 0x7fe, 0x108, 0x090, 0xffe, 0x040, 0x040, 0x7fe, 0x040, 0x040, 0x040, 0x000]),
        '辰' => Some([0x7fe, 0x400, 0x5fc, 0x400, 0x7fe, 0x524, 0x518, 0x510, 0x548, 0x586, 0x900, 0x000]),
        '进' => Some([0x424, 0x224, 0x0fe, 0x024, 0xe24, 0x2ff, 0x224, 0x244, 0x284, 0x504, 0x8ff, 0x000]),
        '酉' => Some([0xffe, 0x090, 0x090, 0x7fc, 0x494, 0x514, 0x41c, 0x404, 0x7fc, 0x404, 0x7fc, 0x000]),
        '重' => Some([0x1fc, 0x040, 0x7fe, 0x3fc, 0x244, 0x3fc, 0x244, 0x3fc, 0x040, 0x3fc, 0x040, 0x7fe]),
        '闰' => Some([0x43e, 0x202, 0x802, 0x9f2, 0x842, 0x842, 0x9f2, 0x842, 0x842, 0xbfa, 0x806, 0x000]),
        '阳' => Some([0xf3e, 0x922, 0xa22, 0xc22, 0xa3e, 0x922, 0x922, 0xa22, 0xc22, 0x822, 0x83e, 0x000]),
        '降' => Some([0xe10, 0xa3e, 0xc44, 0xa28, 0xa10, 0xc6e, 0x890, 0x87c, 0x848, 0x8fe, 0x808, 0x808]),
        '除' => Some([0xe10, 0xa28, 0xc44, 0xaba, 0xa10, 0xc7e, 0x810, 0x854, 0x852, 0x892, 0x830, 0x000]),
        '雨' => Some([0x7fe, 0x040, 0x7fe, 0x442, 0x54a, 0x4ca, 0x442, 0x54a, 0x4ca, 0x442, 0x446, 0x000]),
        '雪' => Some([0x7fe, 0x040, 0x7fe, 0x556, 0x556, 0x000, 0x3fc, 0x004, 0x3fc, 0x004, 0x3fc, 0x000]),
        '霜' => Some([0x7fe, 0x040, 0x7fe, 0x556, 0x000, 0x23e, 0x7a2, 0x23e, 0x722, 0xabe, 0x222, 0x23e]),
        '露' => Some([0x7fe, 0x040, 0x7fe, 0x556, 0x000, 0x710, 0x53e, 0x714, 0x228, 0x75e, 0x452, 0xf9e]),
        '马' => Some([0x7f8, 0x008, 0x208, 0x208, 0x3f8, 0x004, 0x004, 0x7f4, 0x004, 0x004, 0x01c, 0x000]),
        '鸡' => Some([0x03e, 0xf22, 0x12a, 0x522, 0x23c, 0x220, 0x53f, 0x881, 0x07d, 0x001, 0x00e, 0x000]),
        '黄' => Some([0x110, 0x7fc, 0x150, 0xffe, 0x3f8, 0x248, 0x3f8, 0x248, 0x3f8, 0x110, 0x60c, 0x000]),
        '鼠' => Some([0x33c, 0x404, 0x73c, 0x404, 0x7fc, 0x248, 0x290, 0x248, 0x291, 0x321, 0x267, 0x000]),
        '龙' => Some([0x100, 0x108, 0x104, 0xffe, 0x100, 0x13c, 0x220, 0x260, 0x4a2, 0x522, 0x81e, 0x000]),
        _ => None,
    }
}

fn draw_cjk_12x12(
    disp: &mut FrameDisplay,
    text: &str,
    x: i32,
    y: i32,
    on: bool,
    align: TextAlign,
    gap: i32,
) -> i32 {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let total_w = (chars.len() as i32) * 12 + ((chars.len() as i32) - 1) * gap;
    let start_x = match align {
        TextAlign::Left => x,
        TextAlign::Center => x - total_w / 2,
        TextAlign::Right => x - total_w,
    };
    let mut cx = start_x;
    for ch in chars {
        if let Some(rows) = cjk_glyph_12x12(ch) {
            for (ry, bits) in rows.iter().enumerate() {
                for rx in 0..12_i32 {
                    if ((bits >> (11 - rx)) & 1) == 1 {
                        disp.draw_pixel(cx + rx, y + (ry as i32), on);
                    }
                }
            }
        }
        cx += 12 + gap;
    }
    total_w
}

fn draw_segmented_bar(
    disp: &mut FrameDisplay,
    percent: u8,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    segments: u32,
) {
    disp.stroke_rect(x, y, w, h, 1, true);
    let inner_x = x + 2;
    let inner_y = y + 2;
    let inner_w = w.saturating_sub(4);
    let inner_h = h.saturating_sub(4);
    let seg_count = segments.max(1);
    let gap = 1_u32;
    let total_gaps = gap * seg_count.saturating_sub(1);
    let seg_w = inner_w.saturating_sub(total_gaps) / seg_count;
    let rem_w = inner_w.saturating_sub(seg_w * seg_count + total_gaps);

    let safe_pct = u32::from(percent.min(100));
    let total_fill_px = (inner_w * safe_pct) / 100;

    let mut cur_x = inner_x;
    for i in 0..seg_count {
        let sw = seg_w + u32::from(i < rem_w);
        let seg_start = (cur_x - inner_x) as u32;
        let seg_end = seg_start + sw;
        if total_fill_px >= seg_end {
            disp.fill_rect(cur_x, inner_y, sw, inner_h, true);
        } else if total_fill_px > seg_start {
            let fill_part = total_fill_px - seg_start;
            disp.fill_rect(cur_x, inner_y, fill_part, inner_h, true);
            disp.fill_sparse_dots(cur_x + fill_part as i32, inner_y, sw - fill_part, inner_h);
        } else {
            disp.fill_sparse_dots(cur_x, inner_y, sw, inner_h);
        }
        cur_x += (sw + gap) as i32;
    }
}

fn digit_segments(ch: char) -> Option<[bool; 7]> {
    match ch {
        '0' => Some([true, true, true, true, true, true, false]),
        '1' => Some([false, true, true, false, false, false, false]),
        '2' => Some([true, true, false, true, true, false, true]),
        '3' => Some([true, true, true, true, false, false, true]),
        '4' => Some([false, true, true, false, false, true, true]),
        '5' => Some([true, false, true, true, false, true, true]),
        '6' => Some([true, false, true, true, true, true, true]),
        '7' => Some([true, true, true, false, false, false, false]),
        '8' => Some([true, true, true, true, true, true, true]),
        '9' => Some([true, true, true, true, false, true, true]),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_block_digit(
    disp: &mut FrameDisplay,
    ch: char,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    t: u32,
    shadow: bool,
    on: bool,
) {
    let Some([a, b, c, d, e, f, g]) = digit_segments(ch) else {
        return;
    };
    let w_i32 = w as i32;
    let h_i32 = h as i32;
    let t_i32 = t as i32;
    let mid_y = y + (h_i32 - t_i32) / 2;
    let half_h = ((h_i32 - t_i32) / 2) as u32;

    if ch == '1' {
        let bar_x = x + (w_i32 - t_i32) / 2 + i32::from(w >= 12);
        let beak_w = t.saturating_sub(1).max(2);
        let foot_w = (t + 2 * beak_w).min(w);
        let foot_x = (bar_x - beak_w as i32).max(x);
        let rects = [
            (bar_x - beak_w as i32, y + i32::from(h >= 14), beak_w, t),
            (bar_x, y, t, h),
            (foot_x, y + h_i32 - t_i32, foot_w, t),
        ];
        if shadow && on {
            for (rx, ry, rw, rh) in rects {
                disp.fill_halftone(rx + 2, ry + 2, rw, rh, 2);
            }
        }
        for (rx, ry, rw, rh) in rects {
            disp.fill_rect(rx, ry, rw, rh, on);
        }
        return;
    }

    let mut rects = Vec::with_capacity(7);
    if a {
        rects.push((x, y, w, t));
    }
    if d {
        rects.push((x, y + h_i32 - t_i32, w, t));
    }
    if g {
        rects.push((x, mid_y, w, t));
    }
    if f {
        rects.push((x, y, t, half_h + t));
    }
    if b {
        rects.push((x + w_i32 - t_i32, y, t, half_h + t));
    }
    let lower_h = (h_i32 - (mid_y - y)) as u32;
    if e {
        rects.push((x, mid_y, t, lower_h));
    }
    if c {
        rects.push((x + w_i32 - t_i32, mid_y, t, lower_h));
    }

    if shadow && on {
        for &(rx, ry, rw, rh) in &rects {
            disp.fill_halftone(rx + 2, ry + 2, rw, rh, 2);
        }
    }
    for (rx, ry, rw, rh) in rects {
        disp.fill_rect(rx, ry, rw, rh, on);
    }

    if w >= 10 {
        if a && f {
            disp.draw_pixel(x, y, !on);
        }
        if a && b {
            disp.draw_pixel(x + w_i32 - 1, y, !on);
        }
        if d && e {
            disp.draw_pixel(x, y + h_i32 - 1, !on);
        }
        if d && c {
            disp.draw_pixel(x + w_i32 - 1, y + h_i32 - 1, !on);
        }
    }
}

fn draw_block_percent(disp: &mut FrameDisplay, x: i32, y: i32, size: u32, t: u32, on: bool) {
    let dot = (size / 3).max(t + 1);
    let size_i32 = size as i32;
    let dot_i32 = dot as i32;
    disp.fill_rect(x, y, dot, dot, on);
    disp.fill_rect(x + size_i32 - dot_i32, y + size_i32 - dot_i32, dot, dot, on);
    let steps = size.saturating_sub(2) as i32;
    let half_t = (t / 2) as i32;
    for i in 0..steps {
        let px = x + 1 + i;
        let py = y + size_i32 - 2 - i;
        disp.fill_rect(px, py - half_t, t, t, on);
    }
}

fn block_number_str_width(text: &str, dw: u32, dh: u32, dt: u32, gap: i32) -> i32 {
    let mut width = 0_i32;
    for ch in text.chars() {
        let w = if ch.is_ascii_digit() {
            dw as i32
        } else if ch == ':' || ch == '.' {
            dt.max(2) as i32 + 2
        } else if ch == '-' || ch == '/' {
            dw.saturating_sub(2).max(4) as i32
        } else if ch == '%' {
            dh.saturating_sub(2).max(8) as i32
        } else {
            (dw / 2) as i32
        };
        width += w + gap;
    }
    (width - gap).max(0)
}

#[allow(clippy::too_many_arguments)]
fn draw_block_number_str(
    disp: &mut FrameDisplay,
    text: &str,
    x: i32,
    y: i32,
    dw: u32,
    dh: u32,
    dt: u32,
    gap: i32,
    shadow: bool,
    on: bool,
) -> i32 {
    let mut cx = x;
    let dh_i32 = dh as i32;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            draw_block_digit(disp, ch, cx, y, dw, dh, dt, shadow, on);
            cx += (dw as i32) + gap;
        } else if ch == ':' {
            let dot = dt.max(2);
            let dot_i32 = dot as i32;
            disp.fill_rect(cx + 1, y + dh_i32 / 4, dot, dot, on);
            disp.fill_rect(cx + 1, y + (dh_i32 * 3) / 4 - dot_i32, dot, dot, on);
            cx += dot_i32 + 2 + gap;
        } else if ch == '-' {
            let mw = dw.saturating_sub(2).max(4);
            disp.fill_rect(cx, y + (dh_i32 - dt as i32) / 2, mw, dt, on);
            cx += (mw as i32) + gap;
        } else if ch == '/' {
            let mw = dw.saturating_sub(2).max(4) as i32;
            let span = (dh_i32 - 3).max(1);
            for i in 0..(dh_i32 - 2) {
                let px = cx + mw - 1 - (i * (mw - 1)) / span;
                let py = y + 1 + i;
                disp.fill_rect(px, py, dt, dt, on);
            }
            cx += mw + gap;
        } else if ch == '%' {
            let psize = dh.saturating_sub(2).max(8);
            draw_block_percent(disp, cx, y + (dh_i32 - psize as i32), psize, dt.saturating_sub(1).max(2), on);
            cx += (psize as i32) + gap;
        } else {
            cx += (dw as i32) / 2 + gap;
        }
    }
    cx - x
}

fn micro_pixel_glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b10010, 0b10010, 0b10010, 0b11111, 0b00010, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'C' => [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        _ => [0; 7],
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_micro_pixel_str(
    disp: &mut FrameDisplay,
    text: &str,
    x: i32,
    y: i32,
    scale_x: u32,
    scale_y: u32,
    on: bool,
    gap: i32,
) -> i32 {
    let mut cx = x;
    for ch in text.chars() {
        let rows = micro_pixel_glyph(ch);
        for (ry, row) in rows.iter().enumerate() {
            for rx in 0..5_u32 {
                if ((row >> (4 - rx)) & 1) == 1 {
                    disp.fill_rect(
                        cx + (rx * scale_x) as i32,
                        y + (ry as u32 * scale_y) as i32,
                        scale_x,
                        scale_y,
                        on,
                    );
                }
            }
        }
        cx += (5 * scale_x) as i32 + gap;
    }
    cx - x
}

struct FrameDisplay {
    pixels: Vec<u8>,
}

impl FrameDisplay {
    fn new() -> Self {
        Self {
            pixels: vec![255; (DISPLAY_WIDTH * DISPLAY_HEIGHT) as usize],
        }
    }

    #[inline]
    fn draw_pixel(&mut self, x: i32, y: i32, on: bool) {
        if x >= 0 && y >= 0 && (x as u32) < DISPLAY_WIDTH && (y as u32) < DISPLAY_HEIGHT {
            let index = (y as u32 * DISPLAY_WIDTH + x as u32) as usize;
            self.pixels[index] = if on { 0 } else { 255 };
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, on: bool) {
        for py in y..y.saturating_add(h as i32) {
            for px in x..x.saturating_add(w as i32) {
                self.draw_pixel(px, py, on);
            }
        }
    }

    fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, thickness: u32, on: bool) {
        let t = thickness.min(w).min(h);
        let t_i32 = t as i32;
        let w_i32 = w as i32;
        let h_i32 = h as i32;
        self.fill_rect(x, y, w, t, on);
        self.fill_rect(x, y + h_i32 - t_i32, w, t, on);
        self.fill_rect(x, y, t, h, on);
        self.fill_rect(x + w_i32 - t_i32, y, t, h, on);
    }

    fn fill_halftone(&mut self, x: i32, y: i32, w: u32, h: u32, step: i32) {
        let step = step.max(1);
        for py in y..y.saturating_add(h as i32) {
            for px in x..x.saturating_add(w as i32) {
                if (px + py).rem_euclid(step) == 0 {
                    self.draw_pixel(px, py, true);
                }
            }
        }
    }

    fn fill_sparse_dots(&mut self, x: i32, y: i32, w: u32, h: u32) {
        for py in y..y.saturating_add(h as i32) {
            for px in x..x.saturating_add(w as i32) {
                if px.rem_euclid(2) == 0 && py.rem_euclid(2) == 0 {
                    self.draw_pixel(px, py, true);
                }
            }
        }
    }

    fn draw_dotted_hline(&mut self, x_start: i32, x_end: i32, y: i32, step: i32) {
        for x in x_start..=x_end {
            if x.rem_euclid(step.max(1)) == 0 {
                self.draw_pixel(x, y, true);
            }
        }
    }

    fn draw_dotted_vline(&mut self, x: i32, y_start: i32, y_end: i32, step: i32) {
        for y in y_start..=y_end {
            if y.rem_euclid(step.max(1)) == 0 {
                self.draw_pixel(x, y, true);
            }
        }
    }

    fn draw_chamfer_card(&mut self, x: i32, y: i32, w: u32, h: u32, shadow: u32) {
        let w_i32 = w as i32;
        let h_i32 = h as i32;
        if shadow > 0 {
            let s_i32 = shadow as i32;
            self.fill_rect(x + s_i32, y + h_i32, w, shadow, true);
            self.fill_rect(x + w_i32, y + s_i32, shadow, h, true);
        }
        self.fill_rect(x, y, w, h, false);
        self.stroke_rect(x, y, w, h, 2, true);
        for (cx, cy) in [
            (x, y),
            (x + w_i32 - 1, y),
            (x, y + h_i32 - 1),
            (x + w_i32 - 1, y + h_i32 - 1),
        ] {
            self.draw_pixel(cx, cy, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_2026_oct_15_to_lunar_and_renders_without_panics() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let lunar = solar_to_lunar(date);
        assert_eq!(lunar.ganzhi_year, "丙午");
        assert_eq!(lunar.zodiac, "马");
        assert_eq!(lunar.month_text, "九月");
        assert_eq!(lunar.day_text, "初六");
        assert_eq!(lunar.ganzhi_day, "壬戌日");

        let oct = render_calendar(
            CalendarState {
                year: 2026,
                month: 10,
                day: 15,
                page_id: 1,
            },
            CalendarConfig::default(),
        )
        .unwrap();
        assert_eq!(oct.frame.width, 400);
        assert_eq!(oct.frame.height, 300);
        assert!(oct.visible_text.iter().any(|t| t == "今日 15日"));
    }
}
