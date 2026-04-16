use super::state::{TrayAppState, TrayMenuModel, build_tray_menu_model};
use crate::app::{AppController, RairstreamError, SessionControlService};
use crate::config::AppConfig;

#[derive(Debug)]
pub struct TrayController<S> {
    controller: AppController<S>,
}

impl<S> TrayController<S>
where
    S: SessionControlService,
{
    #[must_use]
    pub fn new(session_service: S, config: AppConfig) -> Self {
        Self {
            controller: AppController::new(session_service, config),
        }
    }

    #[must_use]
    pub fn menu_model(&self) -> TrayMenuModel {
        build_tray_menu_model(&self.state())
    }

    #[must_use]
    pub fn state(&self) -> TrayAppState {
        TrayAppState {
            app_state: self.controller.app_state().clone(),
            devices: self.controller.devices().to_vec(),
            auto_reconnect: self.controller.auto_reconnect(),
            launch_at_startup: self.controller.launch_at_startup(),
            sender_volume_percent: self.controller.sender_volume_percent(),
            sender_muted: self.controller.sender_muted(),
            last_error: self.controller.last_error().map(str::to_string),
        }
    }

    pub fn refresh_devices(&mut self) -> TrayMenuModel {
        self.controller.refresh_devices();
        self.menu_model()
    }

    pub fn initialize(&mut self) -> TrayMenuModel {
        self.controller.initialize();
        self.menu_model()
    }

    pub fn sync_runtime_state(&mut self) -> TrayMenuModel {
        self.controller.refresh_devices();
        self.menu_model()
    }

    pub fn select_device(&mut self, device_id: &str) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.select_device(device_id)?;
        Ok(self.menu_model())
    }

    pub fn submit_pairing_pin(
        &mut self,
        device_id: &str,
        pin: &str,
    ) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.submit_pairing_pin(device_id, pin)?;
        Ok(self.menu_model())
    }

    pub fn cancel_pairing(&mut self, device_id: &str) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.cancel_pairing(device_id)?;
        Ok(self.menu_model())
    }

    pub fn stop_streaming(&mut self) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.stop_streaming()?;
        Ok(self.menu_model())
    }

    pub fn toggle_auto_reconnect(&mut self) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.toggle_auto_reconnect()?;
        Ok(self.menu_model())
    }

    pub fn toggle_launch_at_startup(&mut self) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.toggle_launch_at_startup()?;
        Ok(self.menu_model())
    }

    pub fn set_sender_volume(&mut self, percent: u8) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.set_sender_volume(percent)?;
        Ok(self.menu_model())
    }

    pub fn toggle_sender_mute(&mut self) -> Result<TrayMenuModel, RairstreamError> {
        self.controller.toggle_sender_mute()?;
        Ok(self.menu_model())
    }

    pub fn handle_error(&mut self, device_id: Option<&str>, error: &RairstreamError) {
        self.controller.handle_error(device_id, error);
    }
}

#[cfg(test)]
mod tests {
    use super::TrayController;
    use crate::app::{
        AirPlayGeneration, AppState, DeviceSupport, RairstreamError, ReceiverKind,
        SessionControlService, SessionCoordinator, SessionState, SpeakerDevice,
    };
    use crate::config::{AppConfig, ReceiverAuthFlow, ReceiverCredentials};
    use crate::discovery::StubDiscoveryService;
    use std::sync::{Arc, Mutex};

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
        start_behavior: Arc<Mutex<StartBehavior>>,
        pair_behavior: PairBehavior,
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
            match self.pair_behavior {
                PairBehavior::Success => {
                    *self.start_behavior.lock().unwrap() = StartBehavior::Success;
                    Ok(ReceiverCredentials {
                        auth_flow: ReceiverAuthFlow::Modern,
                        controller_pairing_id: String::from("controller-id"),
                        controller_ltpk_hex: String::from("11"),
                        controller_ltsk_hex: String::from("22"),
                        receiver_pairing_id: String::from("receiver-id"),
                        receiver_ltpk_hex: String::from("33"),
                    })
                }
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
            match *self.start_behavior.lock().unwrap() {
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

        fn reconcile_app_state(
            &self,
            selected_device_id: Option<String>,
        ) -> Result<AppState, RairstreamError> {
            Ok(match *self.start_behavior.lock().unwrap() {
                StartBehavior::Success => AppState {
                    selected_device_id,
                    active_session: SessionState::Streaming {
                        device_id: String::from("living-room"),
                    },
                },
                StartBehavior::AwaitingPairing
                | StartBehavior::Authenticating
                | StartBehavior::Failure => AppState {
                    selected_device_id,
                    active_session: SessionState::Idle,
                },
            })
        }

        fn set_sender_volume_percent(&self, percent: u8) -> Result<(), RairstreamError> {
            let mut last_sender_volume_percent = self.last_sender_volume_percent.lock().unwrap();
            *last_sender_volume_percent = Some(percent);
            Ok(())
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
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );

        let model = controller.refresh_devices();

        assert_eq!(controller.state().devices.len(), 1);
        assert_eq!(model.device_items.len(), 1);
        assert_eq!(model.device_items[0].device_id, "living-room");
        assert_eq!(model.status_label, "Rairstream：正在串流 Living Room");
    }

    #[test]
    fn test_initialize_skips_auto_reconnect_when_runtime_already_streaming() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
    fn test_initialize_auto_reconnects_preferred_device() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
    fn test_select_device_enters_streaming_state() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("bedroom", "Bedroom")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
                start_behavior: Arc::new(Mutex::new(StartBehavior::Authenticating)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
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
    fn test_submit_pairing_pin_transitions_to_streaming() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        controller
            .submit_pairing_pin("receiver", "123456")
            .expect("pairing should succeed");

        assert!(matches!(
            controller.state().app_state.active_session,
            SessionState::Streaming { .. }
        ));
    }

    #[test]
    fn test_submit_pairing_pin_propagates_failure() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::AwaitingPairing)),
                pair_behavior: PairBehavior::Failure,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );

        controller.refresh_devices();
        let result = controller.submit_pairing_pin("receiver", "123456");

        assert!(matches!(
            result,
            Err(RairstreamError::Transport(
                crate::transport::AirPlayError::AuthenticationFailed { .. }
            ))
        ));
    }

    #[test]
    fn test_stop_streaming_returns_idle_model() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );
        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("streaming should start");

        let model = controller.stop_streaming().expect("stop should succeed");

        assert_eq!(model.status_label, "Rairstream：已选择 Receiver");
    }

    #[test]
    fn test_set_sender_volume_updates_runtime_when_streaming() {
        let runtime_volume = Arc::new(Mutex::new(None));
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::clone(&runtime_volume),
            },
            AppConfig::default(),
        );
        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("streaming should start");

        controller
            .set_sender_volume(50)
            .expect("volume update should succeed");

        assert_eq!(*runtime_volume.lock().unwrap(), Some(50));
    }

    #[test]
    fn test_toggle_sender_mute_pushes_zero_runtime_volume() {
        let runtime_volume = Arc::new(Mutex::new(None));
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Success)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::clone(&runtime_volume),
            },
            AppConfig {
                sender_volume_percent: 75,
                ..AppConfig::default()
            },
        );
        controller.refresh_devices();
        controller
            .select_device("receiver")
            .expect("streaming should start");

        controller
            .toggle_sender_mute()
            .expect("mute toggle should succeed");

        assert_eq!(*runtime_volume.lock().unwrap(), Some(0));
    }

    #[test]
    fn test_handle_error_surfaces_pairing_message() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("receiver", "Receiver")],
                start_behavior: Arc::new(Mutex::new(StartBehavior::Failure)),
                pair_behavior: PairBehavior::Success,
                last_sender_volume_percent: Arc::new(Mutex::new(None)),
            },
            AppConfig::default(),
        );
        controller.refresh_devices();

        controller.handle_error(
            Some("receiver"),
            &RairstreamError::Transport(crate::transport::AirPlayError::PairingRequired),
        );

        assert_eq!(
            controller.state().last_error.as_deref(),
            Some("Receiver 需要先完成首次配对，请重新选择并输入 PIN")
        );
    }

    #[test]
    fn session_coordinator_still_implements_session_control_service() {
        fn assert_impl<T: SessionControlService>(_value: &T) {}

        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        assert_impl(&coordinator);
    }
}
