use super::{AppState, RairstreamError, SessionCoordinator, SessionState, SpeakerDevice, platform};
use crate::config::{AppConfig, MAX_SENDER_VOLUME_PERCENT, ReceiverCredentials};
use crate::discovery::DiscoveryService;
use tracing::{debug, info, warn};

pub trait SessionControlService {
    fn discover(&self) -> Vec<SpeakerDevice>;
    fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError>;
    fn pair_with_pin(
        &self,
        device: SpeakerDevice,
        pin: &str,
    ) -> Result<ReceiverCredentials, RairstreamError>;
    fn store_receiver_credentials(
        &self,
        device_id: String,
        receiver_credentials: ReceiverCredentials,
    ) -> Result<(), RairstreamError>;
    fn start_streaming_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError>;
    fn stop_streaming_session(&self) -> Result<AppState, RairstreamError>;
    fn reconcile_app_state(
        &self,
        selected_device_id: Option<String>,
    ) -> Result<AppState, RairstreamError>;
    fn set_sender_volume_percent(&self, percent: u8) -> Result<(), RairstreamError>;
}

impl<D> SessionControlService for SessionCoordinator<D>
where
    D: DiscoveryService,
{
    fn discover(&self) -> Vec<SpeakerDevice> {
        SessionCoordinator::discover(self)
    }

    fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
        SessionCoordinator::prepare_session(self, device)
    }

    fn pair_with_pin(
        &self,
        device: SpeakerDevice,
        pin: &str,
    ) -> Result<ReceiverCredentials, RairstreamError> {
        SessionCoordinator::pair_with_pin(self, &device, pin)
    }

    fn store_receiver_credentials(
        &self,
        device_id: String,
        receiver_credentials: ReceiverCredentials,
    ) -> Result<(), RairstreamError> {
        SessionCoordinator::store_receiver_credentials(self, device_id, receiver_credentials)
    }

    fn start_streaming_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
        SessionCoordinator::start_streaming_session(self, device)
    }

    fn stop_streaming_session(&self) -> Result<AppState, RairstreamError> {
        SessionCoordinator::stop_streaming_session(self)
    }

    fn reconcile_app_state(
        &self,
        selected_device_id: Option<String>,
    ) -> Result<AppState, RairstreamError> {
        SessionCoordinator::reconcile_app_state(self, selected_device_id)
    }

    fn set_sender_volume_percent(&self, percent: u8) -> Result<(), RairstreamError> {
        SessionCoordinator::set_sender_volume_percent(self, percent)
    }
}

#[derive(Debug)]
pub struct AppController<S> {
    session_service: S,
    config: AppConfig,
    app_state: AppState,
    devices: Vec<SpeakerDevice>,
    sender_volume_percent: u8,
    sender_muted: bool,
    last_error: Option<String>,
}

impl<S> AppController<S>
where
    S: SessionControlService,
{
    #[must_use]
    pub fn new(session_service: S, config: AppConfig) -> Self {
        let preferred_device_id = config.preferred_device_id.clone();
        let sender_volume_percent = config.sender_volume_percent;
        let sender_muted = config.sender_muted;
        let launch_at_startup =
            platform::is_launch_at_startup_enabled().unwrap_or(config.launch_at_startup);
        let mut config = config;
        config.set_launch_at_startup(launch_at_startup);
        Self {
            session_service,
            config,
            app_state: AppState {
                selected_device_id: preferred_device_id,
                active_session: SessionState::Idle,
            },
            devices: Vec::new(),
            sender_volume_percent,
            sender_muted,
            last_error: None,
        }
    }

    #[must_use]
    pub fn app_state(&self) -> &AppState {
        &self.app_state
    }

    #[must_use]
    pub fn devices(&self) -> &[SpeakerDevice] {
        &self.devices
    }

    #[must_use]
    pub fn auto_reconnect(&self) -> bool {
        self.config.auto_reconnect
    }

    #[must_use]
    pub fn launch_at_startup(&self) -> bool {
        self.config.launch_at_startup
    }

    #[must_use]
    pub fn sender_volume_percent(&self) -> u8 {
        self.sender_volume_percent
    }

    #[must_use]
    pub fn sender_muted(&self) -> bool {
        self.sender_muted
    }

    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    #[must_use]
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn refresh_devices(&mut self) {
        debug!("请求刷新设备列表");
        self.app_state.active_session = SessionState::Discovering;

        let devices = self.session_service.discover();
        let selected_device_id = self.retain_selected_device_id(&devices);

        self.devices = devices;
        self.app_state = self
            .session_service
            .reconcile_app_state(selected_device_id)
            .unwrap_or_else(|error| {
                warn!(error = %error, "刷新设备后对账运行态失败，回退到 Idle");
                AppState {
                    selected_device_id: self.retain_selected_device_id(&self.devices),
                    active_session: SessionState::Idle,
                }
            });
        self.last_error = None;

        info!(device_count = self.devices.len(), "设备列表刷新完成");
    }

    pub fn initialize(&mut self) {
        self.refresh_devices();
        if matches!(
            self.app_state.active_session,
            SessionState::Streaming { .. }
        ) {
            return;
        }
        if !self.config.auto_reconnect {
            return;
        }

        let Some(device_id) = self.app_state.selected_device_id.clone() else {
            return;
        };

        if let Err(error) = self.select_device(&device_id) {
            self.handle_error(Some(&device_id), &error);
        }
    }

    pub fn resolve_target_device(
        &self,
        device_filter: Option<&str>,
    ) -> Result<SpeakerDevice, RairstreamError> {
        if let Some(filter) = device_filter {
            return self.find_device_by_filter(filter);
        }

        if let Some(selected_device_id) = self.app_state.selected_device_id.as_deref()
            && let Ok(device) = self.find_device(selected_device_id)
        {
            return Ok(device);
        }

        match self.devices.as_slice() {
            [device] => Ok(device.clone()),
            [] => Err(RairstreamError::InvalidConfiguration {
                message: String::from("未发现可用 AirPlay / RAOP 设备"),
            }),
            _ => Err(RairstreamError::InvalidConfiguration {
                message: String::from(
                    "发现多个设备，请使用 --device 指定目标设备，或先设置上次使用设备",
                ),
            }),
        }
    }

    pub fn select_device(&mut self, device_id: &str) -> Result<(), RairstreamError> {
        info!(device_id, "请求选择设备");
        if matches!(
            &self.app_state.active_session,
            SessionState::Streaming {
                device_id: active_device_id,
            } if active_device_id == device_id
        ) {
            info!(device_id, "目标设备已在串流，转为停止串流");
            return self.stop_streaming();
        }

        let device = self.find_device(device_id)?;
        self.session_service
            .set_sender_volume_percent(self.effective_sender_volume_percent())?;
        let app_state = self
            .session_service
            .start_streaming_session(device.clone())?;

        self.config.set_preferred_device_id(Some(device.id.clone()));
        self.config.save()?;
        self.app_state = app_state;
        self.last_error = None;

        Ok(())
    }

    pub fn submit_pairing_pin(
        &mut self,
        device_id: &str,
        pin: &str,
    ) -> Result<(), RairstreamError> {
        info!(device_id, "提交首次配对 PIN");
        let device = self.find_device(device_id)?;
        let receiver_credentials = self.session_service.pair_with_pin(device.clone(), pin)?;
        self.session_service
            .store_receiver_credentials(device.id.clone(), receiver_credentials.clone())?;
        self.config
            .upsert_paired_receiver(device.id.clone(), receiver_credentials);
        self.config.save()?;
        self.app_state.selected_device_id = Some(device.id.clone());
        self.app_state.active_session = SessionState::Authenticating {
            device_id: device.id.clone(),
        };
        self.session_service
            .set_sender_volume_percent(self.effective_sender_volume_percent())?;
        let app_state = self.session_service.start_streaming_session(device)?;
        self.app_state = app_state;
        self.last_error = None;
        Ok(())
    }

    pub fn cancel_pairing(&mut self, device_id: &str) -> Result<(), RairstreamError> {
        info!(device_id, "取消首次配对输入");
        let device = self.find_device(device_id)?;
        self.app_state = AppState {
            selected_device_id: Some(device.id),
            active_session: SessionState::Idle,
        };
        self.last_error = None;
        Ok(())
    }

    pub fn stop_streaming(&mut self) -> Result<(), RairstreamError> {
        info!("请求停止串流");
        let mut app_state = self.session_service.stop_streaming_session()?;
        app_state
            .selected_device_id
            .clone_from(&self.app_state.selected_device_id);
        self.config
            .set_preferred_device_id(app_state.selected_device_id.clone());
        self.config.save()?;
        self.app_state = app_state;
        self.last_error = None;
        Ok(())
    }

    pub fn toggle_auto_reconnect(&mut self) -> Result<(), RairstreamError> {
        self.config.auto_reconnect = !self.config.auto_reconnect;
        self.config.save()?;
        self.last_error = None;
        Ok(())
    }

    pub fn toggle_launch_at_startup(&mut self) -> Result<(), RairstreamError> {
        let enabled = !self.config.launch_at_startup;
        platform::set_launch_at_startup_enabled(enabled)?;
        self.config.set_launch_at_startup(enabled);
        self.config.save()?;
        self.last_error = None;
        Ok(())
    }

    pub fn set_sender_volume(&mut self, percent: u8) -> Result<(), RairstreamError> {
        let percent = percent.min(MAX_SENDER_VOLUME_PERCENT);
        self.config.set_sender_volume_percent(percent);
        self.config.save()?;
        self.sender_volume_percent = self.config.sender_volume_percent;

        if matches!(
            self.app_state.active_session,
            SessionState::Streaming { .. }
        ) {
            self.session_service
                .set_sender_volume_percent(self.effective_sender_volume_percent())?;
        }

        self.last_error = None;
        Ok(())
    }

    pub fn toggle_sender_mute(&mut self) -> Result<(), RairstreamError> {
        self.config.set_sender_muted(!self.config.sender_muted);
        self.config.save()?;
        self.sender_muted = self.config.sender_muted;

        if matches!(
            self.app_state.active_session,
            SessionState::Streaming { .. }
        ) {
            self.session_service
                .set_sender_volume_percent(self.effective_sender_volume_percent())?;
        }

        self.last_error = None;
        Ok(())
    }

    #[must_use]
    pub fn effective_sender_volume_percent(&self) -> u8 {
        if self.sender_muted {
            0
        } else {
            self.sender_volume_percent
        }
    }

    pub fn handle_error(&mut self, device_id: Option<&str>, error: &RairstreamError) {
        self.recover_from_error(device_id, error);
        self.last_error = Some(self.describe_error(device_id, error));
    }

    #[must_use]
    pub fn describe_error(&self, device_id: Option<&str>, error: &RairstreamError) -> String {
        let device_name = device_id.and_then(|device_id| {
            self.devices
                .iter()
                .find(|device| device.id == device_id)
                .map(|device| device.name.as_str())
        });

        match error {
            RairstreamError::Transport(crate::transport::AirPlayError::PairingRequired) => {
                match device_name {
                    Some(device_name) => {
                        format!("{device_name} 需要先完成首次配对，请重新选择并输入 PIN")
                    }
                    None => String::from("目标设备需要先完成首次配对，请重新选择并输入 PIN"),
                }
            }
            RairstreamError::Transport(crate::transport::AirPlayError::CredentialsMissing) => {
                match device_name {
                    Some(device_name) => format!("{device_name} 缺少可用配对记录，请重新配对"),
                    None => String::from("目标设备缺少可用配对记录，请重新配对"),
                }
            }
            RairstreamError::Transport(crate::transport::AirPlayError::AuthenticationFailed {
                ..
            }) => match device_name {
                Some(device_name) => format!("{device_name} 认证失败，请重新配对后再试"),
                None => String::from("目标设备认证失败，请重新配对后再试"),
            },
            _ => error.to_string(),
        }
    }

    fn find_device_by_filter(&self, filter: &str) -> Result<SpeakerDevice, RairstreamError> {
        for matcher in [
            DeviceMatcher::ExactId,
            DeviceMatcher::ExactName,
            DeviceMatcher::ExactHost,
            DeviceMatcher::Contains,
        ] {
            let matches = self
                .devices
                .iter()
                .filter(|device| matcher.matches(device, filter))
                .cloned()
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [device] => return Ok(device.clone()),
                [] => {}
                _ => {
                    return Err(RairstreamError::InvalidConfiguration {
                        message: format!("设备过滤条件匹配到多个目标: {filter}"),
                    });
                }
            }
        }

        Err(RairstreamError::InvalidConfiguration {
            message: format!("未找到匹配设备: {filter}"),
        })
    }

    fn retain_selected_device_id(&self, devices: &[SpeakerDevice]) -> Option<String> {
        self.app_state
            .selected_device_id
            .as_ref()
            .filter(|selected_device_id| {
                devices
                    .iter()
                    .any(|device| device.id.as_str() == selected_device_id.as_str())
            })
            .cloned()
    }

    fn recover_from_error(&mut self, device_id: Option<&str>, error: &RairstreamError) {
        if let RairstreamError::Transport(
            crate::transport::AirPlayError::CredentialsMissing
            | crate::transport::AirPlayError::AuthenticationFailed { .. },
        ) = error
        {
            let Some(device_id) = device_id else {
                return;
            };
            self.config.remove_paired_receiver(device_id);
            if let Err(save_error) = self.config.save() {
                warn!(device_id, error = %save_error, "清理失效配对记录失败，保留内存态更新");
            }
        }
    }

    fn find_device(&self, device_id: &str) -> Result<SpeakerDevice, RairstreamError> {
        self.devices
            .iter()
            .find(|candidate| candidate.id == device_id)
            .cloned()
            .ok_or_else(|| {
                warn!(device_id, "状态中未找到目标设备");
                RairstreamError::InvalidConfiguration {
                    message: format!("device id {device_id} not found in state"),
                }
            })
    }
}

#[derive(Clone, Copy)]
enum DeviceMatcher {
    ExactId,
    ExactName,
    ExactHost,
    Contains,
}

impl DeviceMatcher {
    fn matches(self, device: &SpeakerDevice, filter: &str) -> bool {
        match self {
            Self::ExactId => device.id == filter,
            Self::ExactName => device.name == filter,
            Self::ExactHost => device.host == filter,
            Self::Contains => {
                let filter = filter.to_ascii_lowercase();
                device.id.to_ascii_lowercase().contains(&filter)
                    || device.name.to_ascii_lowercase().contains(&filter)
                    || device.host.to_ascii_lowercase().contains(&filter)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppController, SessionControlService};
    use crate::app::{
        AirPlayGeneration, AppState, DeviceSupport, RairstreamError, ReceiverKind, SessionState,
        SpeakerDevice,
    };
    use crate::config::{AppConfig, ReceiverAuthFlow, ReceiverCredentials};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Copy)]
    enum StartBehavior {
        Success,
        AwaitingPairing,
        Authenticating,
        AuthFailure,
    }

    #[derive(Debug, Clone)]
    struct StubSessionService {
        devices: Vec<SpeakerDevice>,
        start_behavior: Arc<Mutex<StartBehavior>>,
        stored_credentials: Arc<Mutex<Vec<String>>>,
        last_sender_volume_percent: Arc<Mutex<Option<u8>>>,
    }

    impl SessionControlService for StubSessionService {
        fn discover(&self) -> Vec<SpeakerDevice> {
            self.devices.clone()
        }

        fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
            Ok(AppState {
                selected_device_id: Some(device.id.clone()),
                active_session: SessionState::Connecting {
                    device_id: device.id,
                },
            })
        }

        fn pair_with_pin(
            &self,
            _device: SpeakerDevice,
            _pin: &str,
        ) -> Result<ReceiverCredentials, RairstreamError> {
            *self.start_behavior.lock().expect("lock start behavior") = StartBehavior::Success;
            Ok(ReceiverCredentials {
                auth_flow: ReceiverAuthFlow::Modern,
                controller_pairing_id: String::from("controller-id"),
                controller_ltpk_hex: String::from("11"),
                controller_ltsk_hex: String::from("22"),
                receiver_pairing_id: String::from("receiver-id"),
                receiver_ltpk_hex: String::from("33"),
            })
        }

        fn store_receiver_credentials(
            &self,
            device_id: String,
            _receiver_credentials: ReceiverCredentials,
        ) -> Result<(), RairstreamError> {
            self.stored_credentials
                .lock()
                .expect("lock stored credentials")
                .push(device_id);
            Ok(())
        }

        fn start_streaming_session(
            &self,
            device: SpeakerDevice,
        ) -> Result<AppState, RairstreamError> {
            match *self.start_behavior.lock().expect("lock start behavior") {
                StartBehavior::Success => Ok(AppState {
                    selected_device_id: Some(device.id.clone()),
                    active_session: SessionState::Streaming {
                        device_id: device.id,
                    },
                }),
                StartBehavior::AwaitingPairing => Ok(AppState {
                    selected_device_id: Some(device.id.clone()),
                    active_session: SessionState::AwaitingPairing {
                        device_id: device.id,
                    },
                }),
                StartBehavior::Authenticating => Ok(AppState {
                    selected_device_id: Some(device.id.clone()),
                    active_session: SessionState::Authenticating {
                        device_id: device.id,
                    },
                }),
                StartBehavior::AuthFailure => Err(RairstreamError::Transport(
                    crate::transport::AirPlayError::AuthenticationFailed {
                        message: String::from("stub auth failure"),
                    },
                )),
            }
        }

        fn stop_streaming_session(&self) -> Result<AppState, RairstreamError> {
            Ok(AppState {
                selected_device_id: None,
                active_session: SessionState::Idle,
            })
        }

        fn reconcile_app_state(
            &self,
            selected_device_id: Option<String>,
        ) -> Result<AppState, RairstreamError> {
            Ok(
                match *self.start_behavior.lock().expect("lock start behavior") {
                    StartBehavior::Success => AppState {
                        selected_device_id,
                        active_session: SessionState::Streaming {
                            device_id: String::from("living-room"),
                        },
                    },
                    StartBehavior::AwaitingPairing
                    | StartBehavior::Authenticating
                    | StartBehavior::AuthFailure => AppState {
                        selected_device_id,
                        active_session: SessionState::Idle,
                    },
                },
            )
        }

        fn set_sender_volume_percent(&self, percent: u8) -> Result<(), RairstreamError> {
            *self
                .last_sender_volume_percent
                .lock()
                .expect("lock sender volume") = Some(percent);
            Ok(())
        }
    }

    fn build_device(id: &str, name: &str, host: &str) -> SpeakerDevice {
        SpeakerDevice {
            id: id.to_string(),
            name: name.to_string(),
            host: host.to_string(),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ClassicRaop,
            support: DeviceSupport::Supported,
        }
    }

    #[test]
    fn refresh_devices_selects_preferred_device_when_available() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![
                    build_device("living-room", "Living Room", "192.168.1.10"),
                    build_device("kitchen", "Kitchen", "192.168.1.11"),
                ],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig {
                preferred_device_id: Some(String::from("kitchen")),
                ..AppConfig::default()
            },
        );

        controller.refresh_devices();

        assert_eq!(
            controller.app_state().selected_device_id.as_deref(),
            Some("kitchen")
        );
        assert_eq!(controller.devices().len(), 2);
        assert_eq!(controller.app_state().active_session, SessionState::Idle);
    }

    #[test]
    fn refresh_devices_keeps_streaming_when_runtime_session_is_active() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig {
                preferred_device_id: Some(String::from("living-room")),
                ..AppConfig::default()
            },
        );

        controller.refresh_devices();

        assert!(matches!(
            controller.app_state().active_session,
            SessionState::Streaming { .. }
        ));
        assert_eq!(
            controller.app_state().selected_device_id.as_deref(),
            Some("living-room")
        );
    }

    #[test]
    fn resolve_target_device_prefers_last_used_device() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![
                    build_device("living-room", "Living Room", "192.168.1.10"),
                    build_device("kitchen", "Kitchen", "192.168.1.11"),
                ],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig {
                preferred_device_id: Some(String::from("kitchen")),
                ..AppConfig::default()
            },
        );

        controller.refresh_devices();
        let device = controller
            .resolve_target_device(None)
            .expect("preferred device should be selected");

        assert_eq!(device.id, "kitchen");
    }

    #[test]
    fn resolve_target_device_reports_ambiguous_filter() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![
                    build_device("living-room", "Living Room", "192.168.1.10"),
                    build_device("living-room-2", "Living Room Mini", "192.168.1.11"),
                ],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        let error = controller
            .resolve_target_device(Some("living"))
            .expect_err("filter should be ambiguous");

        assert!(error.to_string().contains("多个目标"));
    }

    #[test]
    fn submit_pairing_pin_persists_credentials_and_retries_streaming() {
        let service = StubSessionService {
            devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
            start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
            stored_credentials: Arc::new(Mutex::new(Vec::new())),
            last_sender_volume_percent: Arc::new(Mutex::new(None)),
        };
        let stored_credentials = Arc::clone(&service.stored_credentials);
        let mut controller = AppController::new(service, AppConfig::default());

        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("initial select should enter pairing state");
        controller
            .submit_pairing_pin("receiver", "123456")
            .expect("pairing retry should succeed");

        assert!(matches!(
            controller.app_state().active_session,
            SessionState::Streaming { .. }
        ));
        assert_eq!(
            stored_credentials
                .lock()
                .expect("lock stored credentials")
                .as_slice(),
            ["receiver"]
        );
        assert!(
            controller
                .config()
                .paired_receivers
                .contains_key("receiver")
        );
    }

    #[test]
    fn handle_error_clears_invalid_paired_receiver() {
        let mut config = AppConfig::default();
        config.upsert_paired_receiver(
            "receiver",
            ReceiverCredentials {
                auth_flow: ReceiverAuthFlow::Modern,
                controller_pairing_id: String::from("controller-id"),
                controller_ltpk_hex: String::from("11"),
                controller_ltsk_hex: String::from("22"),
                receiver_pairing_id: String::from("receiver-id"),
                receiver_ltpk_hex: String::from("33"),
            },
        );
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            config,
        );
        controller.refresh_devices();

        controller.handle_error(
            Some("receiver"),
            &RairstreamError::Transport(crate::transport::AirPlayError::AuthenticationFailed {
                message: String::from("forbidden"),
            }),
        );

        assert!(
            !controller
                .config()
                .paired_receivers
                .contains_key("receiver")
        );
        assert_eq!(
            controller.last_error(),
            Some("Receiver 认证失败，请重新配对后再试")
        );
    }

    #[test]
    fn initialize_auto_reconnects_selected_device() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("kitchen", "Kitchen", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig {
                preferred_device_id: Some(String::from("kitchen")),
                auto_reconnect: true,
                ..AppConfig::default()
            },
        );

        controller.initialize();

        assert_eq!(
            controller.app_state().active_session,
            SessionState::AwaitingPairing {
                device_id: String::from("kitchen")
            }
        );
    }

    #[test]
    fn initialize_skips_auto_reconnect_when_runtime_is_already_streaming() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig {
                preferred_device_id: Some(String::from("living-room")),
                auto_reconnect: true,
                ..AppConfig::default()
            },
        );

        controller.initialize();

        assert!(matches!(
            controller.app_state().active_session,
            SessionState::Streaming { .. }
        ));
        assert_eq!(
            controller.app_state().selected_device_id.as_deref(),
            Some("living-room")
        );
    }

    #[test]
    fn stop_streaming_preserves_preferred_device() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );
        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("stream should start");

        controller.stop_streaming().expect("stop should succeed");

        assert_eq!(
            controller.config().preferred_device_id.as_deref(),
            Some("receiver")
        );
        assert_eq!(
            controller.app_state().selected_device_id.as_deref(),
            Some("receiver")
        );
    }

    #[test]
    fn select_device_applies_effective_muted_volume_before_streaming() {
        let service = StubSessionService {
            devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
            start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
            stored_credentials: Arc::new(Mutex::new(Vec::new())),
            last_sender_volume_percent: Arc::new(Mutex::new(None)),
        };
        let last_sender_volume_percent = Arc::clone(&service.last_sender_volume_percent);
        let mut controller = AppController::new(
            service,
            AppConfig {
                sender_volume_percent: 75,
                sender_muted: true,
                ..AppConfig::default()
            },
        );
        controller.refresh_devices();

        controller
            .select_device("receiver")
            .expect("stream should start");

        assert_eq!(
            *last_sender_volume_percent
                .lock()
                .expect("lock sender volume"),
            Some(0)
        );
    }

    #[test]
    fn set_sender_volume_preserves_boosted_value_when_not_streaming() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );

        controller
            .set_sender_volume(125)
            .expect("boosted volume should be accepted");

        assert_eq!(controller.sender_volume_percent(), 125);
        assert_eq!(controller.effective_sender_volume_percent(), 125);
    }

    #[test]
    fn set_sender_volume_pushes_boosted_value_to_runtime_when_streaming() {
        let service = StubSessionService {
            devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
            start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
            stored_credentials: Arc::new(Mutex::new(Vec::new())),
            last_sender_volume_percent: Arc::new(Mutex::new(None)),
        };
        let last_sender_volume_percent = Arc::clone(&service.last_sender_volume_percent);
        let mut controller = AppController::new(service, AppConfig::default());
        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("stream should start");

        controller
            .set_sender_volume(125)
            .expect("boosted volume should update runtime");

        assert_eq!(controller.sender_volume_percent(), 125);
        assert_eq!(
            *last_sender_volume_percent
                .lock()
                .expect("lock sender volume"),
            Some(125)
        );
    }

    #[test]
    fn describe_error_formats_credentials_missing() {
        let mut controller = AppController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver", "192.168.1.10")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Authenticating)),
                stored_credentials: Arc::new(Mutex::new(Vec::new())),
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );
        controller.refresh_devices();

        let message = controller.describe_error(
            Some("receiver"),
            &RairstreamError::Transport(crate::transport::AirPlayError::CredentialsMissing),
        );

        assert_eq!(message, "Receiver 缺少可用配对记录，请重新配对");
    }

    #[test]
    fn start_auth_failure_variant_is_constructible() {
        let _ = StartBehavior::AuthFailure;
    }
}
