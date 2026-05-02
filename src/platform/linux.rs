pub const OS_NAME: &str = "linux";

#[must_use]
pub const fn supports_system_audio_capture() -> bool {
    false
}
