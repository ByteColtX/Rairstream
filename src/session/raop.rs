//! classic `RAOP` 会话、握手与连接生命周期实现。

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AirPlayError, SessionDescriptor};
use crate::audio::CodecDescription;
use crate::rtsp::client::{
    compute_rtsp_keepalive_interval, ensure_success, format_response_status, map_connection_error,
};
use crate::rtsp::{
    RtspClient as SharedRtspClient, RtspKeepalive as SharedRtspKeepalive, RtspRequest,
    RtspResponse, SetupReply, SetupTransport, build_announce_request, build_options_request,
    build_record_request, build_setup_request, build_teardown_request, parse_setup_reply,
};
use crate::timing::raop::TimingResponder;
use crate::transport::{RaopPacketCounters, RaopSinkConfig};
use tracing::{debug, info};

/// `RAOP` 会话生命周期的最小状态集。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaopSessionState {
    Connecting,
    Prepared,
    Streaming,
}

/// 已完成握手的 `RAOP` 控制面连接。
#[derive(Debug)]
pub struct RaopConnection {
    session: RaopSession,
    audio_socket: UdpSocket,
    control_socket: UdpSocket,
    audio_target: SocketAddr,
    control_target: SocketAddr,
    timing_responder: Option<TimingResponder>,
    rtsp_keepalive: Option<SharedRtspKeepalive>,
    transport_error: Arc<Mutex<Option<AirPlayError>>>,
}

impl RaopConnection {
    #[must_use]
    pub fn session(&self) -> &RaopSession {
        &self.session
    }

    pub fn stream_transport(&self) -> Result<crate::transport::RaopStreamTransport, AirPlayError> {
        Ok(crate::transport::RaopStreamTransport {
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
        info!(endpoint = %self.session.descriptor().device.endpoint(), "sending TEARDOWN");
        self.stop_rtsp_keepalive(true)?;
        self.stop_timing_responder();
        debug!(endpoint = %self.session.descriptor().device.endpoint(), "TEARDOWN completed");
        Ok(())
    }

    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.transport_error().is_some()
    }

    #[must_use]
    pub fn transport_error(&self) -> Option<AirPlayError> {
        self.transport_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn stop_timing_responder(&mut self) {
        if let Some(mut timing_responder) = self.timing_responder.take() {
            timing_responder.stop();
        }
    }

    fn stop_rtsp_keepalive(&mut self, send_teardown: bool) -> Result<(), AirPlayError> {
        if let Some(mut rtsp_keepalive) = self.rtsp_keepalive.take() {
            rtsp_keepalive.stop(send_teardown)?;
        }
        Ok(())
    }
}

impl Drop for RaopConnection {
    fn drop(&mut self) {
        let _ = self.stop_rtsp_keepalive(false);
        self.stop_timing_responder();
    }
}

/// 经典 `RAOP` 会话骨架。
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
        let latency_profile = descriptor.latency_profile;
        let buffer_frames = latency_profile.buffer_frames();
        let (initial_sequence, initial_timestamp, audio_ssrc) = build_rtp_session_seed(descriptor);
        debug!(
            device_id = %descriptor.device.id,
            device_name = %descriptor.device.name,
            endpoint = %descriptor.device.endpoint(),
            sample_rate_hz = descriptor.input_format.sample_rate_hz,
            channels = descriptor.input_format.channels,
            bits_per_sample = descriptor.input_format.bits_per_sample,
            sample_type = ?descriptor.input_format.sample_type,
            frames_per_packet = descriptor.frames_per_packet,
            initial_sequence,
            initial_timestamp,
            audio_ssrc,
            requested_buffer_ms = latency_profile.buffer_ms(),
            requested_buffer_frames = buffer_frames,
            latency_profile = ?latency_profile.kind(),
            "creating RAOP session"
        );

        Ok(Self {
            descriptor: descriptor.clone(),
            sink_config: RaopSinkConfig {
                frames_per_packet: descriptor.frames_per_packet,
                sender_volume_percent: descriptor.sender_volume_percent,
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
    pub(crate) fn with_initial_cseq(mut self, cseq: u32) -> Self {
        self.cseq = cseq;
        self
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

    pub fn handshake_with_progress<F>(self, mut progress: F) -> Result<RaopConnection, AirPlayError>
    where
        F: FnMut(&str),
    {
        let endpoint = self.descriptor.device.endpoint();
        info!(endpoint = %endpoint, device_id = %self.descriptor.device.id, "starting RAOP handshake");
        progress(&format!("connecting RTSP control channel: {endpoint}"));
        let rtsp_client = SharedRtspClient::connect(&endpoint)?;
        self.handshake_with_rtsp_client_and_progress(rtsp_client, progress)
    }

    pub(crate) fn handshake_with_rtsp_client(
        self,
        rtsp_client: SharedRtspClient,
    ) -> Result<RaopConnection, AirPlayError> {
        self.handshake_with_rtsp_client_and_progress(rtsp_client, |_| ())
    }

    fn handshake_with_rtsp_client_and_progress<F>(
        mut self,
        mut rtsp_client: SharedRtspClient,
        mut progress: F,
    ) -> Result<RaopConnection, AirPlayError>
    where
        F: FnMut(&str),
    {
        let endpoint = self.descriptor.device.endpoint();
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
        debug!(
            endpoint = %endpoint,
            audio_port,
            control_port,
            timing_port,
            "local UDP ports bound"
        );
        progress(&format!(
            "local UDP ports bound: audio={audio_port}, control={control_port}, timing={timing_port}"
        ));

        progress("sending OPTIONS");
        let options_response = rtsp_client.send(&self.options_request())?;
        progress(&format_response_status("OPTIONS", &options_response));
        ensure_success(&options_response, "OPTIONS")?;

        progress("sending ANNOUNCE");
        let announce_response = rtsp_client.send(&self.announce_request())?;
        progress(&format_response_status("ANNOUNCE", &announce_response));
        ensure_success(&announce_response, "ANNOUNCE")?;

        let timing_responder = TimingResponder::start(timing_socket)?;
        progress("sending SETUP");
        let setup_response = rtsp_client.send(&self.setup_request(setup_transport))?;
        progress(&format_response_status("SETUP", &setup_response));
        self.apply_setup_response(&setup_response)?;

        let setup_reply = self.setup_reply()?.clone();
        debug!(
            endpoint = %endpoint,
            server_port = setup_reply.server_port,
            control_port = setup_reply.control_port,
            timing_port = setup_reply.timing_port,
            session_id = %setup_reply.session_id,
            "parsed UDP ports returned by receiver"
        );
        progress(&format!(
            "receiver UDP ports: audio={}, control={}, timing={}",
            setup_reply.server_port, setup_reply.control_port, setup_reply.timing_port
        ));

        progress("sending RECORD");
        let record_response = rtsp_client.send(&self.record_request()?)?;
        progress(&format_response_status("RECORD", &record_response));
        self.apply_record_response(&record_response)?;

        let keepalive_interval = compute_rtsp_keepalive_interval(setup_reply.session_timeout_secs);
        let transport_error = Arc::new(Mutex::new(None));
        let rtsp_keepalive = SharedRtspKeepalive::start(
            rtsp_client,
            self.descriptor.clone(),
            setup_reply.session_id.clone(),
            self.cseq,
            keepalive_interval,
            Arc::clone(&transport_error),
        )?;
        let audio_target =
            resolve_socket_addr(&self.descriptor.device.host, setup_reply.server_port)?;
        let control_target =
            resolve_socket_addr(&self.descriptor.device.host, setup_reply.control_port)?;
        debug!(audio_target = %audio_target, control_target = %control_target, "resolved remote audio/control endpoints");
        info!(endpoint = %endpoint, keepalive_secs = keepalive_interval.as_secs(), "RAOP session entered Streaming");
        progress("RAOP session entered Streaming");

        Ok(RaopConnection {
            session: self,
            audio_socket,
            control_socket,
            audio_target,
            control_target,
            timing_responder: Some(timing_responder),
            rtsp_keepalive: Some(rtsp_keepalive),
            transport_error,
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
        let setup_reply = parse_setup_reply(response)?;
        debug!(
            session_id = %setup_reply.session_id,
            server_port = setup_reply.server_port,
            control_port = setup_reply.control_port,
            timing_port = setup_reply.timing_port,
            "SETUP response parsed"
        );
        self.setup_reply = Some(setup_reply);
        self.state = RaopSessionState::Prepared;
        info!(state = ?self.state, "RAOP session state changed to Prepared");
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
        info!(state = ?self.state, "RAOP session state changed to Streaming");
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
    let initial_timestamp = apply_startup_latency_offset(
        u32::try_from((hash >> 16) & 0xffff_ffff_u64).unwrap_or_default(),
        descriptor.latency_profile.buffer_frames(),
    );
    let audio_ssrc = u32::try_from((hash >> 8) & 0xffff_ffff_u64)
        .unwrap_or(1)
        .max(1);

    (initial_sequence, initial_timestamp, audio_ssrc)
}

const fn apply_startup_latency_offset(base_timestamp: u32, startup_latency_frames: u32) -> u32 {
    base_timestamp.wrapping_add(startup_latency_frames)
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
            message: format!("failed to resolve receiver address {endpoint}"),
        })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream, UdpSocket};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::audio::RAOP_STARTUP_LATENCY_FRAMES;
    use crate::audio::{AudioFormat, AudioSampleType};
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::rtsp::{RtspResponse, SetupTransport};
    use crate::session::{AirPlayError, PreparedSession, SessionDescriptor};

    use super::{RaopSession, RaopSessionState};

    fn build_receiver(
        port: u16,
        receiver_kind: ReceiverKind,
        pairing_identity: Option<String>,
        receiver_public_key: Option<String>,
    ) -> Receiver {
        Receiver {
            id: String::from("speaker"),
            name: String::from("Speaker"),
            host: String::from("127.0.0.1"),
            port,
            generation: if receiver_kind.is_modern() {
                AirPlayGeneration::AirPlay2
            } else {
                AirPlayGeneration::AirPlay1
            },
            transport_profile: receiver_kind,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            pairing_identity,
            receiver_public_key,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields()
    }

    fn build_descriptor(format: AudioFormat) -> SessionDescriptor {
        SessionDescriptor::new(
            build_receiver(7000, ReceiverKind::ClassicRaop, None, None),
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
        assert_eq!(session.sink_config().sender_volume_percent, 100);
    }

    #[test]
    fn connect_preserves_sender_volume_percent_in_sink_config() {
        let mut descriptor = build_descriptor(AudioFormat::default());
        descriptor.sender_volume_percent = 400;

        let session = RaopSession::connect(&descriptor).unwrap();

        assert_eq!(session.sink_config().sender_volume_percent, 400);
    }

    #[test]
    fn connect_preserves_realtime_packet_size_in_sink_config() {
        let mut descriptor = build_descriptor(AudioFormat::default());
        descriptor.latency_profile = crate::session::LatencyProfile::realtime();
        descriptor.frames_per_packet = descriptor.latency_profile.frames_per_packet();

        let session = RaopSession::connect(&descriptor).unwrap();

        assert_eq!(session.sink_config().frames_per_packet, 128);
    }

    #[test]
    fn connect_rejects_out_of_range_sender_volume_percent() {
        let mut descriptor = build_descriptor(AudioFormat::default());
        descriptor.sender_volume_percent = 401;

        assert!(matches!(
            RaopSession::connect(&descriptor).unwrap_err(),
            AirPlayError::InvalidSession { .. }
        ));
    }

    #[test]
    fn prepared_session_uses_classic_raop_for_classic_receiver() {
        let descriptor = build_descriptor(AudioFormat::default());

        let prepared = PreparedSession::prepare(&descriptor).unwrap();

        match prepared {
            PreparedSession::ClassicRaop(session) => {
                assert_eq!(session.state(), RaopSessionState::Connecting);
            }
            PreparedSession::ModernAirPlay(_) => {
                panic!("classic receiver should use classic raop prepare path");
            }
        }
    }

    #[test]
    fn connect_seeds_non_zero_rtp_session_parameters() {
        let session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        let (sequence, timestamp) = session.packet_counters().peek_audio_packet();

        assert!(sequence > 0);
        assert!(session.audio_ssrc() > 0);
        assert!(timestamp >= RAOP_STARTUP_LATENCY_FRAMES);
    }

    #[test]
    fn connect_seeds_timestamp_with_startup_latency_baseline() {
        let session = RaopSession::connect(&build_descriptor(AudioFormat::default())).unwrap();
        let (_, timestamp) = session.packet_counters().peek_audio_packet();

        assert_eq!(
            timestamp.wrapping_sub(RAOP_STARTUP_LATENCY_FRAMES),
            timestamp - RAOP_STARTUP_LATENCY_FRAMES
        );
    }

    #[test]
    fn startup_latency_offset_adds_raop_baseline_to_timestamp_seed() {
        assert_eq!(
            super::apply_startup_latency_offset(0, RAOP_STARTUP_LATENCY_FRAMES),
            RAOP_STARTUP_LATENCY_FRAMES
        );
        assert_eq!(
            super::apply_startup_latency_offset(1_000, RAOP_STARTUP_LATENCY_FRAMES),
            1_000 + RAOP_STARTUP_LATENCY_FRAMES
        );
    }

    #[test]
    fn startup_latency_offset_preserves_timestamp_distance_from_baseline() {
        let baseline = super::apply_startup_latency_offset(0, RAOP_STARTUP_LATENCY_FRAMES);
        let advanced = super::apply_startup_latency_offset(12_345, RAOP_STARTUP_LATENCY_FRAMES);

        assert_eq!(advanced - baseline, 12_345);
    }

    #[test]
    fn startup_latency_offset_reaches_u32_max_at_exact_rollover_threshold() {
        assert_eq!(
            super::apply_startup_latency_offset(
                u32::MAX - RAOP_STARTUP_LATENCY_FRAMES,
                RAOP_STARTUP_LATENCY_FRAMES
            ),
            u32::MAX
        );
    }

    #[test]
    fn startup_latency_offset_wraps_to_zero_after_rollover_threshold() {
        assert_eq!(
            super::apply_startup_latency_offset(
                u32::MAX - RAOP_STARTUP_LATENCY_FRAMES + 1,
                RAOP_STARTUP_LATENCY_FRAMES
            ),
            0
        );
    }

    #[test]
    fn startup_latency_offset_wraps_at_u32_boundary() {
        assert_eq!(
            super::apply_startup_latency_offset(u32::MAX, RAOP_STARTUP_LATENCY_FRAMES),
            RAOP_STARTUP_LATENCY_FRAMES - 1
        );
    }

    #[test]
    fn startup_latency_offset_uses_requested_buffer_frames() {
        assert_eq!(super::apply_startup_latency_offset(1_000, 4_410), 5_410);
        assert_eq!(super::apply_startup_latency_offset(1_000, 0), 1_000);
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
            build_receiver(server.port(), ReceiverKind::ClassicRaop, None, None),
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
    fn handshake_starts_timing_responder_before_waiting_for_setup_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();

            let options_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(options_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let announce_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(announce_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let setup_request = read_rtsp_message(&mut reader).unwrap();
            let timing_port = setup_request
                .lines()
                .find_map(|line| line.strip_prefix("Transport: "))
                .and_then(|transport| {
                    transport.split(';').find_map(|part| {
                        let (name, value) = part.split_once('=')?;
                        (name.trim() == "timing_port").then_some(value.trim())
                    })
                })
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap();
            requests.push(setup_request);

            let control_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let timing_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            timing_socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let timing_request = [
                0x80, 0xd2, 0x12, 0x34, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
            ];
            timing_socket
                .send_to(&timing_request, ("127.0.0.1", timing_port))
                .unwrap();
            let mut timing_reply = [0_u8; 64];
            let (reply_len, _) = timing_socket.recv_from(&mut timing_reply).unwrap();
            assert_eq!(reply_len, 32);
            assert_eq!(timing_reply[1] & 0x7f, 83);

            let setup_response = format!(
                "RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5100;control_port={};timing_port={}\r\nSession: deadbeef;timeout=60\r\n\r\n",
                control_socket.local_addr().unwrap().port(),
                timing_socket.local_addr().unwrap().port(),
            );
            stream.write_all(setup_response.as_bytes()).unwrap();
            stream.flush().unwrap();

            let record_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(record_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let teardown_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(teardown_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nSession: deadbeef\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            tx.send(requests).unwrap();
        });

        let descriptor = SessionDescriptor::new(
            build_receiver(port, ReceiverKind::ClassicRaop, None, None),
            AudioFormat::default(),
        );

        let connection = RaopSession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap();
        connection.teardown().unwrap();

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests[2].starts_with("SETUP "));
        assert!(requests[3].starts_with("RECORD "));
        assert!(requests[4].starts_with("TEARDOWN "));
    }

    #[test]
    fn handshake_maps_authentication_failure() {
        let (server, _recorded_requests) =
            FakeRtspServer::spawn(vec![String::from("RTSP/1.0 401 Unauthorized\r\n\r\n")]);
        let descriptor = SessionDescriptor::new(
            build_receiver(server.port(), ReceiverKind::ClassicRaop, None, None),
            AudioFormat::default(),
        );

        let error = RaopSession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap_err();

        assert_eq!(error, AirPlayError::AuthenticationRequired);
    }

    struct FakeRtspServer {
        listener: TcpListener,
    }

    impl FakeRtspServer {
        fn spawn(responses: Vec<String>) -> (Self, mpsc::Receiver<Vec<String>>) {
            Self::spawn_bytes(responses.into_iter().map(String::into_bytes).collect())
        }

        fn spawn_bytes(responses: Vec<Vec<u8>>) -> (Self, mpsc::Receiver<Vec<String>>) {
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
                    stream.write_all(&response).unwrap();
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
