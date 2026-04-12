//! `AirPlay` / `RAOP` 会话与传输边界。

use crate::app::{RairstreamError, SpeakerDevice};
use thiserror::Error;

/// 流建立前需要的最小会话信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    pub device: SpeakerDevice,
}

/// `AirPlay` 传输层错误。
#[derive(Debug, Error)]
pub enum AirPlayError {
    #[error("会话尚未接入真实 RAOP 实现")]
    NotReady,
}

/// 首版本的 RAOP 传输占位实现。
#[derive(Debug, Default)]
pub struct RaopSession;

impl RaopSession {
    #[must_use]
    pub fn transport_name() -> &'static str {
        "raop"
    }

    pub fn connect(_descriptor: &SessionDescriptor) -> Result<(), RairstreamError> {
        Err(RairstreamError::NotImplemented {
            feature: "RAOP session connect",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RaopSession;

    #[test]
    fn raop_session_reports_transport_name() {
        assert_eq!(RaopSession::transport_name(), "raop");
    }
}
