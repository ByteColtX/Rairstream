use rairstream::app::{RairstreamError, SessionCoordinator, SessionState};
use rairstream::discovery::StubDiscoveryService;
use rairstream::ui::TrayController;

#[test]
fn test_refresh_with_real_session_coordinator_populates_menu_from_discovery() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let mut controller = TrayController::new(coordinator, None);

    let model = controller.refresh_devices();

    assert_eq!(controller.state().devices.len(), 1);
    assert_eq!(model.status_label, "Rairstream：空闲");
    assert_eq!(model.device_items.len(), 1);
    assert_eq!(model.device_items[0].device_id, "stub-speaker");
    assert_eq!(model.device_items[0].label, "Stub Speaker");
    assert!(model.device_items[0].enabled);
    assert!(!model.device_items[0].selected);
}

#[test]
fn test_refresh_preserves_preferred_device_when_device_still_exists() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let mut controller = TrayController::new(coordinator, Some(String::from("stub-speaker")));

    let model = controller.refresh_devices();

    assert_eq!(
        controller.state().app_state.selected_device_id.as_deref(),
        Some("stub-speaker")
    );
    assert_eq!(model.status_label, "Rairstream：已选择 Stub Speaker");
    assert!(model.device_items[0].selected);
    assert_eq!(model.device_items[0].label, "● Stub Speaker");
}

#[test]
fn test_select_device_transitions_or_returns_runtime_error() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let mut controller = TrayController::new(coordinator, None);

    controller.refresh_devices();
    let result = controller.select_device("stub-speaker");

    #[cfg(target_os = "windows")]
    match result {
        Ok(model) => {
            assert_eq!(model.status_label, "Rairstream：正在串流 Stub Speaker");
            assert_eq!(
                controller.state().app_state.selected_device_id.as_deref(),
                Some("stub-speaker")
            );
            assert!(matches!(
                controller.state().app_state.active_session,
                SessionState::Streaming { .. }
            ));
        }
        Err(RairstreamError::AudioCapture(_)) | Err(RairstreamError::Transport(_)) => {
            assert!(controller.state().app_state.selected_device_id.is_none());
            assert_eq!(
                controller.state().app_state.active_session,
                SessionState::Idle
            );
        }
        Err(error) => panic!("unexpected selection error: {error}"),
    }

    #[cfg(not(target_os = "windows"))]
    match result {
        Err(RairstreamError::NotImplemented { feature }) => {
            assert_eq!(feature, "non-Windows runtime support");
            assert!(controller.state().app_state.selected_device_id.is_none());
            assert_eq!(
                controller.state().app_state.active_session,
                SessionState::Idle
            );
        }
        Err(error) => panic!("unexpected selection error: {error}"),
        Ok(model) => panic!("unexpected selection success: {model:?}"),
    }
}

#[test]
fn test_selecting_unknown_device_returns_configuration_error() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let mut controller = TrayController::new(coordinator, None);

    controller.refresh_devices();
    let result = controller.select_device("missing-device");

    assert!(matches!(
        result,
        Err(RairstreamError::InvalidConfiguration { .. })
    ));
    assert!(controller.state().app_state.selected_device_id.is_none());
    assert_eq!(
        controller.state().app_state.active_session,
        SessionState::Idle
    );
}
