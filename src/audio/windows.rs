use super::{AudioCaptureError, AudioChunk, AudioFormat, AudioSampleType, CaptureDriver};
use wasapi::{AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, StreamMode};

pub(super) fn preferred_format() -> Result<AudioFormat, AudioCaptureError> {
    let _com = ComGuard::mta()?;
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(&Direction::Render)?;
    let audio_client = device.get_iaudioclient()?;
    let wave_format = audio_client.get_mixformat()?;

    map_wave_format(&wave_format)
}

pub(super) fn build_default_driver() -> Result<WindowsCaptureDriver, AudioCaptureError> {
    WindowsCaptureDriver::new()
}

struct ComGuard;

impl ComGuard {
    fn mta() -> Result<Self, AudioCaptureError> {
        wasapi::initialize_mta().ok().map_err(|error| {
            AudioCaptureError::RuntimeInitialization {
                message: error.to_string(),
            }
        })?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}

pub(super) struct WindowsCaptureDriver {
    _com: ComGuard,
    audio_client: AudioClient,
    capture_client: AudioCaptureClient,
    event_handle: Handle,
    format: AudioFormat,
    bytes_per_frame: usize,
}

impl WindowsCaptureDriver {
    fn new() -> Result<Self, AudioCaptureError> {
        let com = ComGuard::mta()?;
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut audio_client = device.get_iaudioclient()?;
        let wave_format = audio_client.get_mixformat()?;
        let format = map_wave_format(&wave_format)?;
        let (default_period, _) = audio_client.get_device_period()?;
        let stream_mode = StreamMode::EventsShared {
            autoconvert: false,
            buffer_duration_hns: default_period,
        };

        audio_client.initialize_client(&wave_format, &Direction::Capture, &stream_mode)?;

        let event_handle = audio_client.set_get_eventhandle()?;
        let capture_client = audio_client.get_audiocaptureclient()?;
        let bytes_per_frame = format.block_align_bytes()?;

        Ok(Self {
            _com: com,
            audio_client,
            capture_client,
            event_handle,
            format,
            bytes_per_frame,
        })
    }

    fn try_read_chunk(&mut self) -> Result<Option<AudioChunk>, AudioCaptureError> {
        let Some(packet_frames) = self.capture_client.get_next_packet_size()? else {
            return Ok(None);
        };

        if packet_frames == 0 {
            return Ok(None);
        }

        let mut bytes = vec![0_u8; packet_frames as usize * self.bytes_per_frame];
        let (frames, buffer_info) = self.capture_client.read_from_device(&mut bytes)?;

        if frames == 0 {
            return Ok(None);
        }

        bytes.truncate(frames as usize * self.bytes_per_frame);

        if buffer_info.flags.silent {
            bytes.fill(0);
        }

        Ok(Some(AudioChunk::new(self.format, bytes)?))
    }
}

impl CaptureDriver for WindowsCaptureDriver {
    fn start(&mut self) -> Result<(), AudioCaptureError> {
        self.audio_client.start_stream()?;
        Ok(())
    }

    fn next_chunk(&mut self, timeout_ms: u32) -> Result<Option<AudioChunk>, AudioCaptureError> {
        if let Some(chunk) = self.try_read_chunk()? {
            return Ok(Some(chunk));
        }

        match self.event_handle.wait_for_event(timeout_ms) {
            Ok(()) => self.try_read_chunk(),
            Err(wasapi::WasapiError::EventTimeout) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn stop(&mut self) -> Result<(), AudioCaptureError> {
        self.audio_client.stop_stream()?;
        Ok(())
    }
}

fn map_wave_format(wave_format: &wasapi::WaveFormat) -> Result<AudioFormat, AudioCaptureError> {
    let sample_type = match wave_format.get_subformat()? {
        wasapi::SampleType::Int => AudioSampleType::Int,
        wasapi::SampleType::Float => AudioSampleType::Float,
    };

    Ok(AudioFormat {
        sample_rate_hz: wave_format.get_samplespersec(),
        channels: wave_format.get_nchannels(),
        bits_per_sample: wave_format.get_bitspersample(),
        sample_type,
    })
}
