use symphonia::core::audio::{AudioBufferRef, SampleBuffer};

use crate::audio::{AudioCaptureError, AudioChunk, AudioFormat, AudioSampleType};

pub fn audio_buffer_ref_to_chunk(
    decoded: AudioBufferRef<'_>,
) -> Result<AudioChunk, AudioCaptureError> {
    let spec = *decoded.spec();
    let format = AudioFormat {
        sample_rate_hz: spec.rate,
        channels: u16::try_from(spec.channels.count()).map_err(|_| {
            AudioCaptureError::InvalidFormat {
                message: String::from("channel count exceeded supported range"),
            }
        })?,
        bits_per_sample: 32,
        sample_type: AudioSampleType::Float,
    };
    let mut sample_buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
    sample_buffer.copy_interleaved_ref(decoded);
    let bytes = sample_buffer
        .samples()
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();

    AudioChunk::new(format, bytes)
}
