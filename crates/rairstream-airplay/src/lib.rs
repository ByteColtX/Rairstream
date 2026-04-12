//! AirPlay / RAOP 会话与传输边界。

use rairstream_core::{RairstreamError, SessionBackend, SpeakerDevice};
use thiserror::Error;

/// 流建立前需要的最小会话信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    pub device: SpeakerDevice,
}

/// AirPlay 传输层错误。
#[derive(Debug, Error)]
pub enum AirPlayError {
    #[error("会话尚未接入真实 RAOP 实现")]
    NotReady,
}

/// 首版本的 RAOP 传输占位实现。
#[derive(Debug, Default)]
pub struct RaopSession;

impl SessionBackend for RaopSession {
    fn transport_name(&self) -> &'static str {
        "raop"
    }
}

impl RaopSession {
    pub fn connect(&self, _descriptor: &SessionDescriptor) -> Result<(), RairstreamError> {
        Err(RairstreamError::NotImplemented {
            feature: "RAOP session connect",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RaopSession;
    use rairstream_core::SessionBackend;

    #[test]
    fn raop_session_reports_transport_name() {
        let session = RaopSession;

        assert_eq!(session.transport_name(), "raop");
    }
}
