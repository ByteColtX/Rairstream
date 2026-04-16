use std::mem;

use num_traits::ToPrimitive;

use crate::audio::{AudioChunk, AudioFormat, AudioSampleType};

use super::{
    AirPlayError, RAOP_BITS_PER_SAMPLE, RAOP_CHANNELS, RAOP_FRAMES_PER_PACKET, RAOP_SAMPLE_RATE_HZ,
};

/// 首版 `RAOP` 发送端使用的固定音频描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecDescription {
    pub encoding_name: &'static str,
    pub rtpmap: &'static str,
    pub fmtp: Option<String>,
}

impl CodecDescription {
    #[must_use]
    pub fn pcm_stereo() -> Self {
        Self {
            encoding_name: "L16",
            rtpmap: "L16/44100/2",
            fmtp: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AudioResampler {
    source_format: AudioFormat,
    sender_volume_gain: f64,
    phase_numerator: u32,
    pending_input_frames: Vec<[f64; 2]>,
    pending_output_frames: Vec<[f64; 2]>,
}

impl AudioResampler {
    #[must_use]
    pub fn new(source_format: AudioFormat) -> Self {
        Self {
            source_format,
            sender_volume_gain: 1.0,
            phase_numerator: 0,
            pending_input_frames: Vec::new(),
            pending_output_frames: Vec::new(),
        }
    }

    pub fn set_sender_volume_percent(&mut self, percent: u8) {
        self.sender_volume_gain = f64::from(percent.min(100)) / 100.0;
    }

    pub fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<Vec<u8>>, AirPlayError> {
        if chunk.format != self.source_format {
            return Err(AirPlayError::UnsupportedAudioFormat {
                message: String::from("同一条采集流内的音频格式发生了变化"),
            });
        }

        let mut mixed_frames = decode_and_downmix(chunk)?;
        if mixed_frames.is_empty() {
            return Ok(Vec::new());
        }
        apply_gain(&mut mixed_frames, self.sender_volume_gain);

        let resampled_frames = if self.source_format.sample_rate_hz == RAOP_SAMPLE_RATE_HZ {
            mixed_frames
        } else {
            let mut combined_frames = mem::take(&mut self.pending_input_frames);
            combined_frames.append(&mut mixed_frames);
            let (resampled_frames, consumed_frames) = resample_frames(
                &combined_frames,
                &mut self.phase_numerator,
                self.source_format.sample_rate_hz,
                RAOP_SAMPLE_RATE_HZ,
            );
            self.pending_input_frames = combined_frames[consumed_frames..].to_vec();
            resampled_frames
        };

        Ok(drain_pcm_packets(
            resampled_frames,
            RAOP_FRAMES_PER_PACKET,
            &mut self.pending_output_frames,
        ))
    }
}

pub fn validate_input_format(format: AudioFormat) -> Result<(), AirPlayError> {
    format
        .block_align_bytes()
        .map(|_| ())
        .map_err(|error| AirPlayError::UnsupportedAudioFormat {
            message: error.to_string(),
        })
}

fn decode_and_downmix(chunk: &AudioChunk) -> Result<Vec<[f64; 2]>, AirPlayError> {
    let bytes_per_sample = usize::from(chunk.format.bits_per_sample / 8);
    let channels = usize::from(chunk.format.channels);
    let block_align =
        chunk
            .format
            .block_align_bytes()
            .map_err(|error| AirPlayError::UnsupportedAudioFormat {
                message: error.to_string(),
            })?;
    let left_count =
        u32::try_from(channels.div_ceil(2)).map_err(|_| AirPlayError::UnsupportedAudioFormat {
            message: String::from("声道数超出首版支持范围"),
        })?;
    let right_count =
        u32::try_from(channels / 2).map_err(|_| AirPlayError::UnsupportedAudioFormat {
            message: String::from("声道数超出首版支持范围"),
        })?;
    let mut frames = Vec::with_capacity(chunk.frames);

    for frame_index in 0..chunk.frames {
        let frame_offset = frame_index * block_align;
        let mut left_sum = 0.0_f64;
        let mut right_sum = 0.0_f64;

        for channel_index in 0..channels {
            let sample_offset = frame_offset + channel_index * bytes_per_sample;
            let sample = decode_sample(
                &chunk.bytes[sample_offset..sample_offset + bytes_per_sample],
                chunk.format,
            )?;

            if channel_index % 2 == 0 {
                left_sum += sample;
            } else {
                right_sum += sample;
            }
        }

        let left = if left_count == 0 {
            0.0
        } else {
            left_sum / f64::from(left_count)
        };
        let right = if right_count == 0 {
            left
        } else {
            right_sum / f64::from(right_count)
        };
        frames.push([left, right]);
    }

    Ok(frames)
}

fn decode_sample(bytes: &[u8], format: AudioFormat) -> Result<f64, AirPlayError> {
    match (format.sample_type, format.bits_per_sample) {
        (AudioSampleType::Float, 32) => {
            let mut sample = [0_u8; 4];
            sample.copy_from_slice(bytes);
            Ok(f64::from(f32::from_le_bytes(sample).clamp(-1.0, 1.0)))
        }
        (AudioSampleType::Int, 16) => {
            let mut sample = [0_u8; 2];
            sample.copy_from_slice(bytes);
            Ok(f64::from(i16::from_le_bytes(sample)) / f64::from(i16::MAX))
        }
        (AudioSampleType::Int, 32) => {
            let mut sample = [0_u8; 4];
            sample.copy_from_slice(bytes);
            Ok(f64::from(i32::from_le_bytes(sample)) / f64::from(i32::MAX))
        }
        _ => Err(AirPlayError::UnsupportedAudioFormat {
            message: format!(
                "暂不支持 {:?} / {}bit 输入格式",
                format.sample_type, format.bits_per_sample
            ),
        }),
    }
}

fn apply_gain(frames: &mut [[f64; 2]], gain: f64) {
    for frame in frames {
        frame[0] *= gain;
        frame[1] *= gain;
    }
}

fn resample_frames(
    input: &[[f64; 2]],
    phase_numerator: &mut u32,
    source_rate_hz: u32,
    target_rate_hz: u32,
) -> (Vec<[f64; 2]>, usize) {
    if input.len() < 2 {
        return (Vec::new(), 0);
    }

    let mut output = Vec::new();
    let mut source_index = 0_usize;
    let mut phase = *phase_numerator;

    while source_index + 1 < input.len() {
        let fraction = f64::from(phase) / f64::from(target_rate_hz);
        let current = input[source_index];
        let next = input[source_index + 1];
        output.push([
            interpolate_sample(current[0], next[0], fraction),
            interpolate_sample(current[1], next[1], fraction),
        ]);

        phase = phase.saturating_add(source_rate_hz);
        source_index =
            source_index.saturating_add(usize::try_from(phase / target_rate_hz).unwrap_or(0));
        phase %= target_rate_hz;
    }

    *phase_numerator = phase;
    let consumed_frames = source_index.min(input.len().saturating_sub(1));
    (output, consumed_frames)
}

fn interpolate_sample(current: f64, next: f64, fraction: f64) -> f64 {
    current + (next - current) * fraction
}

fn drain_pcm_packets(
    mut frames: Vec<[f64; 2]>,
    frames_per_packet: usize,
    pending_output_frames: &mut Vec<[f64; 2]>,
) -> Vec<Vec<u8>> {
    if !pending_output_frames.is_empty() {
        let mut combined_frames = mem::take(pending_output_frames);
        combined_frames.append(&mut frames);
        frames = combined_frames;
    }

    let completed_frames = frames.len() / frames_per_packet * frames_per_packet;
    let pending_frames = frames.split_off(completed_frames);
    let packets = frames
        .chunks_exact(frames_per_packet)
        .map(encode_pcm_packet)
        .collect();
    *pending_output_frames = pending_frames;
    packets
}

fn encode_pcm_packet(frames: &[[f64; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        frames.len() * usize::from(RAOP_CHANNELS) * usize::from(RAOP_BITS_PER_SAMPLE / 8),
    );

    for frame in frames {
        bytes.extend_from_slice(&quantize_sample(frame[0]).to_be_bytes());
        bytes.extend_from_slice(&quantize_sample(frame[1]).to_be_bytes());
    }

    bytes
}

fn quantize_sample(sample: f64) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * f64::from(i16::MAX);
    scaled.round().to_i16().unwrap_or_else(|| {
        if scaled.is_sign_negative() {
            i16::MIN
        } else {
            i16::MAX
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AudioResampler, CodecDescription, decode_and_downmix, encode_pcm_packet,
        validate_input_format,
    };
    use crate::audio::{AudioChunk, AudioFormat, AudioSampleType};
    use crate::transport::AirPlayError;

    #[test]
    fn pcm_codec_description_matches_raop_mvp() {
        let codec = CodecDescription::pcm_stereo();

        assert_eq!(codec.encoding_name, "L16");
        assert_eq!(codec.rtpmap, "L16/44100/2");
        assert!(codec.fmtp.is_none());
    }

    #[test]
    fn validate_input_format_accepts_default_pcm_profile() {
        let result = validate_input_format(AudioFormat::default());

        assert!(result.is_ok());
    }

    #[test]
    fn validate_input_format_accepts_common_windows_mix_profile() {
        let result = validate_input_format(AudioFormat {
            sample_rate_hz: 48_000,
            channels: 8,
            bits_per_sample: 32,
            sample_type: crate::audio::AudioSampleType::Float,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn validate_input_format_rejects_zero_channels() {
        let error = validate_input_format(AudioFormat {
            channels: 0,
            ..AudioFormat::default()
        })
        .unwrap_err();

        assert!(matches!(error, AirPlayError::UnsupportedAudioFormat { .. }));
    }

    #[test]
    fn downmixes_multichannel_float_input_to_stereo() {
        let format = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 4,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Float,
        };
        let chunk = AudioChunk::new(
            format,
            [0.8_f32, 0.2, 0.4, 0.6]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.6).abs() < 0.001);
        assert!((frames[0][1] - 0.4).abs() < 0.001);
    }

    #[test]
    fn resampler_outputs_pcm_packets_for_common_windows_mix_profile() {
        let format = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 8,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Float,
        };
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..768 {
            for sample in [0.1_f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8] {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 2);
        assert!(packets.iter().all(|packet| packet.len() == 352 * 4));
    }

    #[test]
    fn resampler_accepts_matching_raop_pcm_input() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..704 {
            bytes.extend_from_slice(&1000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-1000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 2);
    }

    #[test]
    fn resampler_keeps_partial_packet_between_chunks() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::new(format);
        let mut bytes_a = Vec::new();
        let mut bytes_b = Vec::new();
        for _ in 0..400 {
            bytes_a.extend_from_slice(&1000_i16.to_le_bytes());
            bytes_a.extend_from_slice(&(-1000_i16).to_le_bytes());
        }
        for _ in 0..304 {
            bytes_b.extend_from_slice(&1000_i16.to_le_bytes());
            bytes_b.extend_from_slice(&(-1000_i16).to_le_bytes());
        }

        let packets_a = resampler
            .push_chunk(&AudioChunk::new(format, bytes_a).unwrap())
            .unwrap();
        let packets_b = resampler
            .push_chunk(&AudioChunk::new(format, bytes_b).unwrap())
            .unwrap();

        assert_eq!(packets_a.len(), 1);
        assert_eq!(packets_b.len(), 1);
    }

    #[test]
    fn resampler_scales_output_to_silence_at_zero_percent() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::new(format);
        resampler.set_sender_volume_percent(0);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&1000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-1000_i16).to_le_bytes());
        }

        let packets = resampler
            .push_chunk(&AudioChunk::new(format, bytes).unwrap())
            .unwrap();

        assert_eq!(packets.len(), 1);
        assert!(packets[0].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn resampler_scales_output_by_half_at_fifty_percent() {
        let format = AudioFormat::default();
        let mut full_resampler = AudioResampler::new(format);
        let mut half_resampler = AudioResampler::new(format);
        half_resampler.set_sender_volume_percent(50);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&10_000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-10_000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let full_packets = full_resampler.push_chunk(&chunk).unwrap();
        let half_packets = half_resampler.push_chunk(&chunk).unwrap();
        let full_left = i16::from_be_bytes([full_packets[0][0], full_packets[0][1]]);
        let half_left = i16::from_be_bytes([half_packets[0][0], half_packets[0][1]]);

        assert!(half_left.abs() < full_left.abs());
        assert!((i32::from(half_left.abs()) * 2 - i32::from(full_left.abs())).abs() <= 1);
    }

    #[test]
    fn encode_pcm_packet_writes_big_endian_stereo_samples() {
        let bytes = encode_pcm_packet(&[[0.5, -0.5]]);

        assert_eq!(bytes.len(), 4);
        assert!(bytes[0] != 0 || bytes[1] != 0);
        assert!(bytes[2] != 0 || bytes[3] != 0);
    }
}
