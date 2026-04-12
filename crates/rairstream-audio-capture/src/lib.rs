//! 音频采集层：对上层暴露统一的输入抽象。

use rairstream_core::{CaptureBackend, RairstreamError};

/// 音频流基础格式信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate_hz: 44_100,
            channels: 2,
            bits_per_sample: 16,
        }
    }
}

/// 首版本预留的 Windows 回环采集后端。
#[derive(Debug, Default)]
pub struct WindowsLoopbackCapture;

impl CaptureBackend for WindowsLoopbackCapture {
    fn backend_name(&self) -> &'static str {
        "windows-wasapi-loopback"
    }
}

impl WindowsLoopbackCapture {
    pub fn preferred_format(&self) -> AudioFormat {
        AudioFormat::default()
    }

    pub fn start(&self) -> Result<(), RairstreamError> {
        Err(RairstreamError::NotImplemented {
            feature: "WASAPI loopback capture",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioFormat, WindowsLoopbackCapture};
    use rairstream_core::CaptureBackend;

    #[test]
    fn default_audio_format_matches_cd_quality() {
        assert_eq!(AudioFormat::default().sample_rate_hz, 44_100);
    }

    #[test]
    fn windows_backend_reports_name() {
        let backend = WindowsLoopbackCapture;

        assert_eq!(backend.backend_name(), "windows-wasapi-loopback");
    }
}
