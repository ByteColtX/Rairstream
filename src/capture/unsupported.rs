use crate::audio::AudioCaptureError;

pub fn capture_unsupported() -> AudioCaptureError {
    AudioCaptureError::UnsupportedPlatform
}
