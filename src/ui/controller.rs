use super::state::{TrayAppState, TrayMenuModel, build_tray_menu_model};
use crate::app::{AppState, RairstreamError, SessionCoordinator, SessionState, SpeakerDevice};
use crate::config::{AppConfig, ReceiverCredentials};
use crate::discovery::DiscoveryService;
use tracing::{debug, info, warn};

pub trait TraySessionService {
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
}

impl<D> TraySessionService for SessionCoordinator<D>
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
}

#[derive(Debug)]
pub struct TrayController<S> {
    session_service: S,
    config: AppConfig,
    state: TrayAppState,
}

impl<S> TrayController<S>
where
    S: TraySessionService,
{
    #[must_use]
    pub fn new(session_service: S, config: AppConfig) -> Self {
        let preferred_device_id = config.preferred_device_id.clone();
        Self {
            session_service,
            config,
            state: TrayAppState {
                app_state: AppState {
                    selected_device_id: preferred_device_id,
                    active_session: SessionState::Idle,
                },
                devices: Vec::new(),
                last_error: None,
            },
        }
    }

    #[must_use]
    pub fn menu_model(&self) -> TrayMenuModel {
        build_tray_menu_model(&self.state)
    }

    #[must_use]
    pub fn state(&self) -> &TrayAppState {
        &self.state
    }

    pub fn refresh_devices(&mut self) -> TrayMenuModel {
        debug!("托盘请求刷新设备列表");
        self.state.app_state.active_session = SessionState::Discovering;

        let devices = self.session_service.discover();
        let selected_device_id = self.retain_selected_device_id(&devices);

        self.state.devices = devices;
        self.state.app_state.selected_device_id = selected_device_id;
        self.state.app_state.active_session = SessionState::Idle;
        self.state.last_error = None;

        info!(
            device_count = self.state.devices.len(),
            "托盘设备列表刷新完成"
        );
        self.menu_model()
    }

    pub fn initialize(&mut self) -> TrayMenuModel {
        let model = self.refresh_devices();
        if !self.config.auto_reconnect {
            return model;
        }

        let Some(device_id) = self.state.app_state.selected_device_id.clone() else {
            return model;
        };

        match self.select_device(&device_id) {
            Ok(model) => model,
            Err(error) => {
                self.handle_error(Some(&device_id), &error);
                self.menu_model()
            }
        }
    }

    pub fn select_device(&mut self, device_id: &str) -> Result<TrayMenuModel, RairstreamError> {
        info!(device_id, "托盘请求选择设备");
        if matches!(
            &self.state.app_state.active_session,
            SessionState::Streaming {
                device_id: active_device_id,
            } if active_device_id == device_id
        ) {
            info!(device_id, "目标设备已在串流，转为停止串流");
            return self.stop_streaming();
        }

        let device = self.find_device(device_id)?;
        let app_state = self
            .session_service
            .start_streaming_session(device.clone())?;

        self.config.set_preferred_device_id(Some(device.id.clone()));
        self.config.save()?;
        self.state.app_state = app_state;
        self.state.last_error = None;

        Ok(self.menu_model())
    }

    pub fn submit_pairing_pin(
        &mut self,
        device_id: &str,
        pin: &str,
    ) -> Result<TrayMenuModel, RairstreamError> {
        info!(device_id, "托盘提交首次配对 PIN");
        let device = self.find_device(device_id)?;
        let receiver_credentials = self.session_service.pair_with_pin(device.clone(), pin)?;
        self.session_service
            .store_receiver_credentials(device.id.clone(), receiver_credentials.clone())?;
        self.config
            .upsert_paired_receiver(device.id.clone(), receiver_credentials);
        self.config.save()?;
        self.state.app_state.selected_device_id = Some(device.id.clone());
        self.state.app_state.active_session = SessionState::Authenticating {
            device_id: device.id.clone(),
        };
        let app_state = self.session_service.start_streaming_session(device)?;
        self.state.app_state = app_state;
        self.state.last_error = None;
        Ok(self.menu_model())
    }

    pub fn cancel_pairing(&mut self, device_id: &str) -> Result<TrayMenuModel, RairstreamError> {
        info!(device_id, "托盘取消首次配对输入");
        let device = self.find_device(device_id)?;
        self.state.app_state = AppState {
            selected_device_id: Some(device.id),
            active_session: SessionState::Idle,
        };
        self.state.last_error = None;
        Ok(self.menu_model())
    }

    pub fn stop_streaming(&mut self) -> Result<TrayMenuModel, RairstreamError> {
        info!("托盘请求停止串流");
        let mut app_state = self.session_service.stop_streaming_session()?;
        app_state
            .selected_device_id
            .clone_from(&self.state.app_state.selected_device_id);
        self.config
            .set_preferred_device_id(app_state.selected_device_id.clone());
        self.config.save()?;
        self.state.app_state = app_state;
        self.state.last_error = None;
        Ok(self.menu_model())
    }

    fn retain_selected_device_id(&self, devices: &[SpeakerDevice]) -> Option<String> {
        self.state
            .app_state
            .selected_device_id
            .as_ref()
            .filter(|selected_device_id| {
                devices
                    .iter()
                    .any(|device| device.id.as_str() == selected_device_id.as_str())
            })
            .cloned()
    }

    pub fn handle_error(&mut self, device_id: Option<&str>, error: &RairstreamError) {
        self.state.last_error = Some(self.describe_error(device_id, error));
    }

    fn describe_error(&self, device_id: Option<&str>, error: &RairstreamError) -> String {
        let device_name = device_id.and_then(|device_id| {
            self.state
                .devices
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

    fn find_device(&self, device_id: &str) -> Result<SpeakerDevice, RairstreamError> {
        self.state
            .devices
            .iter()
            .find(|candidate| candidate.id == device_id)
            .cloned()
            .ok_or_else(|| {
                warn!(device_id, "托盘状态中未找到目标设备");
                RairstreamError::InvalidConfiguration {
                    message: format!("device id {device_id} not found in tray state"),
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{TrayController, TraySessionService};
    use crate::app::{
        AirPlayGeneration, AppState, DeviceSupport, RairstreamError, ReceiverKind,
        SessionCoordinator, SessionState, SpeakerDevice,
    };
    use crate::config::{AppConfig, ReceiverAuthFlow, ReceiverCredentials};
    use crate::discovery::StubDiscoveryService;

    #[derive(Debug, Clone, Copy)]
    enum StartBehavior {
        Success,
        AwaitingPairing,
        Authenticating,
        Failure,
    }

    #[derive(Debug, Clone, Copy)]
    enum PairBehavior {
        Success,
        Failure,
    }

    #[derive(Debug, Clone)]
    struct StubSessionService {
        devices: Vec<SpeakerDevice>,
        start_behavior: StartBehavior,
        pair_behavior: PairBehavior,
    }

    impl TraySessionService for StubSessionService {
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
            match self.pair_behavior {
                PairBehavior::Success => Ok(ReceiverCredentials {
                    auth_flow: ReceiverAuthFlow::Modern,
                    controller_pairing_id: String::from("controller-id"),
                    controller_ltpk_hex: String::from("11"),
                    controller_ltsk_hex: String::from("22"),
                    receiver_pairing_id: String::from("receiver-id"),
                    receiver_ltpk_hex: String::from("33"),
                }),
                PairBehavior::Failure => Err(RairstreamError::Transport(
                    crate::transport::AirPlayError::AuthenticationFailed {
                        message: String::from("stub pair failure"),
                    },
                )),
            }
        }

        fn store_receiver_credentials(
            &self,
            _device_id: String,
            _receiver_credentials: ReceiverCredentials,
        ) -> Result<(), RairstreamError> {
            Ok(())
        }

        fn start_streaming_session(
            &self,
            device: SpeakerDevice,
        ) -> Result<AppState, RairstreamError> {
            match self.start_behavior {
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
                StartBehavior::Failure => Err(RairstreamError::InvalidConfiguration {
                    message: String::from("stub start failure"),
                }),
            }
        }

        fn stop_streaming_session(&self) -> Result<AppState, RairstreamError> {
            Ok(AppState {
                selected_device_id: None,
                active_session: SessionState::Idle,
            })
        }
    }

    fn build_device(id: &str, name: &str) -> SpeakerDevice {
        SpeakerDevice {
            id: id.to_string(),
            name: name.to_string(),
            host: String::from("192.168.1.20"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ClassicRaop,
            support: DeviceSupport::Supported,
        }
    }

    #[test]
    fn test_refresh_devices_populates_menu_model() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        let model = controller.refresh_devices();

        assert_eq!(controller.state().devices.len(), 1);
        assert_eq!(model.device_items.len(), 1);
        assert_eq!(model.device_items[0].device_id, "living-room");
    }

    #[test]
    fn test_initialize_auto_reconnects_preferred_device() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig {
                auto_reconnect: true,
                preferred_device_id: Some(String::from("living-room")),
                ..AppConfig::default()
            },
        );

        let model = controller.initialize();

        assert_eq!(model.status_label, "Rairstream：正在串流 Living Room");
        assert!(matches!(
            controller.state().app_state.active_session,
            SessionState::Streaming { .. }
        ));
    }

    #[test]
    fn test_initialize_skips_auto_reconnect_when_disabled() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig {
                auto_reconnect: false,
                preferred_device_id: Some(String::from("living-room")),
                ..AppConfig::default()
            },
        );

        let model = controller.initialize();

        assert_eq!(model.status_label, "Rairstream：已选择 Living Room");
        assert_eq!(
            controller.state().app_state.active_session,
            SessionState::Idle
        );
    }

    #[test]
    fn test_refresh_devices_preserves_existing_selection() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("kitchen", "Kitchen")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig {
                preferred_device_id: Some(String::from("kitchen")),
                ..AppConfig::default()
            },
        );

        controller.refresh_devices();

        assert_eq!(
            controller.state().app_state.selected_device_id.as_deref(),
            Some("kitchen")
        );
    }

    #[test]
    fn test_refresh_devices_clears_missing_selection() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("office", "Office")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig {
                preferred_device_id: Some(String::from("missing-device")),
                ..AppConfig::default()
            },
        );

        controller.refresh_devices();

        assert!(controller.state().app_state.selected_device_id.is_none());
    }

    #[test]
    fn test_select_device_enters_streaming_state() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("bedroom", "Bedroom")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        let model = controller
            .select_device("bedroom")
            .expect("stub selection should succeed");

        assert_eq!(model.status_label, "Rairstream：正在串流 Bedroom");
        assert_eq!(
            controller.state().app_state.selected_device_id.as_deref(),
            Some("bedroom")
        );
        assert_eq!(
            controller.config.preferred_device_id.as_deref(),
            Some("bedroom")
        );
        assert!(matches!(
            controller.state().app_state.active_session,
            SessionState::Streaming { .. }
        ));
    }

    #[test]
    fn test_select_device_keeps_pairing_related_state() {
        let mut pairing_controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("bedroom", "Bedroom")],
                start_behavior: StartBehavior::AwaitingPairing,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );
        pairing_controller.refresh_devices();
        let pairing_model = pairing_controller
            .select_device("bedroom")
            .expect("stub pairing selection should succeed");
        assert_eq!(pairing_model.status_label, "Rairstream：等待配对 Bedroom");
        assert!(matches!(
            pairing_controller.state().app_state.active_session,
            SessionState::AwaitingPairing { .. }
        ));

        let mut auth_controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Authenticating,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );
        auth_controller.refresh_devices();
        let auth_model = auth_controller
            .select_device("den")
            .expect("stub auth selection should succeed");
        assert_eq!(auth_model.status_label, "Rairstream：正在认证 Den");
        assert!(matches!(
            auth_controller.state().app_state.active_session,
            SessionState::Authenticating { .. }
        ));
    }

    #[test]
    fn test_select_device_rejects_missing_device() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("studio", "Studio")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        let result = controller.select_device("missing-device");

        assert!(matches!(
            result,
            Err(RairstreamError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn test_select_device_preserves_state_on_start_failure() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Failure,
                pair_behavior: PairBehavior::Failure,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        let result = controller.select_device("den");

        assert!(result.is_err());
        assert!(controller.state().app_state.selected_device_id.is_none());
        assert_eq!(
            controller.state().app_state.active_session,
            SessionState::Idle
        );
    }

    #[test]
    fn test_submit_pairing_pin_retries_same_device_and_enters_streaming() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller.state.app_state = AppState {
            selected_device_id: Some(String::from("den")),
            active_session: SessionState::AwaitingPairing {
                device_id: String::from("den"),
            },
        };

        let model = controller.submit_pairing_pin("den", "1234").unwrap();

        assert_eq!(model.status_label, "Rairstream：正在串流 Den");
        assert!(matches!(
            controller.state().app_state.active_session,
            SessionState::Streaming { .. }
        ));
    }

    #[test]
    fn test_cancel_pairing_returns_idle_for_same_device() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::AwaitingPairing,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller.state.app_state = AppState {
            selected_device_id: Some(String::from("den")),
            active_session: SessionState::AwaitingPairing {
                device_id: String::from("den"),
            },
        };

        let model = controller.cancel_pairing("den").unwrap();

        assert_eq!(model.status_label, "Rairstream：已选择 Den");
        assert_eq!(
            controller.state().app_state.active_session,
            SessionState::Idle
        );
    }

    #[test]
    fn test_select_streaming_device_stops_session() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller.select_device("den").unwrap();
        let model = controller.stop_streaming().unwrap();

        assert_eq!(model.status_label, "Rairstream：已选择 Den");
        assert_eq!(
            controller.config.preferred_device_id.as_deref(),
            Some("den")
        );
        assert_eq!(
            controller.state().app_state.active_session,
            SessionState::Idle
        );
    }

    #[test]
    fn test_handle_error_maps_authentication_recovery_message() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller.handle_error(
            Some("den"),
            &RairstreamError::Transport(crate::transport::AirPlayError::AuthenticationFailed {
                message: String::from("forbidden"),
            }),
        );

        assert_eq!(
            controller.state().last_error.as_deref(),
            Some("Den 认证失败，请重新配对后再试")
        );
        assert_eq!(
            controller.menu_model().status_label,
            "Rairstream：Den 认证失败，请重新配对后再试"
        );
    }

    #[test]
    fn test_handle_error_maps_missing_credentials_recovery_message() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                start_behavior: StartBehavior::Success,
                pair_behavior: PairBehavior::Success,
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller.handle_error(
            Some("den"),
            &RairstreamError::Transport(crate::transport::AirPlayError::CredentialsMissing),
        );

        assert_eq!(
            controller.state().last_error.as_deref(),
            Some("Den 缺少可用配对记录，请重新配对")
        );
    }

    #[test]
    fn test_controller_reuses_session_coordinator_runtime_constraints() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let mut controller = TrayController::new(coordinator, AppConfig::default());

        controller.refresh_devices();
        let result = controller.select_device("stub-speaker");

        assert!(result.is_err());
    }
}
