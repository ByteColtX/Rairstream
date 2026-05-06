use crate::error::RairstreamError;

pub fn run() -> Result<(), String> {
    Err(RairstreamError::UnsupportedPlatform {
        os: crate::platform::current_platform().os,
        feature: "system tray",
    }
    .to_string())
}
