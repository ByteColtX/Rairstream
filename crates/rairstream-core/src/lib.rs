//! Rairstream 核心领域模型与共享契约。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 当前应用支持的目标流协议版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirPlayGeneration {
    /// 首版本仅计划支持 AirPlay 1 / RAOP 兼容设备。
    AirPlay1,
}

/// 发现到的输出设备摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerDevice {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub generation: AirPlayGeneration,
}

impl SpeakerDevice {
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// 应用启动后维护的最小运行状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub selected_device_id: Option<String>,
    pub active_session: SessionState,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            selected_device_id: None,
            active_session: SessionState::Idle,
        }
    }
}

/// 流会话状态机的初始形态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Idle,
    Discovering,
    Connecting { device_id: String },
    Streaming { device_id: String },
}

/// 各子系统统一使用的错误类型。
#[derive(Debug, Error)]
pub enum RairstreamError {
    #[error("功能尚未实现: {feature}")]
    NotImplemented { feature: &'static str },
    #[error("配置错误: {message}")]
    InvalidConfiguration { message: String },
}

/// 为后续多平台音频采集预留的抽象。
pub trait CaptureBackend {
    fn backend_name(&self) -> &'static str;
}

/// 为后续不同 AirPlay 会话实现预留的抽象。
pub trait SessionBackend {
    fn transport_name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::{AirPlayGeneration, AppState, SessionState, SpeakerDevice};

    #[test]
    fn speaker_device_builds_endpoint() {
        let device = SpeakerDevice {
            id: String::from("living-room"),
            name: String::from("Living Room"),
            host: String::from("192.168.1.50"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
        };

        assert_eq!(device.endpoint(), "192.168.1.50:7000");
    }

    #[test]
    fn app_state_defaults_to_idle() {
        let state = AppState::default();

        assert!(state.selected_device_id.is_none());
        assert_eq!(state.active_session, SessionState::Idle);
    }
}
