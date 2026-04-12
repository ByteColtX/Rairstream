use super::state::{TrayAppState, TrayMenuModel, build_tray_menu_model};
use crate::app::{AppState, RairstreamError, SessionCoordinator, SessionState, SpeakerDevice};
use crate::discovery::DiscoveryService;

pub trait TraySessionService {
    fn discover(&self) -> Vec<SpeakerDevice>;
    fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError>;
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
}

#[derive(Debug)]
pub struct TrayController<S> {
    session_service: S,
    state: TrayAppState,
}

impl<S> TrayController<S>
where
    S: TraySessionService,
{
    #[must_use]
    pub fn new(session_service: S, preferred_device_id: Option<String>) -> Self {
        Self {
            session_service,
            state: TrayAppState {
                app_state: AppState {
                    selected_device_id: preferred_device_id,
                    active_session: SessionState::Idle,
                },
                devices: Vec::new(),
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
        self.state.app_state.active_session = SessionState::Discovering;

        let devices = self.session_service.discover();
        let selected_device_id = self.retain_selected_device_id(&devices);

        self.state.devices = devices;
        self.state.app_state.selected_device_id = selected_device_id;
        self.state.app_state.active_session = SessionState::Idle;

        self.menu_model()
    }

    pub fn select_device(&mut self, device_id: &str) -> Result<TrayMenuModel, RairstreamError> {
        let device = self
            .state
            .devices
            .iter()
            .find(|candidate| candidate.id == device_id)
            .cloned()
            .ok_or_else(|| RairstreamError::InvalidConfiguration {
                message: format!("device id {device_id} not found in tray state"),
            })?;
        let app_state = self.session_service.prepare_session(device)?;

        self.state.app_state = app_state;

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
}

#[cfg(test)]
mod tests {
    use super::{TrayController, TraySessionService};
    use crate::app::{
        AirPlayGeneration, AppState, RairstreamError, SessionCoordinator, SessionState,
        SpeakerDevice,
    };
    use crate::discovery::StubDiscoveryService;

    #[derive(Debug, Clone, Copy)]
    enum PrepareBehavior {
        Success,
        Failure,
    }

    #[derive(Debug, Clone)]
    struct StubSessionService {
        devices: Vec<SpeakerDevice>,
        prepare_behavior: PrepareBehavior,
    }

    impl TraySessionService for StubSessionService {
        fn discover(&self) -> Vec<SpeakerDevice> {
            self.devices.clone()
        }

        fn prepare_session(&self, device: SpeakerDevice) -> Result<AppState, RairstreamError> {
            match self.prepare_behavior {
                PrepareBehavior::Success => Ok(AppState {
                    selected_device_id: Some(device.id.clone()),
                    active_session: SessionState::Connecting {
                        device_id: device.id,
                    },
                }),
                PrepareBehavior::Failure => Err(RairstreamError::InvalidConfiguration {
                    message: String::from("stub prepare failure"),
                }),
            }
        }
    }

    fn build_device(id: &str, name: &str) -> SpeakerDevice {
        SpeakerDevice {
            id: id.to_string(),
            name: name.to_string(),
            host: String::from("192.168.1.20"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
        }
    }

    #[test]
    fn test_refresh_devices_populates_menu_model() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("living-room", "Living Room")],
                prepare_behavior: PrepareBehavior::Success,
            },
            None,
        );

        let model = controller.refresh_devices();

        assert_eq!(controller.state().devices.len(), 1);
        assert_eq!(model.device_items.len(), 1);
        assert_eq!(model.device_items[0].device_id, "living-room");
    }

    #[test]
    fn test_refresh_devices_preserves_existing_selection() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("kitchen", "Kitchen")],
                prepare_behavior: PrepareBehavior::Success,
            },
            Some(String::from("kitchen")),
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
                prepare_behavior: PrepareBehavior::Success,
            },
            Some(String::from("missing-device")),
        );

        controller.refresh_devices();

        assert!(controller.state().app_state.selected_device_id.is_none());
    }

    #[test]
    fn test_select_device_enters_connecting_state() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("bedroom", "Bedroom")],
                prepare_behavior: PrepareBehavior::Success,
            },
            None,
        );

        controller.refresh_devices();
        let model = controller
            .select_device("bedroom")
            .expect("stub selection should succeed");

        assert_eq!(model.status_label, "Rairstream：正在连接 Bedroom");
        assert_eq!(
            controller.state().app_state.selected_device_id.as_deref(),
            Some("bedroom")
        );
        assert!(matches!(
            controller.state().app_state.active_session,
            SessionState::Connecting { .. }
        ));
    }

    #[test]
    fn test_select_device_rejects_missing_device() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("studio", "Studio")],
                prepare_behavior: PrepareBehavior::Success,
            },
            None,
        );

        controller.refresh_devices();
        let result = controller.select_device("missing-device");

        assert!(matches!(
            result,
            Err(RairstreamError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn test_select_device_preserves_state_on_prepare_failure() {
        let mut controller = TrayController::new(
            StubSessionService {
                devices: vec![build_device("den", "Den")],
                prepare_behavior: PrepareBehavior::Failure,
            },
            None,
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
    fn test_controller_reuses_session_coordinator_runtime_constraints() {
        let coordinator = SessionCoordinator::new(StubDiscoveryService);
        let mut controller = TrayController::new(coordinator, None);

        controller.refresh_devices();
        let result = controller.select_device("stub-speaker");

        if cfg!(target_os = "windows") {
            let model = result.expect("windows should allow session preparation");
            assert_eq!(model.status_label, "Rairstream：正在连接 Stub Speaker");
        } else {
            assert!(result.is_err());
        }
    }
}
