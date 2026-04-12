use rairstream::app::{RairstreamError, SessionCoordinator, SessionState};
use rairstream::config::AppConfig;
use rairstream::discovery::StubDiscoveryService;

#[test]
fn test_default_config_starts_with_auto_reconnect_enabled() {
    let config = AppConfig::default();

    assert!(config.auto_reconnect);
    assert!(config.preferred_device_id.is_none());
}

#[test]
fn test_session_coordinator_with_stub_discovery_exposes_stub_device() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let devices = coordinator.discover();

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "stub-speaker");
    assert_eq!(devices[0].name, "Stub Speaker");
    assert_eq!(devices[0].host, "127.0.0.1");
    assert_eq!(devices[0].port, 7000);
}

#[test]
fn test_prepare_session_returns_platform_or_runtime_appropriate_result() {
    let coordinator = SessionCoordinator::new(StubDiscoveryService);
    let device = coordinator.discover().remove(0);
    let result = coordinator.prepare_session(device.clone());

    #[cfg(target_os = "windows")]
    match result {
        Ok(state) => {
            assert_eq!(
                state.selected_device_id.as_deref(),
                Some(device.id.as_str())
            );
            assert_eq!(
                state.active_session,
                SessionState::Connecting {
                    device_id: device.id,
                }
            );
        }
        Err(RairstreamError::AudioCapture(_)) => {}
        Err(error) => panic!("unexpected prepare_session error: {error}"),
    }

    #[cfg(not(target_os = "windows"))]
    match result {
        Err(RairstreamError::NotImplemented { feature }) => {
            assert_eq!(feature, "non-Windows runtime support");
        }
        Err(error) => panic!("unexpected prepare_session error: {error}"),
        Ok(state) => panic!("unexpected prepare_session success: {state:?}"),
    }
}
