use rairstream::app::platform::{
    current_platform, ensure_supported_runtime, is_launch_at_startup_enabled,
    set_launch_at_startup_enabled,
};

#[test]
fn test_current_platform_reports_expected_contract() {
    let platform = current_platform();

    assert_eq!(platform.os, std::env::consts::OS);

    #[cfg(target_os = "windows")]
    assert!(platform.supports_system_audio_capture);

    #[cfg(not(target_os = "windows"))]
    assert!(!platform.supports_system_audio_capture);
}

#[test]
fn test_ensure_supported_runtime_matches_target_platform() {
    let result = ensure_supported_runtime();

    #[cfg(target_os = "windows")]
    result.expect("windows target should be supported");

    #[cfg(not(target_os = "windows"))]
    match result {
        Err(rairstream::app::RairstreamError::NotImplemented { feature }) => {
            assert_eq!(feature, "non-Windows runtime support");
        }
        Err(error) => panic!("unexpected runtime error: {error}"),
        Ok(()) => panic!("unexpected runtime support on non-Windows target"),
    }
}

#[test]
fn test_launch_at_startup_platform_contract_matches_target_platform() {
    #[cfg(target_os = "windows")]
    {
        is_launch_at_startup_enabled().expect("windows target should expose startup state");
    }

    #[cfg(not(target_os = "windows"))]
    {
        match is_launch_at_startup_enabled() {
            Err(rairstream::app::RairstreamError::NotImplemented { feature }) => {
                assert_eq!(feature, "launch at startup support");
            }
            Err(error) => panic!("unexpected startup state error: {error}"),
            Ok(enabled) => panic!("unexpected startup state on non-Windows target: {enabled}"),
        }

        match set_launch_at_startup_enabled(true) {
            Err(rairstream::app::RairstreamError::NotImplemented { feature }) => {
                assert_eq!(feature, "launch at startup support");
            }
            Err(error) => panic!("unexpected startup toggle error: {error}"),
            Ok(()) => panic!("unexpected startup toggle success on non-Windows target"),
        }
    }
}
