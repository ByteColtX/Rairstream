#[cfg(target_os = "linux")]
pub mod linux;
pub mod unsupported;
#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub supports_system_audio_capture: bool,
}

#[must_use]
pub fn current_platform() -> PlatformInfo {
    #[cfg(target_os = "windows")]
    {
        return PlatformInfo {
            os: windows::OS_NAME,
            supports_system_audio_capture: windows::supports_system_audio_capture(),
        };
    }

    #[cfg(target_os = "linux")]
    {
        return PlatformInfo {
            os: linux::OS_NAME,
            supports_system_audio_capture: linux::supports_system_audio_capture(),
        };
    }

    #[allow(unreachable_code)]
    PlatformInfo {
        os: unsupported::OS_NAME,
        supports_system_audio_capture: unsupported::supports_system_audio_capture(),
    }
}
