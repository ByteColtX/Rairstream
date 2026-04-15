//! 应用层：维护共享领域模型与会话编排入口。

pub mod platform;
mod session;

use crate::audio::AudioCaptureError;
use crate::config::ConfigError;
use crate::transport::AirPlayError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use session::{PreparedSession, SessionCoordinator};

/// 当前应用支持的目标流协议版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirPlayGeneration {
    /// 首版本仅计划支持 `AirPlay` 1 / `RAOP` 兼容设备。
    AirPlay1,
}

/// 当前 MVP 对目标设备的支持状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DeviceSupport {
    #[default]
    Supported,
    Unsupported {
        reason: UnsupportedReason,
    },
}

/// 当前 MVP 尚未覆盖的设备限制原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnsupportedReason {
    AuthenticationRequiredReceiver,
}

/// 设备会话建立时需要走的握手路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReceiverKind {
    #[default]
    ClassicRaop,
    ModernAirPlayAuth,
}

/// 发现到的输出设备摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerDevice {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub generation: AirPlayGeneration,
    #[serde(default)]
    pub pairing_id: Option<String>,
    #[serde(default)]
    pub receiver_public_key: Option<String>,
    #[serde(default)]
    pub receiver_kind: ReceiverKind,
    #[serde(default)]
    pub support: DeviceSupport,
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
    AwaitingPairing { device_id: String },
    Authenticating { device_id: String },
    Streaming { device_id: String },
}

/// 各子系统统一使用的错误类型。
#[derive(Debug, Error)]
pub enum RairstreamError {
    #[error("功能尚未实现: {feature}")]
    NotImplemented { feature: &'static str },
    #[error("配置错误: {message}")]
    InvalidConfiguration { message: String },
    #[error(transparent)]
    AudioCapture(#[from] AudioCaptureError),
    #[error(transparent)]
    Transport(#[from] AirPlayError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

#[cfg(test)]
mod tests {
    use super::{
        AirPlayGeneration, AppState, DeviceSupport, ReceiverKind, SessionState, SpeakerDevice,
    };

    #[test]
    fn speaker_device_builds_endpoint() {
        let device = SpeakerDevice {
            id: String::from("living-room"),
            name: String::from("Living Room"),
            host: String::from("192.168.1.50"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ClassicRaop,
            support: DeviceSupport::Supported,
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
