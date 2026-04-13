use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::codec::CodecDescription;
use super::packet::RaopPacketCounters;
use super::rtsp::{
    RtspRequest, RtspResponse, SetupReply, SetupTransport, build_announce_request,
    build_options_request, build_record_request, build_setup_request, build_teardown_request,
    parse_setup_reply,
};
use super::{AirPlayError, RAOP_STARTUP_LATENCY_FRAMES, RaopSinkConfig, SessionDescriptor};

const RTSP_IO_TIMEOUT: Duration = Duration::from_secs(5);
const TIMING_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// `RAOP` 会话生命周期的最小状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaopSessionState {
    Connecting,
    Prepared,
    Streaming,
}

/// 进入实时发送阶段后给 sink 使用的传输句柄。
#[derive(Debug)]
pub struct RaopStreamTransport {
    pub audio_socket: UdpSocket,
    pub control_socket: UdpSocket,
    pub audio_target: SocketAddr,
    pub control_target: SocketAddr,
    pub audio_ssrc: u32,
    pub packet_counters: RaopPacketCounters,
    pub sink_config: RaopSinkConfig,
}

/// 已完成握手的 `RAOP` 控制面连接。
#[derive(Debug)]
pub struct RaopConnection {
    session: RaopSession,
    rtsp_client: RtspClient,
    audio_socket: UdpSocket,
    control_socket: UdpSocket,
    audio_target: SocketAddr,
    control_target: SocketAddr,
    timing_responder: Option<TimingResponder>,
}

impl RaopConnection {
    #[must_use]
    pub fn session(&self) -> &RaopSession {
        &self.session
    }

    pub fn stream_transport(&self) -> Result<RaopStreamTransport, AirPlayError> {
        Ok(RaopStreamTransport {
            audio_socket: self
                .audio_socket
                .try_clone()
                .map_err(map_connection_error)?,
            control_socket: self
                .control_socket
                .try_clone()
                .map_err(map_connection_error)?,
            audio_target: self.audio_target,
            control_target: self.control_target,
            audio_ssrc: self.session.audio_ssrc(),
            packet_counters: self.session.packet_counters(),
            sink_config: self.session.sink_config(),
        })
    }

    pub fn teardown(mut self) -> Result<(), AirPlayError> {
        let request = self.session.teardown_request()?;
        let response = self.rtsp_client.send(&request)?;
        ensure_success(&response, "TEARDOWN")?;
        self.stop_timing_responder();
        Ok(())
    }

    fn stop_timing_responder(&mut self) {
        if let Some(mut timing_responder) = self.timing_responder.take() {
            timing_responder.stop();
        }
    }
}

impl Drop for RaopConnection {
    fn drop(&mut self) {
        self.stop_timing_responder();
    }
}

/// 首版 `RAOP` 会话骨架。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaopSession {
    descriptor: SessionDescriptor,
    sink_config: RaopSinkConfig,
    codec: CodecDescription,
    state: RaopSessionState,
    cseq: u32,
    setup_reply: Option<SetupReply>,
    audio_ssrc: u32,
    packet_counters: RaopPacketCounters,
}

impl RaopSession {
    #[must_use]
    pub fn transport_name() -> &'static str {
        "raop"
    }

    pub fn connect(descriptor: &SessionDescriptor) -> Result<Self, AirPlayError> {
        descriptor.validate()?;
        let (initial_sequence, initial_timestamp, audio_ssrc) = build_rtp_session_seed(descriptor);

        Ok(Self {
            descriptor: descriptor.clone(),
            sink_config: RaopSinkConfig {
                frames_per_packet: descriptor.frames_per_packet,
                ..RaopSinkConfig::default()
            },
            codec: CodecDescription::pcm_stereo(),
            state: RaopSessionState::Connecting,
            cseq: 0,
            setup_reply: None,
            audio_ssrc,
            packet_counters: RaopPacketCounters::new(initial_sequence, initial_timestamp),
        })
    }

    #[must_use]
    pub fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn sink_config(&self) -> RaopSinkConfig {
        self.sink_config
    }

    #[must_use]
    pub fn state(&self) -> RaopSessionState {
        self.state
    }

    #[must_use]
    pub fn audio_ssrc(&self) -> u32 {
        self.audio_ssrc
    }

    #[must_use]
    pub fn packet_counters(&self) -> RaopPacketCounters {
        self.packet_counters
    }

    pub fn handshake(self) -> Result<RaopConnection, AirPlayError> {
        self.handshake_with_progress(|_| ())
    }

    pub fn handshake_with_progress<F>(
        mut self,
        mut progress: F,
    ) -> Result<RaopConnection, AirPlayError>
    where
        F: FnMut(&str),
    {
        let endpoint = self.descriptor.device.endpoint();
        progress(&format!("连接 RTSP 控制通道: {endpoint}"));
        let mut rtsp_client = RtspClient::connect(&endpoint)?;
        let audio_socket = bind_udp_socket()?;
        let control_socket = bind_udp_socket()?;
        let timing_socket = bind_udp_socket()?;
        let audio_port = audio_socket
            .local_addr()
            .map_err(map_connection_error)?
            .port();
        let control_port = control_socket
            .local_addr()
            .map_err(map_connection_error)?
            .port();
        let timing_port = timing_socket
            .local_addr()
            .map_err(map_connection_error)?
            .port();
        let setup_transport = SetupTransport {
            control_port,
            timing_port,
        };
        progress(&format!(
            "本地 UDP 端口已绑定: audio={audio_port}, control={control_port}, timing={timing_port}"
        ));

        progress("发送 OPTIONS");
        let options_response = rtsp_client.send(&self.options_request())?;
        progress(&format_response_status("OPTIONS", &options_response));
        ensure_success(&options_response, "OPTIONS")?;

        progress("发送 ANNOUNCE");
        let announce_response = rtsp_client.send(&self.announce_request())?;
        progress(&format_response_status("ANNOUNCE", &announce_response));
        ensure_success(&announce_response, "ANNOUNCE")?;

        progress("发送 SETUP");
        let setup_response = rtsp_client.send(&self.setup_request(setup_transport))?;
        progress(&format_response_status("SETUP", &setup_response));
        self.apply_setup_response(&setup_response)?;

        let setup_reply = self.setup_reply()?.clone();
        progress(&format!(
            "设备 UDP 端口: audio={}, control={}, timing={}",
            setup_reply.server_port, setup_reply.control_port, setup_reply.timing_port
        ));

        progress("发送 RECORD");
        let record_response = rtsp_client.send(&self.record_request()?)?;
        progress(&format_response_status("RECORD", &record_response));
        self.apply_record_response(&record_response)?;

        let audio_target =
            resolve_socket_addr(&self.descriptor.device.host, setup_reply.server_port)?;
        let control_target =
            resolve_socket_addr(&self.descriptor.device.host, setup_reply.control_port)?;
        let timing_responder = TimingResponder::start(timing_socket)?;
        progress("RAOP 会话已进入 Streaming");

        Ok(RaopConnection {
            session: self,
            rtsp_client,
            audio_socket,
            control_socket,
            audio_target,
            control_target,
            timing_responder: Some(timing_responder),
        })
    }

    #[must_use]
    pub fn options_request(&mut self) -> RtspRequest {
        let cseq = self.next_cseq();
        build_options_request(&self.descriptor, cseq)
    }

    #[must_use]
    pub fn announce_request(&mut self) -> RtspRequest {
        let cseq = self.next_cseq();
        build_announce_request(&self.descriptor, cseq, &self.codec)
    }

    #[must_use]
    pub fn setup_request(&mut self, transport: SetupTransport) -> RtspRequest {
        let cseq = self.next_cseq();
        build_setup_request(&self.descriptor, cseq, transport)
    }

    pub fn apply_setup_response(&mut self, response: &RtspResponse) -> Result<(), AirPlayError> {
        self.setup_reply = Some(parse_setup_reply(response)?);
        self.state = RaopSessionState::Prepared;
        Ok(())
    }

    pub fn record_request(&mut self) -> Result<RtspRequest, AirPlayError> {
        let session_id = self.session_id()?.to_string();
        let (sequence, rtp_timestamp) = self.packet_counters.peek_audio_packet();
        let cseq = self.next_cseq();

        Ok(build_record_request(
            &self.descriptor,
            cseq,
            &session_id,
            sequence,
            rtp_timestamp,
        ))
    }

    pub fn apply_record_response(&mut self, response: &RtspResponse) -> Result<(), AirPlayError> {
        ensure_success(response, "RECORD")?;
        self.state = RaopSessionState::Streaming;
        Ok(())
    }

    pub fn teardown_request(&mut self) -> Result<RtspRequest, AirPlayError> {
        let session_id = self.session_id()?.to_string();
        let cseq = self.next_cseq();

        Ok(build_teardown_request(&self.descriptor, cseq, &session_id))
    }

    fn next_cseq(&mut self) -> u32 {
        self.cseq = self.cseq.saturating_add(1);
        self.cseq
    }

    fn session_id(&self) -> Result<&str, AirPlayError> {
        self.setup_reply
            .as_ref()
            .map(|reply| reply.session_id.as_str())
            .ok_or(AirPlayError::NotReady)
    }

    fn setup_reply(&self) -> Result<&SetupReply, AirPlayError> {
        self.setup_reply.as_ref().ok_or(AirPlayError::NotReady)
    }
}

#[derive(Debug)]
struct RtspClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl RtspClient {
    fn connect(endpoint: &str) -> Result<Self, AirPlayError> {
        let writer = TcpStream::connect(endpoint).map_err(map_connection_error)?;
        writer
            .set_read_timeout(Some(RTSP_IO_TIMEOUT))
            .map_err(map_connection_error)?;
        writer
            .set_write_timeout(Some(RTSP_IO_TIMEOUT))
            .map_err(map_connection_error)?;
        let reader = BufReader::new(writer.try_clone().map_err(map_connection_error)?);

        Ok(Self { writer, reader })
    }

    fn send(&mut self, request: &RtspRequest) -> Result<RtspResponse, AirPlayError> {
        self.writer
            .write_all(request.encode().as_bytes())
            .map_err(map_connection_error)?;
        self.writer.flush().map_err(map_connection_error)?;
        self.read_response()
    }

    fn read_response(&mut self) -> Result<RtspResponse, AirPlayError> {
        let mut raw = String::new();
        let mut content_length = 0_usize;

        loop {
            let mut line = String::new();
            let bytes_read = self
                .reader
                .read_line(&mut line)
                .map_err(map_connection_error)?;
            if bytes_read == 0 {
                return Err(AirPlayError::ConnectionFailed {
                    message: String::from("RTSP 连接在响应完成前被关闭"),
                });
            }

            let normalized = line.trim_end_matches(['\r', '\n']);
            raw.push_str(normalized);
            raw.push_str("\r\n");

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
                            message: format!("RTSP Content-Length 无效: {}", value.trim()),
                        })?;
            }
        }

        if content_length > 0 {
            let mut body = vec![0_u8; content_length];
            self.reader
                .read_exact(&mut body)
                .map_err(map_connection_error)?;
            raw.push_str(&String::from_utf8_lossy(&body));
        }

        RtspResponse::parse(&raw)
    }
}

#[derive(Debug)]
struct TimingResponder {
    stop_requested: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TimingResponder {
    fn start(socket: UdpSocket) -> Result<Self, AirPlayError> {
        socket
            .set_read_timeout(Some(TIMING_POLL_TIMEOUT))
            .map_err(map_connection_error)?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let worker = thread::Builder::new()
            .name(String::from("raop-timing-responder"))
            .spawn(move || run_timing_responder(socket, worker_stop_requested))
            .map_err(map_connection_error)?;

        Ok(Self {
            stop_requested,
            worker: Some(worker),
        })
    }

    fn stop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for TimingResponder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_timing_responder(socket: UdpSocket, stop_requested: Arc<AtomicBool>) {
    let mut buffer = [0_u8; 1500];

    while !stop_requested.load(Ordering::Acquire) {
        match socket.recv_from(&mut buffer) {
            Ok((len, peer_addr)) => {
                if let Some(reply) = build_timing_reply(&buffer[..len]) {
                    let _ = socket.send_to(&reply, peer_addr);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }

    drop(stop_requested);
    drop(socket);
}

fn build_timing_reply(request: &[u8]) -> Option<[u8; 32]> {
    if request.len() < 32 || request[1] & 0x7f != 82 {
        return None;
    }

    let receive_timestamp = ntp_timestamp_now();
    let transmit_timestamp = ntp_timestamp_now();
    let mut reply = [0_u8; 32];
    reply[0] = 0x80;
    reply[1] = 0x80 | 0x53;
    reply[2..4].copy_from_slice(&request[2..4]);
    reply[4..8].copy_from_slice(&request[4..8]);
    reply[8..16].copy_from_slice(&request[24..32]);
    reply[16..24].copy_from_slice(&receive_timestamp.to_be_bytes());
    reply[24..32].copy_from_slice(&transmit_timestamp.to_be_bytes());
    Some(reply)
}

fn bind_udp_socket() -> Result<UdpSocket, AirPlayError> {
    UdpSocket::bind("0.0.0.0:0").map_err(map_connection_error)
}

fn build_rtp_session_seed(descriptor: &SessionDescriptor) -> (u16, u32, u32) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    hash_identifier_segment(&mut hash, descriptor.device.id.as_bytes());
    hash_identifier_segment(&mut hash, descriptor.device.host.as_bytes());
    hash_identifier_segment(&mut hash, &descriptor.device.port.to_be_bytes());
    hash_identifier_segment(&mut hash, &timestamp.to_be_bytes());

    let initial_sequence = u16::try_from(hash & 0xffff_u64).unwrap_or(1).max(1);
    let initial_timestamp = u32::try_from((hash >> 16) & 0xffff_ffff_u64)
        .unwrap_or_default()
        .wrapping_add(RAOP_STARTUP_LATENCY_FRAMES);
    let audio_ssrc = u32::try_from((hash >> 8) & 0xffff_ffff_u64)
        .unwrap_or(1)
        .max(1);

    (initial_sequence, initial_timestamp, audio_ssrc)
}

fn hash_identifier_segment(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3_u64);
    }

    *hash ^= u64::from(b'|');
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3_u64);
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<SocketAddr, AirPlayError> {
    let endpoint = format!("{host}:{port}");
    endpoint
        .to_socket_addrs()
        .map_err(map_connection_error)?
        .next()
        .ok_or_else(|| AirPlayError::ConnectionFailed {
            message: format!("无法解析设备地址 {endpoint}"),
        })
}

fn format_response_status(method: &str, response: &RtspResponse) -> String {
    format!(
        "{method} -> {} {}",
        response.status.code, response.status.reason_phrase
    )
}

fn ensure_success(response: &RtspResponse, method: &str) -> Result<(), AirPlayError> {
    if response.is_success() {
        return Ok(());
    }

    if matches!(response.status.code, 401 | 403) {
        return Err(AirPlayError::AuthenticationRequired);
    }

    Err(AirPlayError::Protocol {
        message: format!("{method} 返回了失败状态码 {}", response.status.code),
    })
}

fn map_connection_error(error: std::io::Error) -> AirPlayError {
    let message = error.to_string();
    drop(error);
    AirPlayError::ConnectionFailed { message }
}

fn ntp_timestamp_now() -> u64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs().saturating_add(2_208_988_800);
    let fractional = ((u128::from(duration.subsec_nanos())) << 32) / 1_000_000_000_u128;

    (seconds << 32) | u64::try_from(fractional).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream, UdpSocket};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::{RaopSession, RaopSessionState, build_timing_reply};
    use crate::app::{AirPlayGeneration, SpeakerDevice};
    use crate::audio::{AudioFormat, AudioSampleType};
    use crate::transport::AirPlayError;
    use crate::transport::RAOP_STARTUP_LATENCY_FRAMES;
    use crate::transport::RtspResponse;
    use crate::transport::SessionDescriptor;
    use crate::transport::rtsp::SetupTransport;

    fn build_descriptor(format: AudioFormat) -> SessionDescriptor {
        SessionDescriptor::new(
            SpeakerDevice {
                id: String::from("speaker"),
                name: String::from("Speaker"),
                host: String::from("127.0.0.1"),
                port: 7000,
                generation: AirPlayGeneration::AirPlay1,
            },
            format,
        )
    }

    fn build_setup_response() -> RtspResponse {
        RtspResponse::parse(
            "RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5000;control_port=5001;timing_port=5002\r\nSession: deadbeef;timeout=60\r\n\r\n",
        )
        .unwrap()
    }

    #[test]
    fn connect_builds_connecting_session_for_supported_format() {
        let session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();

        assert_eq!(session.state(), RaopSessionState::Connecting);
        assert_eq!(session.sink_config().frames_per_packet, 352);
    }

    #[test]
    fn connect_seeds_non_zero_rtp_session_parameters() {
        let session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        let (sequence, timestamp) = session.packet_counters().peek_audio_packet();

        assert!(sequence > 0);
        assert!(timestamp >= RAOP_STARTUP_LATENCY_FRAMES);
        assert!(session.audio_ssrc() > 0);
    }

    #[test]
    fn connect_accepts_common_windows_mix_profile() {
        let session = RaopSession::connect(&build_descriptor(AudioFormat {
            sample_rate_hz: 48_000,
            channels: 8,
            bits_per_sample: 32,
            sample_type: AudioSampleType::Float,
        }))
        .unwrap();

        assert_eq!(session.state(), RaopSessionState::Connecting);
    }

    #[test]
    fn request_builders_advance_cseq() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();

        let options = session.options_request();
        let announce = session.announce_request();
        let setup = session.setup_request(SetupTransport {
            control_port: 6001,
            timing_port: 6002,
        });

        assert_eq!(options.headers.get("CSeq"), Some("1"));
        assert_eq!(announce.headers.get("CSeq"), Some("2"));
        assert_eq!(setup.headers.get("CSeq"), Some("3"));
    }

    #[test]
    fn apply_setup_response_moves_session_to_prepared() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();

        session
            .apply_setup_response(&build_setup_response())
            .unwrap();

        assert_eq!(session.state(), RaopSessionState::Prepared);
    }

    #[test]
    fn record_request_requires_setup_to_finish() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        let error = session.record_request().unwrap_err();

        assert_eq!(error, AirPlayError::NotReady);
    }

    #[test]
    fn record_request_uses_setup_session_without_advancing_stream_state() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        session
            .apply_setup_response(&build_setup_response())
            .unwrap();
        let expected_packet_counters = session.packet_counters();
        let (sequence, rtp_timestamp) = expected_packet_counters.peek_audio_packet();

        let request = session.record_request().unwrap();

        assert_eq!(request.headers.get("Session"), Some("deadbeef"));
        let expected_rtp_info = format!("seq={sequence};rtptime={rtp_timestamp}");
        assert_eq!(
            request.headers.get("RTP-Info"),
            Some(expected_rtp_info.as_str())
        );
        assert_eq!(session.state(), RaopSessionState::Prepared);
        assert_eq!(session.packet_counters(), expected_packet_counters);
    }

    #[test]
    fn apply_record_response_moves_session_to_streaming() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        session
            .apply_setup_response(&build_setup_response())
            .unwrap();

        session
            .apply_record_response(&RtspResponse::success(200))
            .unwrap();

        assert_eq!(session.state(), RaopSessionState::Streaming);
    }

    #[test]
    fn teardown_request_uses_setup_session() {
        let mut session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        session
            .apply_setup_response(&build_setup_response())
            .unwrap();

        let request = session.teardown_request().unwrap();

        assert_eq!(request.headers.get("Session"), Some("deadbeef"));
    }

    #[test]
    fn handshake_runs_rtsp_flow_with_dynamic_udp_ports() {
        let (server, recorded_requests) = FakeRtspServer::spawn(vec![
            String::from("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n"),
            String::from("RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n"),
            String::from(
                "RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5100;control_port=5101;timing_port=5102\r\nSession: deadbeef;timeout=60\r\n\r\n",
            ),
            String::from("RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n"),
            String::from("RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n"),
        ]);
        let descriptor = SessionDescriptor::new(
            SpeakerDevice {
                id: String::from("speaker"),
                name: String::from("Speaker"),
                host: String::from("127.0.0.1"),
                port: server.port(),
                generation: AirPlayGeneration::AirPlay1,
            },
            AudioFormat::default(),
        );

        let connection = RaopSession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap();
        let expected_audio_ssrc = connection.session().audio_ssrc();
        let expected_packet_counters = connection.session().packet_counters();
        let transport = connection.stream_transport().unwrap();
        let local_control_port = transport.control_socket.local_addr().unwrap().port();
        assert_eq!(transport.audio_ssrc, expected_audio_ssrc);
        assert_eq!(transport.packet_counters, expected_packet_counters);
        connection.teardown().unwrap();

        let requests = recorded_requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests[0].starts_with("OPTIONS "));
        assert!(requests[1].starts_with("ANNOUNCE "));
        assert!(requests[2].starts_with("SETUP "));
        assert!(requests[3].starts_with("RECORD "));
        assert!(requests[4].starts_with("TEARDOWN "));
        assert!(requests[2].contains("control_port="));
        assert!(!requests[2].contains("control_port=6001"));
        assert!(requests[2].contains(&format!("control_port={local_control_port}")));
    }

    #[test]
    fn handshake_maps_authentication_failure() {
        let (server, _recorded_requests) =
            FakeRtspServer::spawn(vec![String::from("RTSP/1.0 401 Unauthorized\r\n\r\n")]);
        let descriptor = SessionDescriptor::new(
            SpeakerDevice {
                id: String::from("speaker"),
                name: String::from("Speaker"),
                host: String::from("127.0.0.1"),
                port: server.port(),
                generation: AirPlayGeneration::AirPlay1,
            },
            AudioFormat::default(),
        );

        let error = RaopSession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap_err();

        assert_eq!(error, AirPlayError::AuthenticationRequired);
    }

    #[test]
    fn timing_reply_copies_sequence_and_request_timestamp() {
        let request = [
            0x80, 0xd2, 0x12, 0x34, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
        ];

        let reply = build_timing_reply(&request).unwrap();

        assert_eq!(reply[1] & 0x7f, 83);
        assert_eq!(&reply[2..4], &[0x12, 0x34]);
        assert_eq!(
            &reply[8..16],
            &[0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
        );
    }

    struct FakeRtspServer {
        listener: TcpListener,
    }

    impl FakeRtspServer {
        fn spawn(responses: Vec<String>) -> (Self, mpsc::Receiver<Vec<String>>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port_listener = listener.try_clone().unwrap();
            let (tx, rx) = mpsc::channel();

            thread::spawn(move || {
                let (mut stream, _) = port_listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut requests = Vec::new();

                for response in responses {
                    let request = read_rtsp_message(&mut reader).unwrap();
                    requests.push(request);
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                }

                tx.send(requests).unwrap();
            });

            (Self { listener }, rx)
        }

        fn port(&self) -> u16 {
            self.listener.local_addr().unwrap().port()
        }
    }

    fn read_rtsp_message(reader: &mut BufReader<TcpStream>) -> Result<String, std::io::Error> {
        let mut raw = String::new();
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
            raw.push_str(normalized);
            raw.push_str("\r\n");

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
            raw.push_str(&String::from_utf8_lossy(&body));
        }

        Ok(raw)
    }

    #[test]
    fn timing_responder_replies_to_request_packet() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let request = [
            0x80, 0xd2, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8,
        ];

        let reply = build_timing_reply(&request).unwrap();
        socket
            .send_to(&request, socket.local_addr().unwrap())
            .unwrap();

        assert_eq!(reply.len(), 32);
    }
}
