use std::collections::HashMap;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::{AudioChunk, CaptureConfig, FileChunkDecoder};
use crate::capture::WindowsLoopbackCapture;
use crate::error::RairstreamError;
use crate::pairing::ReceiverCredentials;
use crate::receiver::Receiver;

use super::connect::{ConnectedReceiver, build_group_sink, connect_receivers};

pub struct PlaybackSession {
    connections: Vec<ConnectedReceiver>,
    capture: Option<crate::audio::RunningCapture>,
}

impl PlaybackSession {
    pub fn stop(mut self) -> Result<(), RairstreamError> {
        let capture_result = match self.capture.take() {
            Some(capture) => capture.stop().map_err(Into::into),
            None => Ok(()),
        };

        combine_playback_results(capture_result, teardown_connections(self.connections))
    }
}

pub fn play_capture(
    receivers: &[Receiver],
    paired_receivers: &HashMap<String, ReceiverCredentials>,
    sender_volume_percent: u16,
) -> Result<PlaybackSession, RairstreamError> {
    let format = WindowsLoopbackCapture::preferred_format()?;
    let connections =
        connect_receivers(receivers, format, paired_receivers, sender_volume_percent)?;
    let sink = match build_group_sink(&connections, format, sender_volume_percent) {
        Ok(sink) => sink,
        Err(error) => {
            return fail_after_cleanup(error, teardown_connections(connections));
        }
    };
    let capture = match WindowsLoopbackCapture::start_with_config(sink, CaptureConfig::default()) {
        Ok(capture) => capture,
        Err(error) => {
            return fail_after_cleanup(error.into(), teardown_connections(connections));
        }
    };

    Ok(PlaybackSession {
        connections,
        capture: Some(capture),
    })
}

pub fn play_file(
    path: &Path,
    receivers: &[Receiver],
    paired_receivers: &HashMap<String, ReceiverCredentials>,
    sender_volume_percent: u16,
) -> Result<(), RairstreamError> {
    let mut decoder = FileChunkDecoder::open(path)?;
    let Some(first_chunk) = decoder.next_chunk()? else {
        return Err(RairstreamError::InvalidInput {
            message: format!(
                "audio file `{}` did not contain decodable samples",
                path.display()
            ),
        });
    };
    let connections = connect_receivers(
        receivers,
        first_chunk.format,
        paired_receivers,
        sender_volume_percent,
    )?;
    let mut sink = match build_group_sink(&connections, first_chunk.format, sender_volume_percent) {
        Ok(sink) => sink,
        Err(error) => {
            return combine_playback_results(Err(error), teardown_connections(connections));
        }
    };

    let result = stream_file_chunks(&mut decoder, &mut sink, first_chunk);
    combine_playback_results(result, teardown_connections(connections))
}

fn stream_file_chunks(
    decoder: &mut FileChunkDecoder,
    sink: &mut impl crate::audio::AudioSink,
    first_chunk: AudioChunk,
) -> Result<(), RairstreamError> {
    stream_chunks(
        sink,
        first_chunk,
        || decoder.next_chunk(),
        Instant::now(),
        sleep_until,
    )?;
    Ok(())
}

fn chunk_duration(chunk: &AudioChunk) -> Duration {
    if chunk.format.sample_rate_hz == 0 {
        return Duration::ZERO;
    }

    Duration::from_secs_f64(chunk.frames as f64 / f64::from(chunk.format.sample_rate_hz))
}

fn sleep_until(start: Instant, played: Duration) {
    let deadline = start + played;
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        thread::sleep(remaining);
    }
}

fn stream_chunks<S, N, W>(
    sink: &mut S,
    first_chunk: AudioChunk,
    mut next_chunk: N,
    start: Instant,
    mut wait_until: W,
) -> Result<(), crate::audio::AudioCaptureError>
where
    S: crate::audio::AudioSink,
    N: FnMut() -> Result<Option<AudioChunk>, crate::audio::AudioCaptureError>,
    W: FnMut(Instant, Duration),
{
    let mut played = Duration::ZERO;
    let first_chunk_duration = chunk_duration(&first_chunk);
    sink.write(first_chunk)?;
    played += first_chunk_duration;

    while let Some(chunk) = next_chunk()? {
        wait_until(start, played);
        let duration = chunk_duration(&chunk);
        sink.write(chunk)?;
        played += duration;
    }

    Ok(())
}

fn combine_playback_results(
    primary: Result<(), RairstreamError>,
    cleanup: Result<(), RairstreamError>,
) -> Result<(), RairstreamError> {
    match (primary, cleanup) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn fail_after_cleanup<T>(
    primary: RairstreamError,
    cleanup: Result<(), RairstreamError>,
) -> Result<T, RairstreamError> {
    match combine_playback_results(Err(primary), cleanup) {
        Ok(()) => {
            unreachable!("cleanup after an error must not succeed without returning an error")
        }
        Err(error) => Err(error),
    }
}

fn teardown_connections(connections: Vec<ConnectedReceiver>) -> Result<(), RairstreamError> {
    let mut first_error: Option<crate::session::AirPlayError> = None;
    for connection in connections {
        if let Err(error) = connection.connection.teardown() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }

    if let Some(error) = first_error {
        return Err(error.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Instant;

    use crate::audio::{AudioCaptureError, AudioChunk, AudioFormat, AudioSink};
    use crate::error::RairstreamError;

    use super::{chunk_duration, combine_playback_results, stream_chunks};

    #[derive(Debug, Default)]
    struct RecordingSink {
        chunks: Vec<AudioChunk>,
    }

    impl AudioSink for RecordingSink {
        fn write(&mut self, chunk: AudioChunk) -> Result<(), AudioCaptureError> {
            self.chunks.push(chunk);
            Ok(())
        }
    }

    fn pcm_chunk(frames: usize) -> AudioChunk {
        AudioChunk::new(AudioFormat::default(), vec![0; frames * 4]).unwrap()
    }

    #[test]
    fn stream_chunks_waits_only_before_follow_up_chunks() {
        let first = pcm_chunk(176);
        let second = pcm_chunk(176);
        let mut responses = VecDeque::from([Ok(Some(second.clone())), Ok(None)]);
        let mut waits = Vec::new();
        let mut sink = RecordingSink::default();

        stream_chunks(
            &mut sink,
            first.clone(),
            || responses.pop_front().unwrap_or(Ok(None)),
            Instant::now(),
            |_, played| waits.push(played),
        )
        .unwrap();

        assert_eq!(sink.chunks, vec![first.clone(), second]);
        assert_eq!(waits, vec![chunk_duration(&first)]);
    }

    #[test]
    fn combine_playback_results_prefers_primary_error() {
        let result = combine_playback_results(
            Err(RairstreamError::Playback {
                message: String::from("stream failed"),
            }),
            Err(RairstreamError::InvalidInput {
                message: String::from("teardown failed"),
            }),
        )
        .unwrap_err();

        assert!(matches!(result, RairstreamError::Playback { .. }));
    }

    #[test]
    fn combine_playback_results_returns_cleanup_error_when_primary_succeeds() {
        let result = combine_playback_results(
            Ok(()),
            Err(RairstreamError::InvalidInput {
                message: String::from("teardown failed"),
            }),
        )
        .unwrap_err();

        assert!(matches!(result, RairstreamError::InvalidInput { .. }));
    }

    #[test]
    fn stream_chunks_propagates_sink_errors() {
        struct FailingSink;

        impl AudioSink for FailingSink {
            fn write(&mut self, _chunk: AudioChunk) -> Result<(), AudioCaptureError> {
                Err(AudioCaptureError::InvalidFormat {
                    message: String::from("audio sink rejected chunk"),
                })
            }
        }

        let error = stream_chunks(
            &mut FailingSink,
            pcm_chunk(176),
            || Ok(None),
            Instant::now(),
            |_, _| unreachable!("wait should not be called after a failed first write"),
        )
        .unwrap_err();

        assert!(matches!(error, AudioCaptureError::InvalidFormat { .. }));
    }
}
