//! 会话编排层：连接配置、发现、采集与传输。

use super::platform::ensure_supported_runtime;
use super::{AppState, RairstreamError, SessionState, SpeakerDevice};
use crate::audio::WindowsLoopbackCapture;
use crate::discovery::DiscoveryService;
use crate::transport::{RaopSession, SessionDescriptor};

/// 首版本会话协调器。
pub struct SessionCoordinator<D> {
    discovery: D,
}

impl<D> SessionCoordinator<D>
where
    D: DiscoveryService,
{
    pub fn new(discovery: D) -> Self {
        Self { discovery }
    }

    pub fn discover(&self) -> Vec<SpeakerDevice> {
        self.discovery.discover_devices()
    }

    pub fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
        ensure_supported_runtime()?;

        let _format = WindowsLoopbackCapture::preferred_format()?;
        let descriptor = SessionDescriptor {
            device: device.clone(),
        };
        let _transport = RaopSession::transport_name();
        let _ = descriptor;

        Ok(AppState {
            selected_device_id: Some(device.id.clone()),
            active_session: SessionState::Connecting {
                device_id: device.id,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCoordinator;
    use crate::app::SessionState;
    use crate::discovery::StubDiscoveryService;

    #[test]
    fn coordinator_exposes_discovered_devices() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);

        assert_eq!(coordinator.discover().len(), 1);
    }

    #[test]
    fn coordinator_can_prepare_stub_session_on_windows_only() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let device = coordinator.discover().remove(0);
        let result = coordinator.prepare_session(device);

        if cfg!(target_os = "windows") {
            let state = result.expect("windows should be supported");
            assert!(matches!(
                state.active_session,
                SessionState::Connecting { .. }
            ));
        } else {
            assert!(result.is_err());
        }
    }
}
