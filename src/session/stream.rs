use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::{AudioChunk, CaptureConfig, FileChunkDecoder};
use crate::capture::WindowsLoopbackCapture;
use crate::error::RairstreamError;
use crate::pairing::ReceiverCredentials;
use crate::receiver::Receiver;

use super::LatencyProfile;
use super::connect::{ConnectedReceiver, build_group_sink, connect_receivers};

pub struct PlaybackSession {
    connections: Vec<ConnectedReceiver>,
    capture: Option<crate::audio::RunningCapture>,
}

impl PlaybackSession {
    #[must_use]
    pub fn transport_error(&self) -> Option<RairstreamError> {
        self.connections
            .iter()
            .find_map(|connection| connection.connection.transport_error())
            .map(Into::into)
    }

    pub fn stop(mut self) -> Result<(), RairstreamError> {
        let capture_result = match self.capture.take() {
            Some(capture) => capture.stop().map_err(Into::into),
            None => Ok(()),
        };

        combine_playback_results(capture_result, teardown_connections(self.connections))
    }
}

pub fn play_capture<S>(
    receivers: &[Receiver],
    paired_receivers: &HashMap<String, ReceiverCredentials, S>,
    sender_volume_percent: u16,
    latency_profile: LatencyProfile,
) -> Result<PlaybackSession, RairstreamError>
where
    S: BuildHasher,
{
    let format = WindowsLoopbackCapture::preferred_format()?;
    let connections = connect_receivers(
        receivers,
        format,
        paired_receivers,
        sender_volume_percent,
        latency_profile,
    )?;
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

pub fn play_file<S>(
    path: &Path,
    receivers: &[Receiver],
    paired_receivers: &HashMap<String, ReceiverCredentials, S>,
    sender_volume_percent: u16,
    latency_profile: LatencyProfile,
) -> Result<(), RairstreamError>
where
    S: BuildHasher,
{
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
        latency_profile,
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

    let frames = chunk.frames as u128;
    let sample_rate = u128::from(chunk.format.sample_rate_hz);
    let total_nanos = frames.saturating_mul(1_000_000_000) / sample_rate;
    let seconds = total_nanos / 1_000_000_000;
    let nanos = total_nanos % 1_000_000_000;

    Duration::new(
        u64::try_from(seconds).unwrap_or(u64::MAX),
        u32::try_from(nanos).unwrap_or(999_999_999),
    )
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
    use std::collections::{HashMap, VecDeque};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::audio::{AudioCaptureError, AudioChunk, AudioFormat, AudioSink};
    use crate::error::RairstreamError;
    use crate::pairing::ReceiverCredentials;
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::session::{AirPlayError, LatencyProfile};

    use super::{
        PlaybackSession, chunk_duration, combine_playback_results, connect_receivers, stream_chunks,
    };

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

    #[test]
    fn playback_session_reports_keepalive_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            for response in [
                "RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n",
                "RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n",
                "RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5100;control_port=5101;timing_port=5102\r\nSession: deadbeef;timeout=1\r\n\r\n",
                "RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n",
            ] {
                let _request = read_rtsp_message(&mut reader).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }

            let keepalive_request = read_rtsp_message(&mut reader).unwrap();
            assert!(keepalive_request.starts_with("OPTIONS *"));
            stream
                .write_all(b"RTSP/1.0 500 Server Error\r\nSession: deadbeef\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            tx.send(()).unwrap();
        });

        let paired_receivers: HashMap<String, ReceiverCredentials> = HashMap::new();
        let connections = connect_receivers(
            &[build_receiver(port)],
            AudioFormat::default(),
            &paired_receivers,
            100,
            LatencyProfile::safe(),
        )
        .unwrap();
        let session = PlaybackSession {
            connections,
            capture: None,
        };

        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let error = wait_for_transport_error(&session).unwrap();

        assert!(matches!(
            error,
            RairstreamError::Session(AirPlayError::Protocol { message })
                if message == "RTSP keepalive returned failure status 500"
        ));

        session.stop().unwrap();
    }

    fn build_receiver(port: u16) -> Receiver {
        Receiver {
            id: String::from("speaker"),
            name: String::from("Speaker"),
            host: String::from("127.0.0.1"),
            port,
            generation: AirPlayGeneration::AirPlay1,
            transport_profile: ReceiverKind::ClassicRaop,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields()
    }

    fn read_rtsp_message(reader: &mut BufReader<TcpStream>) -> Result<String, std::io::Error> {
        let raw = read_rtsp_message_bytes(reader)?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    fn read_rtsp_message_bytes(
        reader: &mut BufReader<TcpStream>,
    ) -> Result<Vec<u8>, std::io::Error> {
        let mut raw = Vec::new();
        let mut content_length = 0_usize;

        loop {
            let mut line = String::new();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }

            let normalized = line.trim_end_matches(['\r', '\n']);
            raw.extend_from_slice(normalized.as_bytes());
            raw.extend_from_slice(b"\r\n");

            if normalized.is_empty() {
                break;
            }

            if let Some((name, value)) = normalized.split_once(':')
                && name.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_length = value.trim().parse::<usize>().unwrap();
            }
        }

        if content_length > 0 {
            let mut body = vec![0_u8; content_length];
            reader.read_exact(&mut body)?;
            raw.extend_from_slice(&body);
        }

        Ok(raw)
    }

    fn wait_for_transport_error(session: &PlaybackSession) -> Option<RairstreamError> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if let Some(error) = session.transport_error() {
                return Some(error);
            }
            thread::sleep(Duration::from_millis(25));
        }
        None
    }
}
