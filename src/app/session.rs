//! 会话编排层：连接配置、发现、采集与传输。

use std::sync::Mutex;

use super::platform::ensure_supported_runtime;
use super::{
    AppState, DeviceSupport, RairstreamError, SessionState, SpeakerDevice, UnsupportedReason,
};
use crate::audio::{RunningCapture, WindowsLoopbackCapture};
use crate::config::ReceiverCredentials;
use crate::discovery::DiscoveryService;
use crate::transport::{
    AirPlayError, PreparedConnection, PreparedTransportSession, RaopAudioSink, SessionDescriptor,
};
use tracing::{debug, info, warn};

fn map_transport_state(device_id: String, error: &AirPlayError) -> Option<SessionState> {
    match error {
        AirPlayError::PairingRequired => Some(SessionState::AwaitingPairing { device_id }),
        AirPlayError::CredentialsMissing => Some(SessionState::Authenticating { device_id }),
        AirPlayError::AuthenticationFailed { message }
            if message.contains("/pair-verify 返回了未预期状态码 500") =>
        {
            Some(SessionState::AwaitingPairing { device_id })
        }
        _ => None,
    }
}

/// 应用层准备完成但尚未真正发起网络握手的会话结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSession {
    pub app_state: AppState,
    pub transport: PreparedTransportSession,
}

struct ActiveStreamSession {
    device_id: String,
    capture: Option<RunningCapture>,
    transport: Option<PreparedConnection>,
}

impl ActiveStreamSession {
    fn new(device_id: String, capture: RunningCapture, transport: PreparedConnection) -> Self {
        Self {
            device_id,
            capture: Some(capture),
            transport: Some(transport),
        }
    }

    fn stop(mut self) -> Result<(), RairstreamError> {
        info!(device_id = %self.device_id, "开始停止活动串流会话");
        if let Some(capture) = self.capture.take() {
            capture.stop()?;
            debug!(device_id = %self.device_id, "系统音频捕获已停止");
        }

        if let Some(transport) = self.transport.take() {
            transport.teardown()?;
            debug!(device_id = %self.device_id, "RAOP 控制连接已拆除");
        }

        Ok(())
    }
}

impl Drop for ActiveStreamSession {
    fn drop(&mut self) {
        if let Some(capture) = self.capture.take()
            && let Err(error) = capture.stop()
        {
            warn!(device_id = %self.device_id, error = %error, "释放会话时停止音频捕获失败");
        }

        if let Some(transport) = self.transport.take()
            && let Err(error) = transport.teardown()
        {
            warn!(device_id = %self.device_id, error = %error, "释放会话时拆除 RAOP 连接失败");
        }
    }
}

/// 首版本会话协调器。
pub struct SessionCoordinator<D> {
    discovery: D,
    paired_receivers: Mutex<std::collections::HashMap<String, ReceiverCredentials>>,
    active_session: Mutex<Option<ActiveStreamSession>>,
}

impl<D> SessionCoordinator<D>
where
    D: DiscoveryService,
{
    pub fn new(discovery: D) -> Self {
        Self::with_paired_receivers(discovery, std::collections::HashMap::new())
    }

    pub fn with_paired_receivers(
        discovery: D,
        paired_receivers: std::collections::HashMap<String, ReceiverCredentials>,
    ) -> Self {
        Self {
            discovery,
            paired_receivers: Mutex::new(paired_receivers),
            active_session: Mutex::new(None),
        }
    }

    pub fn store_receiver_credentials(
        &self,
        device_id: impl Into<String>,
        receiver_credentials: ReceiverCredentials,
    ) -> Result<(), RairstreamError> {
        let mut paired_receivers = self.lock_paired_receivers()?;
        paired_receivers.insert(device_id.into(), receiver_credentials);
        Ok(())
    }

    pub fn discover(&self) -> Vec<SpeakerDevice> {
        debug!("开始发现 AirPlay / RAOP 设备");
        let devices = self.discovery.discover_devices();
        info!(device_count = devices.len(), "设备发现完成");
        devices
    }

    pub fn prepare_transport_session(
        &self,
        device: SpeakerDevice,
    ) -> Result<PreparedSession, RairstreamError> {
        info!(
            device_id = %device.id,
            device_name = %device.name,
            endpoint = %device.endpoint(),
            "开始准备传输会话"
        );
        ensure_supported_runtime()?;
        ensure_supported_target(&device)?;

        let format = WindowsLoopbackCapture::preferred_format()?;
        debug!(
            device_id = %device.id,
            sample_rate_hz = format.sample_rate_hz,
            channels = format.channels,
            bits_per_sample = format.bits_per_sample,
            sample_type = ?format.sample_type,
            "已获取默认回环采集格式"
        );
        let descriptor = self.session_descriptor(&device, format);
        let transport = PreparedTransportSession::prepare(&descriptor)?;
        debug!(
            device_id = %device.id,
            transport = transport.transport_name(),
            "传输准备入口已按设备类型完成分流"
        );
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

    pub fn pair_with_pin(
        &self,
        device: &SpeakerDevice,
        pin: &str,
    ) -> Result<ReceiverCredentials, RairstreamError> {
        info!(
            device_id = %device.id,
            device_name = %device.name,
            endpoint = %device.endpoint(),
            "开始执行首次配对编排"
        );
        ensure_supported_runtime()?;
        ensure_supported_target(device)?;

        let format = WindowsLoopbackCapture::preferred_format()?;
        let descriptor = self.session_descriptor(device, format);
        let transport = PreparedTransportSession::prepare(&descriptor)?;

        transport.pair_with_pin(pin).map_err(Into::into)
    }

    pub fn start_streaming_session(
        &self,
        device: SpeakerDevice,
    ) -> Result<AppState, RairstreamError> {
        info!(
            device_id = %device.id,
            device_name = %device.name,
            endpoint = %device.endpoint(),
            "开始启动串流会话"
        );
        ensure_supported_runtime()?;
        ensure_supported_target(&device)?;

        let format = WindowsLoopbackCapture::preferred_format()?;
        debug!(
            device_id = %device.id,
            sample_rate_hz = format.sample_rate_hz,
            channels = format.channels,
            bits_per_sample = format.bits_per_sample,
            sample_type = ?format.sample_type,
            "已获取串流采集格式"
        );
        let descriptor = self.session_descriptor(&device, format);
        let transport = PreparedTransportSession::prepare(&descriptor)?;
        debug!(
            device_id = %device.id,
            transport = transport.transport_name(),
            "开始执行分流后的传输握手"
        );
        let connection = match transport.handshake() {
            Ok(connection) => connection,
            Err(error) => {
                if let Some(active_session) = map_transport_state(device.id.clone(), &error) {
                    info!(device_id = %device.id, state = ?active_session, "传输握手进入认证相关状态");
                    return Ok(AppState {
                        selected_device_id: Some(device.id.clone()),
                        active_session,
                    });
                }
                return Err(error.into());
            }
        };
        info!(device_id = %device.id, "传输握手完成");
        let sink = RaopAudioSink::new(format, connection.stream_transport()?);
        let capture = match WindowsLoopbackCapture::start(sink) {
            Ok(capture) => capture,
            Err(error) => {
                warn!(device_id = %device.id, error = %error, "启动音频捕获失败，准备拆除 RAOP 会话");
                let _ = connection.teardown();
                return Err(error.into());
            }
        };
        debug!(device_id = %device.id, "系统音频捕获已启动");
        let previous = self.replace_active_session(ActiveStreamSession::new(
            device.id.clone(),
            capture,
            connection,
        ))?;

        if let Some(previous) = previous {
            info!(previous_device_id = %previous.device_id, "新会话已接管，停止旧串流会话");
            previous.stop()?;
        }

        info!(device_id = %device.id, "串流会话已进入 Streaming");
        Ok(AppState {
            selected_device_id: Some(device.id.clone()),
            active_session: SessionState::Streaming {
                device_id: device.id,
            },
        })
    }

    pub fn stop_streaming_session(&self) -> Result<AppState, RairstreamError> {
        info!("收到停止串流请求");
        let stopped_device_id = self.take_active_session()?.map(|session| {
            let device_id = session.device_id.clone();
            session.stop().map(|()| device_id)
        });
        let stopped_device_id = stopped_device_id.transpose()?;
        if let Some(device_id) = &stopped_device_id {
            info!(device_id = %device_id, "串流会话已停止并回到 Idle");
        } else {
            debug!("当前没有活动串流会话，保持 Idle");
        }

        Ok(AppState {
            selected_device_id: stopped_device_id,
            active_session: SessionState::Idle,
        })
    }

    fn session_descriptor(
        &self,
        device: &SpeakerDevice,
        format: crate::audio::AudioFormat,
    ) -> SessionDescriptor {
        let descriptor = SessionDescriptor::new(device.clone(), format);
        match self.paired_receiver(device.id.as_str()) {
            Ok(Some(receiver_credentials)) => {
                descriptor.with_receiver_credentials(receiver_credentials)
            }
            Ok(None) | Err(_) => descriptor,
        }
    }

    fn paired_receiver(
        &self,
        device_id: &str,
    ) -> Result<Option<ReceiverCredentials>, RairstreamError> {
        let paired_receivers = self.lock_paired_receivers()?;
        Ok(paired_receivers.get(device_id).cloned())
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

    fn lock_paired_receivers(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, ReceiverCredentials>>,
        RairstreamError,
    > {
        self.paired_receivers
            .lock()
            .map_err(|_| RairstreamError::InvalidConfiguration {
                message: String::from("配对凭据运行态锁已损坏"),
            })
    }
}

fn ensure_supported_target(device: &SpeakerDevice) -> Result<(), RairstreamError> {
    match device.support {
        DeviceSupport::Supported => Ok(()),
        DeviceSupport::Unsupported {
            reason: UnsupportedReason::AuthenticationRequiredReceiver,
        } => Err(AirPlayError::AuthenticationRequired.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCoordinator;
    use crate::app::{
        AirPlayGeneration, DeviceSupport, RairstreamError, ReceiverKind, SessionState,
        SpeakerDevice, UnsupportedReason,
    };
    use crate::audio::{AudioFormat, AudioSampleType};
    use crate::discovery::StubDiscoveryService;
    use crate::transport::{
        AirPlayError, ModernAirPlaySession, PreparedTransportSession, RaopSessionState,
        SessionDescriptor,
    };

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
            match prepared.transport {
                PreparedTransportSession::ClassicRaop(session) => {
                    assert_eq!(session.state(), RaopSessionState::Connecting);
                }
                PreparedTransportSession::ModernAirPlay(_) => {
                    panic!("stub device should still use classic raop path");
                }
            }
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

    #[test]
    fn prepared_transport_session_uses_airplay_receiver_branch() {
        let descriptor = SessionDescriptor::new(
            SpeakerDevice {
                id: String::from("receiver"),
                name: String::from("Receiver"),
                host: String::from("192.168.1.50"),
                port: 7000,
                generation: AirPlayGeneration::AirPlay1,
                pairing_id: None,
                receiver_public_key: None,
                receiver_kind: ReceiverKind::ModernAirPlayAuth,
                support: DeviceSupport::Supported,
            },
            AudioFormat {
                sample_rate_hz: 44_100,
                channels: 2,
                bits_per_sample: 16,
                sample_type: AudioSampleType::Int,
            },
        );

        let prepared = PreparedTransportSession::prepare(&descriptor).unwrap();

        match prepared {
            PreparedTransportSession::ModernAirPlay(session) => {
                assert_eq!(
                    session.descriptor().device.receiver_kind,
                    ReceiverKind::ModernAirPlayAuth
                );
            }
            PreparedTransportSession::ClassicRaop(_) => {
                panic!("AirPlay Receiver should use the authenticated prepare branch");
            }
        }
    }

    #[test]
    fn airplay_receiver_transport_error_maps_to_auth_related_session_state() {
        let pairing_state =
            super::map_transport_state(String::from("receiver"), &AirPlayError::PairingRequired);
        let credential_state =
            super::map_transport_state(String::from("receiver"), &AirPlayError::CredentialsMissing);
        let auth_failed_state = super::map_transport_state(
            String::from("receiver"),
            &AirPlayError::AuthenticationFailed {
                message: String::from("forbidden"),
            },
        );
        let pair_verify_500_state = super::map_transport_state(
            String::from("receiver"),
            &AirPlayError::AuthenticationFailed {
                message: String::from("/pair-verify 返回了未预期状态码 500"),
            },
        );

        assert_eq!(
            pairing_state,
            Some(SessionState::AwaitingPairing {
                device_id: String::from("receiver")
            })
        );
        assert_eq!(
            credential_state,
            Some(SessionState::Authenticating {
                device_id: String::from("receiver")
            })
        );
        assert_eq!(
            pair_verify_500_state,
            Some(SessionState::AwaitingPairing {
                device_id: String::from("receiver")
            })
        );
        assert!(auth_failed_state.is_none());
    }

    #[test]
    fn airplay_receiver_prepare_path_can_surface_auth_session_state_before_streaming() {
        let session = ModernAirPlaySession::connect(&SessionDescriptor::new(
            SpeakerDevice {
                id: String::from("receiver"),
                name: String::from("Receiver"),
                host: String::from("192.168.1.50"),
                port: 7000,
                generation: AirPlayGeneration::AirPlay1,
                pairing_id: None,
                receiver_public_key: None,
                receiver_kind: ReceiverKind::ModernAirPlayAuth,
                support: DeviceSupport::Supported,
            },
            AudioFormat {
                sample_rate_hz: 44_100,
                channels: 2,
                bits_per_sample: 16,
                sample_type: AudioSampleType::Int,
            },
        ))
        .unwrap();

        assert_eq!(
            session.descriptor().device.receiver_kind,
            ReceiverKind::ModernAirPlayAuth
        );
    }

    #[test]
    fn coordinator_rejects_unsupported_device_before_prepare_transport() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let device = SpeakerDevice {
            id: String::from("receiver"),
            name: String::from("Receiver"),
            host: String::from("192.168.1.50"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ModernAirPlayAuth,
            support: DeviceSupport::Unsupported {
                reason: UnsupportedReason::AuthenticationRequiredReceiver,
            },
        };

        let error = coordinator.prepare_transport_session(device).unwrap_err();

        if cfg!(target_os = "windows") {
            assert!(matches!(
                error,
                RairstreamError::Transport(AirPlayError::AuthenticationRequired)
            ));
        } else {
            assert!(matches!(
                error,
                RairstreamError::NotImplemented {
                    feature: "non-Windows runtime support"
                }
            ));
        }
    }

    #[test]
    fn coordinator_rejects_unsupported_device_before_start_streaming() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let device = SpeakerDevice {
            id: String::from("receiver"),
            name: String::from("Receiver"),
            host: String::from("192.168.1.50"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ModernAirPlayAuth,
            support: DeviceSupport::Unsupported {
                reason: UnsupportedReason::AuthenticationRequiredReceiver,
            },
        };

        let error = coordinator.start_streaming_session(device).unwrap_err();

        if cfg!(target_os = "windows") {
            assert!(matches!(
                error,
                RairstreamError::Transport(AirPlayError::AuthenticationRequired)
            ));
        } else {
            assert!(matches!(
                error,
                RairstreamError::NotImplemented {
                    feature: "non-Windows runtime support"
                }
            ));
        }
    }
}
