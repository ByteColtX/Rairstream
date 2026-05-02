use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

use crate::audio::convert::audio_buffer_ref_to_chunk;
use crate::audio::{AudioCaptureError, AudioChunk, AudioFormat};

pub struct FileChunkDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    output_format: Option<AudioFormat>,
    exhausted: bool,
}

impl FileChunkDecoder {
    pub fn open(path: &Path) -> Result<Self, AudioCaptureError> {
        let file = File::open(path).map_err(|error| AudioCaptureError::RuntimeInitialization {
            message: format!("failed to open `{}`: {error}", path.display()),
        })?;
        let media_source = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            hint.with_extension(extension);
        }

        let probed = get_probe()
            .format(
                &hint,
                media_source,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|error| AudioCaptureError::RuntimeInitialization {
                message: format!("failed to probe `{}`: {error}", path.display()),
            })?;
        let format = probed.format;
        let (track_id, decoder) = {
            let track = format
                .default_track()
                .or_else(|| {
                    format
                        .tracks()
                        .iter()
                        .find(|track| track.codec_params.sample_rate.is_some())
                })
                .ok_or_else(|| AudioCaptureError::InvalidFormat {
                    message: format!(
                        "file `{}` did not contain a decodable audio track",
                        path.display()
                    ),
                })?;
            let track_id = track.id;
            let decoder = get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .map_err(|error| AudioCaptureError::RuntimeInitialization {
                    message: format!("failed to create decoder for `{}`: {error}", path.display()),
                })?;
            (track_id, decoder)
        };

        Ok(Self {
            format,
            decoder,
            track_id,
            output_format: None,
            exhausted: false,
        })
    }

    pub fn next_chunk(&mut self) -> Result<Option<AudioChunk>, AudioCaptureError> {
        if self.exhausted {
            return Ok(None);
        }

        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    self.exhausted = true;
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.exhausted = true;
                    return Err(decoder_reset_error());
                }
                Err(error) => {
                    return Err(AudioCaptureError::RuntimeInitialization {
                        message: error.to_string(),
                    });
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(SymphoniaError::ResetRequired) => {
                    self.exhausted = true;
                    return Err(decoder_reset_error());
                }
                Err(error) => {
                    return Err(AudioCaptureError::RuntimeInitialization {
                        message: error.to_string(),
                    });
                }
            };

            let chunk = match decoded {
                AudioBufferRef::U8(_)
                | AudioBufferRef::U16(_)
                | AudioBufferRef::U24(_)
                | AudioBufferRef::U32(_)
                | AudioBufferRef::S8(_)
                | AudioBufferRef::S16(_)
                | AudioBufferRef::S24(_)
                | AudioBufferRef::S32(_)
                | AudioBufferRef::F32(_)
                | AudioBufferRef::F64(_) => audio_buffer_ref_to_chunk(decoded)?,
            };

            if chunk.frames > 0 {
                remember_output_format(&mut self.output_format, &chunk)?;
                return Ok(Some(chunk));
            }
        }
    }
}

fn decoder_reset_error() -> AudioCaptureError {
    AudioCaptureError::InvalidFormat {
        message: String::from(
            "decoded audio format changed mid-stream and requires a decoder reset",
        ),
    }
}

fn remember_output_format(
    output_format: &mut Option<AudioFormat>,
    chunk: &AudioChunk,
) -> Result<(), AudioCaptureError> {
    match output_format {
        Some(format) if *format == chunk.format => Ok(()),
        Some(format) => Err(AudioCaptureError::InvalidFormat {
            message: format!(
                "decoded audio format changed from {} to {}",
                describe_audio_format(*format),
                describe_audio_format(chunk.format)
            ),
        }),
        None => {
            *output_format = Some(chunk.format);
            Ok(())
        }
    }
}

fn describe_audio_format(format: AudioFormat) -> String {
    format!(
        "{} Hz, {} ch, {}-bit {:?}",
        format.sample_rate_hz, format.channels, format.bits_per_sample, format.sample_type
    )
}

#[cfg(test)]
mod tests {
    use super::remember_output_format;
    use crate::audio::{AudioChunk, AudioFormat, AudioSampleType};

    fn chunk_with_format(format: AudioFormat) -> AudioChunk {
        let block_align = format.block_align_bytes().unwrap();
        AudioChunk::new(format, vec![0; block_align]).unwrap()
    }

    #[test]
    fn remember_output_format_accepts_matching_chunks() {
        let mut output_format = None;
        let format = AudioFormat::default();

        remember_output_format(&mut output_format, &chunk_with_format(format)).unwrap();
        remember_output_format(&mut output_format, &chunk_with_format(format)).unwrap();

        assert_eq!(output_format, Some(format));
    }

    #[test]
    fn remember_output_format_rejects_mid_stream_format_changes() {
        let mut output_format = None;
        let initial = AudioFormat::default();
        let changed = AudioFormat {
            sample_rate_hz: 48_000,
            channels: 2,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Float,
        };

        remember_output_format(&mut output_format, &chunk_with_format(initial)).unwrap();
        let error =
            remember_output_format(&mut output_format, &chunk_with_format(changed)).unwrap_err();

        assert!(matches!(
            error,
            crate::audio::AudioCaptureError::InvalidFormat { .. }
        ));
    }
}
