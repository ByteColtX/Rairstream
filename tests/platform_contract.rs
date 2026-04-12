use rairstream::app::platform::{current_platform, ensure_supported_runtime};

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
