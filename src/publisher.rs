use reqwest::blocking::Client;
use reqwest::blocking::multipart::{Form, Part};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use thiserror::Error;

use crate::CalendarOutput;

/// 默认绑定的 ZECTRIX NOTE4 物理设备 ID
pub const DEFAULT_DEVICE_ID: &str = "";
/// 默认月历推送通道（第 1 页）
pub const DEFAULT_PAGE_ID: u8 = 1;
/// 第 3 页已被 Codex 额度监控与重置雷达独占常驻
pub const CODEX_RESERVED_PAGE_ID: u8 = 3;

#[derive(Debug, Error)]
pub enum ZectrixPublishError {
    #[error("ZECTRIX 发布配置无效：{0}")]
    InvalidConfiguration(String),
    #[error("无法编码月历 PNG 图像")]
    Image,
    #[error("网络请求失败：{0}")]
    Network(String),
    #[error("ZECTRIX 云端返回错误：HTTP {status}（code: {code}, message: {message}）")]
    ApiError {
        status: u16,
        code: i64,
        message: String,
    },
    #[error("ZECTRIX 云端接口暂时不可用：{0}")]
    Unavailable(String),
}

/// 校验月历通道：允许 1、2、4、5 页，默认拦截已被 Codex 额度雷达占用的第 3 页
pub fn validate_calendar_page_id(page_id: u8, allow_page_3: bool) -> Result<u8, ZectrixPublishError> {
    if !(1..=5).contains(&page_id) {
        return Err(ZectrixPublishError::InvalidConfiguration(
            "ZECTRIX_PAGE_ID 必须在 1 到 5 之间".into(),
        ));
    }
    if page_id == CODEX_RESERVED_PAGE_ID && !allow_page_3 {
        return Err(ZectrixPublishError::InvalidConfiguration(
            "第 3 页已被 Codex 看板预留；如需覆盖请显式设置 ALLOW_PAGE_3_OVERRIDE=true".into(),
        ));
    }
    Ok(page_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_page_is_reserved_without_explicit_override() {
        assert!(validate_calendar_page_id(3, false).is_err());
        assert_eq!(validate_calendar_page_id(3, true).unwrap(), 3);
        for page in [1, 2, 4, 5] {
            assert_eq!(validate_calendar_page_id(page, false).unwrap(), page);
        }
        assert!(validate_calendar_page_id(0, true).is_err());
        assert!(validate_calendar_page_id(6, true).is_err());
    }
}

pub trait FramePublisher {
    type Error;

    fn publish(&mut self, calendar: &CalendarOutput) -> Result<(), Self::Error>;
}

pub struct ZectrixPublisher {
    client: Client,
    base_url: String,
    device_id: String,
    page_id: u8,
}

impl ZectrixPublisher {
    pub fn new(
        api_key: &str,
        base_url: impl Into<String>,
        device_id: impl Into<String>,
        page_id: u8,
        allow_page_3: bool,
    ) -> Result<Self, ZectrixPublishError> {
        let page_id = validate_calendar_page_id(page_id, allow_page_3)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-API-Key",
            HeaderValue::from_str(api_key.trim()).map_err(|_| {
                ZectrixPublishError::InvalidConfiguration("X-API-Key 格式无效".into())
            })?,
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| {
                ZectrixPublishError::InvalidConfiguration("HTTP 客户端初始化失败".into())
            })?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            device_id: device_id.into(),
            page_id,
        })
    }
}

#[derive(Deserialize)]
struct ApiCodeResponse {
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

impl FramePublisher for ZectrixPublisher {
    type Error = ZectrixPublishError;

    fn publish(&mut self, calendar: &CalendarOutput) -> Result<(), Self::Error> {
        let image_bytes = calendar
            .frame
            .png_bytes()
            .map_err(|_| ZectrixPublishError::Image)?;
        if image_bytes.len() > 2 * 1024 * 1024 {
            return Err(ZectrixPublishError::Image);
        }
        let image_part = Part::bytes(image_bytes)
            .file_name("zectrix-calendar.png")
            .mime_str("image/png")
            .map_err(|_| ZectrixPublishError::Image)?;
        let form = Form::new()
            .text("dither", "false")
            .text("pageId", self.page_id.to_string())
            .part("images", image_part);
        let response = self
            .client
            .post(format!(
                "{}/open/v1/devices/{}/display/image",
                self.base_url, self.device_id
            ))
            .multipart(form)
            .send()
            .map_err(|e| ZectrixPublishError::Network(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body_text = response.text().unwrap_or_default();
            return Err(ZectrixPublishError::Unavailable(format!(
                "HTTP {status_code}: {body_text}"
            )));
        }
        let payload: ApiCodeResponse = response
            .json()
            .map_err(|e| ZectrixPublishError::Unavailable(format!("响应解析失败: {e}")))?;
        if payload.code != 0 {
            let message = payload
                .message
                .or(payload.msg)
                .unwrap_or_else(|| "未知云端错误".into());
            return Err(ZectrixPublishError::ApiError {
                status: status.as_u16(),
                code: payload.code,
                message,
            });
        }
        Ok(())
    }
}
