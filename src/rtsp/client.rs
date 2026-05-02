use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tracing::{debug, trace, warn};

use super::protocol::{RtspRequest, RtspResponse, build_keepalive_request, build_teardown_request};
use crate::session::{AirPlayError, SessionDescriptor};

const RTSP_IO_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RTSP_SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_RTSP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(crate) struct RtspClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl RtspClient {
    pub(crate) fn connect(endpoint: &str) -> Result<Self, AirPlayError> {
        debug!(
            endpoint = %endpoint,
            timeout_secs = RTSP_IO_TIMEOUT.as_secs(),
            "connecting TCP RTSP control channel"
        );
        let writer = TcpStream::connect(endpoint).map_err(map_connection_error)?;
        writer
            .set_read_timeout(Some(RTSP_IO_TIMEOUT))
            .map_err(map_connection_error)?;
        writer
            .set_write_timeout(Some(RTSP_IO_TIMEOUT))
            .map_err(map_connection_error)?;
        let reader = BufReader::new(writer.try_clone().map_err(map_connection_error)?);
        debug!(endpoint = %endpoint, "TCP RTSP control channel connected");

        Ok(Self { writer, reader })
    }

    pub(crate) fn send(&mut self, request: &RtspRequest) -> Result<RtspResponse, AirPlayError> {
        let encoded_request = request.encode();
        debug!(
            method = request.method.as_str(),
            uri = %request.uri,
            cseq = request.headers.get("CSeq").unwrap_or("?"),
            "sending RTSP request"
        );
        trace!(request_bytes = ?encoded_request, "raw RTSP request bytes");
        self.writer
            .write_all(&encoded_request)
            .map_err(map_connection_error)?;
        self.writer.flush().map_err(map_connection_error)?;
        self.read_response()
    }

    fn read_response(&mut self) -> Result<RtspResponse, AirPlayError> {
        let mut head = String::new();
        let mut content_length = 0_usize;

        loop {
            let mut line = String::new();
            let bytes_read = self
                .reader
                .read_line(&mut line)
                .map_err(map_connection_error)?;
            if bytes_read == 0 {
                return Err(AirPlayError::ConnectionFailed {
                    message: String::from("RTSP connection closed before the response completed"),
                });
            }

            let normalized = line.trim_end_matches(['\r', '\n']);
            head.push_str(normalized);
            head.push_str("\r\n");

            if normalized.is_empty() {
                break;
            }

            if let Some((name, value)) = normalized.split_once(':')
                && name.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_length =
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| AirPlayError::Protocol {
                            message: format!("invalid RTSP Content-Length: {}", value.trim()),
                        })?;
            }
        }

        let mut body = vec![0_u8; content_length];
        if content_length > 0 {
            self.reader
                .read_exact(&mut body)
                .map_err(map_connection_error)?;
        }

        let response = RtspResponse::parse_parts(&head, body)?;
        debug!(
            status_code = response.status.code,
            reason_phrase = %response.status.reason_phrase,
            content_length,
            "received RTSP response"
        );
        trace!(response_head = %head, response_body = ?response.body, "raw RTSP response bytes");
        Ok(response)
    }
}

#[derive(Debug)]
enum RtspKeepaliveCommand {
    Stop,
    Teardown(Sender<Result<(), AirPlayError>>),
}

#[derive(Debug)]
pub(crate) struct RtspKeepalive {
    command_tx: Sender<RtspKeepaliveCommand>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct RtspKeepaliveWorker {
    rtsp_client: RtspClient,
    descriptor: SessionDescriptor,
    session_id: String,
    cseq: u32,
    interval: Duration,
    command_rx: Receiver<RtspKeepaliveCommand>,
    transport_terminated: Arc<AtomicBool>,
}

impl RtspKeepalive {
    pub(crate) fn start(
        rtsp_client: RtspClient,
        descriptor: SessionDescriptor,
        session_id: String,
        initial_cseq: u32,
        interval: Duration,
        transport_terminated: Arc<AtomicBool>,
    ) -> Result<Self, AirPlayError> {
        let endpoint = descriptor.device.endpoint();
        let (command_tx, command_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(String::from("raop-rtsp-keepalive"))
            .spawn(move || {
                RtspKeepaliveWorker {
                    rtsp_client,
                    descriptor,
                    session_id,
                    cseq: initial_cseq,
                    interval,
                    command_rx,
                    transport_terminated,
                }
                .run();
            })
            .map_err(map_connection_error)?;
        debug!(
            endpoint = %endpoint,
            interval_secs = interval.as_secs(),
            "RTSP keepalive started"
        );

        Ok(Self {
            command_tx,
            worker: Some(worker),
        })
    }

    pub(crate) fn stop(&mut self, send_teardown: bool) -> Result<(), AirPlayError> {
        if send_teardown {
            let (reply_tx, reply_rx) = mpsc::channel();
            if self
                .command_tx
                .send(RtspKeepaliveCommand::Teardown(reply_tx))
                .is_err()
            {
                self.join_worker();
                return Ok(());
            }
            let result = match reply_rx.recv_timeout(RTSP_IO_TIMEOUT + Duration::from_secs(1)) {
                Ok(result) => result,
                Err(RecvTimeoutError::Timeout) => Err(AirPlayError::ConnectionFailed {
                    message: String::from(
                        "timed out waiting for RTSP keepalive worker to send TEARDOWN",
                    ),
                }),
                Err(RecvTimeoutError::Disconnected) => Ok(()),
            };
            self.join_worker();
            return result;
        }

        let _ = self.command_tx.send(RtspKeepaliveCommand::Stop);
        self.join_worker();
        Ok(())
    }

    fn join_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
            debug!("RTSP keepalive stopped");
        }
    }
}

impl RtspKeepaliveWorker {
    fn run(mut self) {
        let endpoint = self.descriptor.device.endpoint();

        loop {
            match self.command_rx.recv_timeout(self.interval) {
                Ok(RtspKeepaliveCommand::Stop) => {
                    debug!(endpoint = %endpoint, "received RTSP keepalive stop command");
                    break;
                }
                Ok(RtspKeepaliveCommand::Teardown(reply_tx)) => {
                    let result = send_rtsp_teardown(
                        &mut self.rtsp_client,
                        &self.descriptor,
                        &self.session_id,
                        &mut self.cseq,
                    );
                    if let Err(error) = &result {
                        warn!(endpoint = %endpoint, error = %error, "failed to send TEARDOWN");
                    }
                    let _ = reply_tx.send(result);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.cseq = self.cseq.saturating_add(1);
                    let request =
                        build_keepalive_request(&self.descriptor, self.cseq, &self.session_id);
                    match self.rtsp_client.send(&request) {
                        Ok(response) => {
                            if let Err(error) = ensure_success(&response, "RTSP keepalive") {
                                warn!(
                                    endpoint = %endpoint,
                                    status_code = response.status.code,
                                    error = %error,
                                    "RTSP keepalive received failure response"
                                );
                                self.transport_terminated.store(true, Ordering::SeqCst);
                                break;
                            }
                            trace!(
                                endpoint = %endpoint,
                                cseq = self.cseq,
                                status_code = response.status.code,
                                "RTSP keepalive succeeded"
                            );
                        }
                        Err(error) => {
                            warn!(
                                endpoint = %endpoint,
                                cseq = self.cseq,
                                error = %error,
                                "RTSP keepalive failed"
                            );
                            self.transport_terminated.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

pub(crate) fn compute_rtsp_keepalive_interval(session_timeout_secs: Option<u64>) -> Duration {
    let session_timeout_secs =
        session_timeout_secs.unwrap_or(DEFAULT_RTSP_SESSION_TIMEOUT.as_secs());
    let preferred_secs = (session_timeout_secs / 2).max(MIN_RTSP_KEEPALIVE_INTERVAL.as_secs());
    let latest_safe_secs = session_timeout_secs.saturating_sub(1).max(1);

    Duration::from_secs(preferred_secs.min(latest_safe_secs))
}

fn send_rtsp_teardown(
    rtsp_client: &mut RtspClient,
    descriptor: &SessionDescriptor,
    session_id: &str,
    cseq: &mut u32,
) -> Result<(), AirPlayError> {
    *cseq = cseq.saturating_add(1);
    let request = build_teardown_request(descriptor, *cseq, session_id);
    let response = rtsp_client.send(&request)?;
    ensure_success(&response, "TEARDOWN")
}

pub(crate) fn format_response_status(method: &str, response: &RtspResponse) -> String {
    format!(
        "{method} -> {} {}",
        response.status.code, response.status.reason_phrase
    )
}

pub(crate) fn ensure_success(response: &RtspResponse, method: &str) -> Result<(), AirPlayError> {
    if response.is_success() {
        return Ok(());
    }

    if matches!(response.status.code, 401 | 403) {
        return Err(AirPlayError::AuthenticationRequired);
    }

    Err(AirPlayError::Protocol {
        message: format!("{method} returned failure status {}", response.status.code),
    })
}

pub(crate) fn map_connection_error(error: std::io::Error) -> AirPlayError {
    let message = error.to_string();
    drop(error);
    AirPlayError::ConnectionFailed { message }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, atomic::AtomicBool, mpsc};
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::audio::AudioFormat;
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::session::SessionDescriptor;

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

    #[test]
    fn keepalive_interval_uses_expected_bounds() {
        let cases = [
            (None, 30),
            (Some(0), 1),
            (Some(1), 1),
            (Some(2), 1),
            (Some(3), 2),
            (Some(4), 3),
            (Some(5), 4),
            (Some(11), 10),
            (Some(12), 11),
            (Some(13), 12),
            (Some(14), 13),
            (Some(15), 14),
            (Some(16), 15),
            (Some(17), 15),
            (Some(29), 15),
            (Some(30), 15),
            (Some(120), 60),
        ];

        for (timeout_secs, expected_secs) in cases {
            assert_eq!(
                compute_rtsp_keepalive_interval(timeout_secs),
                Duration::from_secs(expected_secs)
            );
        }
    }

    #[test]
    fn teardown_sends_keepalive_options_before_teardown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();

            for response in [
                "RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n",
                "RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n",
            ] {
                let request = read_rtsp_message(&mut reader).unwrap();
                requests.push(request);
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }

            tx.send(requests).unwrap();
        });

        let descriptor = SessionDescriptor::new(build_receiver(port), AudioFormat::default());
        let rtsp_client = RtspClient::connect(&descriptor.device.endpoint()).unwrap();
        let mut keepalive = RtspKeepalive::start(
            rtsp_client,
            descriptor,
            String::from("deadbeef"),
            0,
            Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        thread::sleep(Duration::from_millis(1100));
        keepalive.stop(true).unwrap();

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("OPTIONS *"));
        assert!(requests[0].contains("Session: deadbeef"));
        assert!(requests[1].starts_with("TEARDOWN "));
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
}
