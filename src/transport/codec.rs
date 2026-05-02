//! 输入音频的 `decode`、`downmix`、`resample` 与 `packetize` 路径。

use std::mem;

use num_traits::ToPrimitive;

use crate::audio::{AudioChunk, AudioFormat, AudioSampleType};
use crate::config::MAX_SENDER_VOLUME_PERCENT;

use super::{
    AirPlayError, RAOP_BITS_PER_SAMPLE, RAOP_CHANNELS, RAOP_FRAMES_PER_PACKET, RAOP_SAMPLE_RATE_HZ,
};

const CENTER_MIX_GAIN: f64 = 0.707_106_781_186_547_6;
const SURROUND_MIX_GAIN: f64 = 0.5;
const LFE_MIX_GAIN: f64 = 0.5;

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

/// 将输入 `AudioChunk` 重采样并切成 `RAOP` 所需的 PCM packet。
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
        Self::with_sender_volume_percent(source_format, 100)
    }

    #[must_use]
    pub fn with_sender_volume_percent(
        source_format: AudioFormat,
        sender_volume_percent: u16,
    ) -> Self {
        let mut resampler = Self {
            source_format,
            sender_volume_gain: 1.0,
            phase_numerator: 0,
            pending_input_frames: Vec::new(),
            pending_output_frames: Vec::new(),
        };
        resampler.set_sender_volume_percent(sender_volume_percent);
        resampler
    }

    pub fn set_sender_volume_percent(&mut self, percent: u16) {
        self.sender_volume_gain = f64::from(percent.min(MAX_SENDER_VOLUME_PERCENT)) / 100.0;
    }

    pub fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<Vec<u8>>, AirPlayError> {
        if chunk.format != self.source_format {
            return Err(AirPlayError::UnsupportedAudioFormat {
                message: String::from("audio format changed within a single capture stream"),
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
    let _left_count =
        u32::try_from(channels.div_ceil(2)).map_err(|_| AirPlayError::UnsupportedAudioFormat {
            message: String::from("channel count exceeds current support"),
        })?;
    let _right_count =
        u32::try_from(channels / 2).map_err(|_| AirPlayError::UnsupportedAudioFormat {
            message: String::from("channel count exceeds current support"),
        })?;
    let mut frames = Vec::with_capacity(chunk.frames);

    for frame_index in 0..chunk.frames {
        let frame_offset = frame_index * block_align;
        frames.push(decode_frame_to_stereo(
            chunk,
            frame_offset,
            channels,
            bytes_per_sample,
        )?);
    }

    Ok(frames)
}

fn decode_frame_to_stereo(
    chunk: &AudioChunk,
    frame_offset: usize,
    channels: usize,
    bytes_per_sample: usize,
) -> Result<[f64; 2], AirPlayError> {
    let sample = |channel_index: usize| {
        let sample_offset = frame_offset + channel_index * bytes_per_sample;
        decode_sample(
            &chunk.bytes[sample_offset..sample_offset + bytes_per_sample],
            chunk.format,
        )
    };

    match channels {
        0 => Err(AirPlayError::UnsupportedAudioFormat {
            message: String::from("channel count must be greater than 0"),
        }),
        1 => {
            let mono = sample(0)?;
            Ok([mono, mono])
        }
        2 => Ok([sample(0)?, sample(1)?]),
        3 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let center = sample(2)?;
            Ok([
                left + center * CENTER_MIX_GAIN,
                right + center * CENTER_MIX_GAIN,
            ])
        }
        4 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let back_left = sample(2)?;
            let back_right = sample(3)?;
            Ok([
                left + back_left * SURROUND_MIX_GAIN,
                right + back_right * SURROUND_MIX_GAIN,
            ])
        }
        5 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let center = sample(2)?;
            let back_left = sample(3)?;
            let back_right = sample(4)?;
            Ok([
                left + center * CENTER_MIX_GAIN + back_left * SURROUND_MIX_GAIN,
                right + center * CENTER_MIX_GAIN + back_right * SURROUND_MIX_GAIN,
            ])
        }
        6 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let center = sample(2)?;
            let lfe = sample(3)?;
            let back_left = sample(4)?;
            let back_right = sample(5)?;
            Ok([
                left + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_left * SURROUND_MIX_GAIN,
                right
                    + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_right * SURROUND_MIX_GAIN,
            ])
        }
        7 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let center = sample(2)?;
            let lfe = sample(3)?;
            let back_center = sample(4)?;
            let side_left = sample(5)?;
            let side_right = sample(6)?;
            Ok([
                left + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_center * SURROUND_MIX_GAIN
                    + side_left * SURROUND_MIX_GAIN,
                right
                    + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_center * SURROUND_MIX_GAIN
                    + side_right * SURROUND_MIX_GAIN,
            ])
        }
        8 => {
            let left = sample(0)?;
            let right = sample(1)?;
            let center = sample(2)?;
            let lfe = sample(3)?;
            let back_left = sample(4)?;
            let back_right = sample(5)?;
            let side_left = sample(6)?;
            let side_right = sample(7)?;
            Ok([
                left + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_left * SURROUND_MIX_GAIN
                    + side_left * SURROUND_MIX_GAIN,
                right
                    + center * CENTER_MIX_GAIN
                    + lfe * LFE_MIX_GAIN
                    + back_right * SURROUND_MIX_GAIN
                    + side_right * SURROUND_MIX_GAIN,
            ])
        }
        _ => {
            let mut left = sample(0)?;
            let mut right = sample(1)?;

            for channel_index in 2..channels {
                let channel_sample = sample(channel_index)?;
                if channel_index % 2 == 0 {
                    left += channel_sample * SURROUND_MIX_GAIN;
                } else {
                    right += channel_sample * SURROUND_MIX_GAIN;
                }
            }

            Ok([left, right])
        }
    }
}

fn apply_gain(frames: &mut [[f64; 2]], gain_multiplier: f64) {
    for frame in frames {
        frame[0] *= gain_multiplier;
        frame[1] *= gain_multiplier;
    }
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
        (AudioSampleType::Int, 24) => {
            let sample = i32::from_le_bytes([
                bytes[0],
                bytes[1],
                bytes[2],
                if bytes[2] & 0x80 == 0 { 0 } else { 0xff },
            ]);
            Ok(f64::from(sample) / 8_388_607.0)
        }
        (AudioSampleType::Int, 32) => {
            let mut sample = [0_u8; 4];
            sample.copy_from_slice(bytes);
            Ok(f64::from(i32::from_le_bytes(sample)) / f64::from(i32::MAX))
        }
        _ => Err(AirPlayError::UnsupportedAudioFormat {
            message: format!(
                "unsupported input format {:?} / {}bit",
                format.sample_type, format.bits_per_sample
            ),
        }),
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
        bytes.extend_from_slice(&quantize_sample(protect_peak(frame[0])).to_be_bytes());
        bytes.extend_from_slice(&quantize_sample(protect_peak(frame[1])).to_be_bytes());
    }

    bytes
}

fn protect_peak(sample: f64) -> f64 {
    let magnitude = sample.abs();
    if magnitude <= 1.0 {
        return sample;
    }

    let excess = magnitude - 1.0;
    let soft_limited = excess / (1.0 + excess);
    let quantize_floor = (f64::from(i16::MAX) - 256.0) / f64::from(i16::MAX);
    let protected = 1.0 - (1.0 - quantize_floor) * soft_limited;
    sample.signum() * protected
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
        AudioResampler, CodecDescription, decode_and_downmix, encode_pcm_packet, protect_peak,
        validate_input_format,
    };
    use crate::audio::{AudioChunk, AudioFormat, AudioSampleType};
    use crate::config::MAX_SENDER_VOLUME_PERCENT;
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
            [0.4_f32, 0.2, 0.1, 0.3]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.45).abs() < 0.001);
        assert!((frames[0][1] - 0.35).abs() < 0.001);
    }

    #[test]
    fn downmixes_single_channel_input_to_dual_mono() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 1,
            bits_per_sample: 16,
            sample_type: AudioSampleType::Int,
        };
        let chunk = AudioChunk::new(format, 10_000_i16.to_le_bytes().into()).unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - frames[0][1]).abs() < 0.000_001);
    }

    #[test]
    fn downmixes_multichannel_32bit_int_input_to_stereo() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 4,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Int,
        };
        let chunk = AudioChunk::new(
            format,
            [1_073_741_824_i32, 536_870_912, 536_870_912, 1_073_741_824]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.625).abs() < 0.001);
        assert!((frames[0][1] - 0.5).abs() < 0.001);
    }

    #[test]
    fn downmixes_multichannel_24bit_input_to_stereo() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 4,
            bits_per_sample: 24,
            sample_type: AudioSampleType::Int,
        };
        let chunk = AudioChunk::new(
            format,
            [
                0x00_u8, 0x00, 0x20, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x20,
            ]
            .into_iter()
            .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.3125).abs() < 0.001);
        assert!((frames[0][1] - 0.25).abs() < 0.001);
    }

    #[test]
    fn keeps_front_left_and_right_level_for_six_channel_input() {
        let format = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 6,
            bits_per_sample: 16,
            sample_type: AudioSampleType::Int,
        };
        let chunk = AudioChunk::new(
            format,
            [20_000_i16, -20_000, 0, 0, 0, 0]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.6104).abs() < 0.001);
        assert!((frames[0][1] + 0.6104).abs() < 0.001);
    }

    #[test]
    fn keeps_front_left_and_right_level_for_eight_channel_input() {
        let format = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 8,
            bits_per_sample: 16,
            sample_type: AudioSampleType::Int,
        };
        let chunk = AudioChunk::new(
            format,
            [20_000_i16, -20_000, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect(),
        )
        .unwrap();

        let frames = decode_and_downmix(&chunk).unwrap();

        assert_eq!(frames.len(), 1);
        assert!((frames[0][0] - 0.6104).abs() < 0.001);
        assert!((frames[0][1] + 0.6104).abs() < 0.001);
    }

    #[test]
    fn resampler_outputs_packets_for_48khz_input_path() {
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
    fn resampler_outputs_packets_for_44_1khz_input_path_without_resampling() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 2,
            bits_per_sample: 16,
            sample_type: AudioSampleType::Int,
        };
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&1000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-1000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].len(), 352 * 4);
    }

    #[test]
    fn resampler_accepts_24bit_pcm_input() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 2,
            bits_per_sample: 24,
            sample_type: AudioSampleType::Int,
        };
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&[0x00, 0x00, 0x40]);
            bytes.extend_from_slice(&[0x00, 0x00, 0xc0]);
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 1);
        assert!(packets[0].iter().any(|byte| *byte != 0));
    }

    #[test]
    fn resampler_accepts_32bit_int_pcm_input() {
        let format = AudioFormat {
            sample_rate_hz: 44_100,
            channels: 2,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Int,
        };
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&1_073_741_824_i32.to_le_bytes());
            bytes.extend_from_slice(&(-1_073_741_824_i32).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 1);
        assert!(packets[0].iter().any(|byte| *byte != 0));
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
    fn resampler_makes_moderate_samples_louder_above_hundred_percent() {
        let format = AudioFormat::default();
        let mut full_resampler = AudioResampler::new(format);
        let mut boosted_resampler = AudioResampler::new(format);
        boosted_resampler.set_sender_volume_percent(125);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&10_000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-10_000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let full_packets = full_resampler.push_chunk(&chunk).unwrap();
        let boosted_packets = boosted_resampler.push_chunk(&chunk).unwrap();
        let full_left = i16::from_be_bytes([full_packets[0][0], full_packets[0][1]]);
        let boosted_left = i16::from_be_bytes([boosted_packets[0][0], boosted_packets[0][1]]);

        assert!(boosted_left.abs() > full_left.abs());
    }

    #[test]
    fn protect_peak_keeps_unity_and_full_scale_samples_transparent() {
        assert!((protect_peak(0.5) - 0.5).abs() < f64::EPSILON);
        assert!((protect_peak(1.0) - 1.0).abs() < f64::EPSILON);
        assert!((protect_peak(-1.0) - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn resampler_keeps_full_scale_samples_transparent_at_hundred_percent() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::new(format);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&i16::MAX.to_le_bytes());
            bytes.extend_from_slice(&(-i16::MAX).to_le_bytes());
        }

        let packets = resampler
            .push_chunk(&AudioChunk::new(format, bytes).unwrap())
            .unwrap();
        let left = i16::from_be_bytes([packets[0][0], packets[0][1]]);
        let right = i16::from_be_bytes([packets[0][2], packets[0][3]]);

        assert_eq!(left, i16::MAX);
        assert_eq!(right, -i16::MAX);
    }

    #[test]
    fn protect_peak_stays_continuous_just_above_unity() {
        let at_unity = protect_peak(1.0);
        let just_above = protect_peak(1.0 + 1.0e-6);
        let slightly_above = protect_peak(1.001);

        assert!((at_unity - 1.0).abs() < f64::EPSILON);
        assert!(just_above < at_unity);
        assert!(at_unity - just_above < 1.0e-4);
        assert!(slightly_above < 1.0);
        assert!(slightly_above <= just_above);
        assert!(protect_peak(-(1.0 + 1.0e-6)) > -1.0);
        assert!(protect_peak(-(1.0 + 1.0e-6)) < 0.0);
    }

    #[test]
    fn protect_peak_maps_over_full_scale_samples_monotonically_below_full_scale() {
        let mild = protect_peak(1.01);
        let medium = protect_peak(1.2);
        let hot = protect_peak(1.8);

        assert!(mild < 1.0);
        assert!(hot < 1.0);
        assert!(mild > medium);
        assert!(medium > hot);
        assert!(protect_peak(-1.01) > -1.0);
        assert!(protect_peak(-1.01) < 0.0);
        assert!(protect_peak(-1.01) < protect_peak(-1.2));
    }

    #[test]
    fn resampler_preserves_distinction_between_boosted_over_full_scale_samples() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::new(format);
        resampler.set_sender_volume_percent(MAX_SENDER_VOLUME_PERCENT);
        let mut bytes = Vec::new();
        for index in 0..352 {
            let sample = match index % 3 {
                0 => 20_000_i16,
                1 => 24_000_i16,
                _ => 30_000_i16,
            };
            bytes.extend_from_slice(&sample.to_le_bytes());
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();
        let first_packet = &packets[0];
        let first = i16::from_be_bytes([first_packet[0], first_packet[1]]);
        let second = i16::from_be_bytes([first_packet[4], first_packet[5]]);
        let third = i16::from_be_bytes([first_packet[8], first_packet[9]]);

        assert!(first > 0);
        assert!(second < i16::MAX);
        assert!(third < i16::MAX);
        assert!(first > second);
        assert!(second > third);
    }

    #[test]
    fn encode_pcm_packet_writes_big_endian_stereo_samples() {
        let bytes = encode_pcm_packet(&[[0.5, -0.5]]);

        assert_eq!(bytes.len(), 4);
        assert!(bytes[0] != 0 || bytes[1] != 0);
        assert!(bytes[2] != 0 || bytes[3] != 0);
    }

    #[test]
    fn resampler_can_represent_sender_volume_boost_up_to_400_percent() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::with_sender_volume_percent(format, 400);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&1000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-1000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();

        assert_eq!(packets.len(), 1);
        assert_eq!(&packets[0][0..2], &(4000_i16).to_be_bytes());
        assert_eq!(&packets[0][2..4], &(-4000_i16).to_be_bytes());
    }

    #[test]
    fn resampler_clamps_boosted_samples_to_pcm_range() {
        let format = AudioFormat::default();
        let mut resampler = AudioResampler::with_sender_volume_percent(format, 400);
        let mut bytes = Vec::new();
        for _ in 0..352 {
            bytes.extend_from_slice(&30000_i16.to_le_bytes());
            bytes.extend_from_slice(&(-30000_i16).to_le_bytes());
        }
        let chunk = AudioChunk::new(format, bytes).unwrap();

        let packets = resampler.push_chunk(&chunk).unwrap();
        let left = i16::from_be_bytes([packets[0][0], packets[0][1]]);
        let right = i16::from_be_bytes([packets[0][2], packets[0][3]]);

        assert_eq!(packets.len(), 1);
        assert!(left > 30000);
        assert!(left < i16::MAX);
        assert!(right < -30000);
        assert!(right > i16::MIN);
    }
}
