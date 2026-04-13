//! 会话编排层：连接配置、发现、采集与传输。

use std::sync::Mutex;

use super::platform::ensure_supported_runtime;
use super::{AppState, RairstreamError, SessionState, SpeakerDevice};
use crate::audio::{RunningCapture, WindowsLoopbackCapture};
use crate::discovery::DiscoveryService;
use crate::transport::{RaopAudioSink, RaopConnection, RaopSession, SessionDescriptor};

/// 应用层准备完成但尚未真正发起网络握手的会话结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSession {
    pub app_state: AppState,
    pub transport: RaopSession,
}

struct ActiveStreamSession {
    device_id: String,
    capture: Option<RunningCapture>,
    transport: Option<RaopConnection>,
}

impl ActiveStreamSession {
    fn new(device_id: String, capture: RunningCapture, transport: RaopConnection) -> Self {
        Self {
            device_id,
            capture: Some(capture),
            transport: Some(transport),
        }
    }

    fn stop(mut self) -> Result<(), RairstreamError> {
        if let Some(capture) = self.capture.take() {
            capture.stop()?;
        }

        if let Some(transport) = self.transport.take() {
            transport.teardown()?;
        }

        Ok(())
    }
}

impl Drop for ActiveStreamSession {
    fn drop(&mut self) {
        if let Some(capture) = self.capture.take() {
            let _ = capture.stop();
        }

        if let Some(transport) = self.transport.take() {
            let _ = transport.teardown();
        }
    }
}

/// 首版本会话协调器。
pub struct SessionCoordinator<D> {
    discovery: D,
    active_session: Mutex<Option<ActiveStreamSession>>,
}

impl<D> SessionCoordinator<D>
where
    D: DiscoveryService,
{
    pub fn new(discovery: D) -> Self {
        Self {
            discovery,
            active_session: Mutex::new(None),
        }
    }

    pub fn discover(&self) -> Vec<SpeakerDevice> {
        self.discovery.discover_devices()
    }

    pub fn prepare_transport_session(
        &self,
        device: SpeakerDevice,
    ) -> Result<PreparedSession, RairstreamError> {
        ensure_supported_runtime()?;

        let format = WindowsLoopbackCapture::preferred_format()?;
        let descriptor = SessionDescriptor::new(device.clone(), format);
        let transport = RaopSession::connect(&descriptor)?;
        let app_state = AppState {
            selected_device_id: Some(device.id.clone()),
            active_session: SessionState::Connecting {
                device_id: device.id,
            },
        };

        Ok(PreparedSession {
            app_state,
            transport,
        })
    }

    pub fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
        Ok(self.prepare_transport_session(device)?.app_state)
    }

    pub fn start_streaming_session(
        &self,
        device: SpeakerDevice,
    ) -> Result<AppState, RairstreamError> {
        ensure_supported_runtime()?;

        let format = WindowsLoopbackCapture::preferred_format()?;
        let descriptor = SessionDescriptor::new(device.clone(), format);
        let transport = RaopSession::connect(&descriptor)?;
        let connection = transport.handshake()?;
        let sink = RaopAudioSink::new(format, connection.stream_transport()?);
        let capture = match WindowsLoopbackCapture::start(sink) {
            Ok(capture) => capture,
            Err(error) => {
                let _ = connection.teardown();
                return Err(error.into());
            }
        };
        let previous = self.replace_active_session(ActiveStreamSession::new(
            device.id.clone(),
            capture,
            connection,
        ))?;

        if let Some(previous) = previous {
            previous.stop()?;
        }

        Ok(AppState {
            selected_device_id: Some(device.id.clone()),
            active_session: SessionState::Streaming {
                device_id: device.id,
            },
        })
    }

    pub fn stop_streaming_session(&self) -> Result<AppState, RairstreamError> {
        let stopped_device_id = self.take_active_session()?.map(|session| {
            let device_id = session.device_id.clone();
            session.stop().map(|()| device_id)
        });

        Ok(AppState {
            selected_device_id: stopped_device_id.transpose()?,
            active_session: SessionState::Idle,
        })
    }

    fn replace_active_session(
        &self,
        session: ActiveStreamSession,
    ) -> Result<Option<ActiveStreamSession>, RairstreamError> {
        let mut active_session = self.lock_active_session()?;
        Ok(active_session.replace(session))
    }

    fn take_active_session(&self) -> Result<Option<ActiveStreamSession>, RairstreamError> {
        let mut active_session = self.lock_active_session()?;
        Ok(active_session.take())
    }

    fn lock_active_session(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<ActiveStreamSession>>, RairstreamError> {
        self.active_session
            .lock()
            .map_err(|_| RairstreamError::InvalidConfiguration {
                message: String::from("会话运行态锁已损坏"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCoordinator;
    use crate::app::SessionState;
    use crate::discovery::StubDiscoveryService;
    use crate::transport::RaopSessionState;

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

    #[test]
    fn coordinator_can_prepare_transport_session_on_windows_only() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let device = coordinator.discover().remove(0);
        let result = coordinator.prepare_transport_session(device);

        if cfg!(target_os = "windows") {
            let prepared = result.expect("windows should be supported");
            assert!(matches!(
                prepared.app_state.active_session,
                SessionState::Connecting { .. }
            ));
            assert_eq!(prepared.transport.state(), RaopSessionState::Connecting);
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn coordinator_stop_without_active_stream_returns_idle_state() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let state = coordinator.stop_streaming_session().unwrap();

        assert_eq!(state.active_session, SessionState::Idle);
        assert!(state.selected_device_id.is_none());
    }
}
