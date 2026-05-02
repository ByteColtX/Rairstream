//! `RTSP` 报文模型、编解码与握手请求构造。

use std::fmt::Write;
use std::net::UdpSocket;

use super::{AirPlayError, CodecDescription, RAOP_STARTUP_LATENCY_FRAMES, SessionDescriptor};

/// `RTSP` 请求方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspMethod {
    Get,
    Post,
    Options,
    Announce,
    Setup,
    Record,
    Teardown,
}

impl RtspMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Options => "OPTIONS",
            Self::Announce => "ANNOUNCE",
            Self::Setup => "SETUP",
            Self::Record => "RECORD",
            Self::Teardown => "TEARDOWN",
        }
    }
}

/// `RTSP` 头列表，先保留顺序，便于后续按报文顺序输出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RtspHeaders(Vec<(String, String)>);

impl RtspHeaders {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push((name.into(), value.into()));
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[(String, String)] {
        &self.0
    }
}

/// 最小 `RTSP` 请求模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtspRequest {
    pub method: RtspMethod,
    pub uri: String,
    pub headers: RtspHeaders,
    pub body: Vec<u8>,
}

impl RtspRequest {
    #[must_use]
    pub fn new(method: RtspMethod, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            headers: RtspHeaders::new(),
            body: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name, value);
        self
    }

    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    #[must_use]
    pub fn body_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut request = format!("{} {} RTSP/1.0\r\n", self.method.as_str(), self.uri);

        for (name, value) in self.headers.as_slice() {
            let _ = write!(request, "{name}: {value}\r\n");
        }

        request.push_str("\r\n");
        let mut encoded = request.into_bytes();
        encoded.extend_from_slice(&self.body);
        encoded
    }
}

/// `RTSP` 状态行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtspStatus {
    pub code: u16,
    pub reason_phrase: String,
}

/// 最小 `RTSP` 响应模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtspResponse {
    pub status: RtspStatus,
    pub headers: RtspHeaders,
    pub body: Vec<u8>,
}

impl RtspResponse {
    #[must_use]
    pub fn success(code: u16) -> Self {
        Self {
            status: RtspStatus {
                code,
                reason_phrase: String::from("OK"),
            },
            headers: RtspHeaders::new(),
            body: Vec::new(),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, AirPlayError> {
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
        Self::parse_parts(head, body.as_bytes().to_vec())
    }

    pub fn parse_parts(head: &str, body: Vec<u8>) -> Result<Self, AirPlayError> {
        let mut lines = head.split("\r\n");
        let status_line = lines.next().ok_or_else(|| AirPlayError::Protocol {
            message: String::from("missing RTSP status line"),
        })?;
        let mut status_parts = status_line.splitn(3, ' ');
        let _version = status_parts.next().ok_or_else(|| AirPlayError::Protocol {
            message: String::from("RTSP status line is missing the version"),
        })?;
        let code = status_parts
            .next()
            .ok_or_else(|| AirPlayError::Protocol {
                message: String::from("RTSP status line is missing the status code"),
            })?
            .parse::<u16>()
            .map_err(|_| AirPlayError::Protocol {
                message: format!("invalid RTSP status code: {status_line}"),
            })?;
        let reason_phrase = String::from(status_parts.next().unwrap_or(""));

        let mut headers = RtspHeaders::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }

            let Some((name, value)) = line.split_once(':') else {
                return Err(AirPlayError::Protocol {
                    message: format!("invalid RTSP header line: {line}"),
                });
            };
            headers.insert(name.trim(), value.trim());
        }

        Ok(Self {
            status: RtspStatus {
                code,
                reason_phrase,
            },
            headers,
            body,
        })
    }

    #[must_use]
    pub fn body_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status.code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupTransport {
    pub control_port: u16,
    pub timing_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupReply {
    pub session_id: String,
    pub session_timeout_secs: Option<u64>,
    pub server_port: u16,
    pub control_port: u16,
    pub timing_port: u16,
}

#[must_use]
pub fn build_session_uri(descriptor: &SessionDescriptor) -> String {
    format!(
        "rtsp://{}/{}",
        descriptor.device.endpoint(),
        descriptor.stream_session_id()
    )
}

#[must_use]
pub fn build_options_request(descriptor: &SessionDescriptor, cseq: u32) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Options, "*").with_header("CSeq", cseq.to_string()),
    )
}

#[must_use]
pub fn build_info_request(descriptor: &SessionDescriptor, cseq: u32) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Get, "/info")
            .with_header("CSeq", cseq.to_string())
            .with_header("Content-Length", "0"),
    )
}

#[must_use]
pub fn build_pair_setup_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    content_type: &str,
    body: impl Into<Vec<u8>>,
) -> RtspRequest {
    build_post_request(descriptor, cseq, "/pair-setup", content_type, body)
}

#[must_use]
pub fn build_pair_pin_start_request(descriptor: &SessionDescriptor, cseq: u32) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Post, "/pair-pin-start")
            .with_header("CSeq", cseq.to_string())
            .with_header("Content-Length", "0"),
    )
}

#[must_use]
pub fn build_pair_setup_pin_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    body: impl Into<Vec<u8>>,
) -> RtspRequest {
    build_post_request(
        descriptor,
        cseq,
        "/pair-setup-pin",
        "application/x-apple-binary-plist",
        body,
    )
}

#[must_use]
pub fn build_pair_verify_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    content_type: &str,
    body: impl Into<Vec<u8>>,
) -> RtspRequest {
    build_post_request(descriptor, cseq, "/pair-verify", content_type, body)
}

#[must_use]
pub fn build_auth_setup_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    body: impl Into<Vec<u8>>,
) -> RtspRequest {
    let body = body.into();
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Post, "/auth-setup")
            .with_header("CSeq", cseq.to_string())
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Content-Length", body.len().to_string())
            .with_body(body),
    )
}

#[must_use]
pub fn build_announce_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    codec: &CodecDescription,
) -> RtspRequest {
    let body = build_pcm_sdp(descriptor, codec);
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Announce, build_session_uri(descriptor))
            .with_header("CSeq", cseq.to_string())
            .with_header("Content-Type", "application/sdp")
            .with_header("Content-Length", body.len().to_string())
            .with_body(body),
    )
}

#[must_use]
pub fn build_setup_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    transport: SetupTransport,
) -> RtspRequest {
    let transport_header = format!(
        "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={};timing_port={}",
        transport.control_port, transport.timing_port
    );
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Setup, build_session_uri(descriptor))
            .with_header("CSeq", cseq.to_string())
            .with_header("Transport", transport_header),
    )
}

#[must_use]
pub fn build_record_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    session_id: &str,
    sequence: u16,
    rtp_timestamp: u32,
) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Record, build_session_uri(descriptor))
            .with_header("CSeq", cseq.to_string())
            .with_header("Session", session_id)
            .with_header("Range", "npt=0-")
            .with_header(
                "RTP-Info",
                format!("seq={sequence};rtptime={rtp_timestamp}"),
            ),
    )
}

#[must_use]
pub fn build_teardown_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    session_id: &str,
) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Teardown, build_session_uri(descriptor))
            .with_header("CSeq", cseq.to_string())
            .with_header("Session", session_id),
    )
}

#[must_use]
pub fn build_keepalive_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    session_id: &str,
) -> RtspRequest {
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Options, "*")
            .with_header("CSeq", cseq.to_string())
            .with_header("Session", session_id),
    )
}

pub fn parse_setup_reply(response: &RtspResponse) -> Result<SetupReply, AirPlayError> {
    if !response.is_success() {
        return Err(AirPlayError::Protocol {
            message: format!("SETUP returned failure status {}", response.status.code),
        });
    }

    let session_header = response
        .headers
        .get("Session")
        .ok_or_else(|| AirPlayError::Protocol {
            message: String::from("SETUP response is missing the Session header"),
        })?;
    let session_id = session_header
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();

    if session_id.is_empty() {
        return Err(AirPlayError::Protocol {
            message: String::from("SETUP response contained an empty Session header"),
        });
    }

    let transport = response
        .headers
        .get("Transport")
        .ok_or_else(|| AirPlayError::Protocol {
            message: String::from("SETUP response is missing the Transport header"),
        })?;

    Ok(SetupReply {
        session_id,
        session_timeout_secs: parse_session_timeout_secs(session_header),
        server_port: parse_transport_port(transport, "server_port")?,
        control_port: parse_transport_port(transport, "control_port")?,
        timing_port: parse_transport_port(transport, "timing_port")?,
    })
}

fn build_pcm_sdp(descriptor: &SessionDescriptor, codec: &CodecDescription) -> String {
    let sender_ip = resolve_sender_ip(&descriptor.device.host, descriptor.device.port)
        .unwrap_or_else(|| String::from("0.0.0.0"));

    format!(
        "v=0\r\no=Rairstream {} 0 IN IP4 {}\r\ns=Rairstream\r\nc=IN IP4 {}\r\nt=0 0\r\nm=audio 0 RTP/AVP 96\r\na=rtpmap:96 {}\r\na=min-latency:{}\r\n",
        descriptor.stream_session_id(),
        sender_ip,
        sender_ip,
        codec.rtpmap,
        RAOP_STARTUP_LATENCY_FRAMES
    )
}

fn resolve_sender_ip(receiver_host: &str, receiver_port: u16) -> Option<String> {
    let probe_socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    probe_socket.connect((receiver_host, receiver_port)).ok()?;
    let local_addr = probe_socket.local_addr().ok()?;
    Some(local_addr.ip().to_string())
}

fn build_post_request(
    descriptor: &SessionDescriptor,
    cseq: u32,
    uri: &str,
    content_type: &str,
    body: impl Into<Vec<u8>>,
) -> RtspRequest {
    let body = body.into();
    apply_common_headers(
        descriptor,
        RtspRequest::new(RtspMethod::Post, uri)
            .with_header("CSeq", cseq.to_string())
            .with_header("Content-Type", content_type)
            .with_header("Content-Length", body.len().to_string())
            .with_body(body),
    )
}

fn apply_common_headers(descriptor: &SessionDescriptor, request: RtspRequest) -> RtspRequest {
    request
        .with_header("User-Agent", "Rairstream/0.1")
        .with_header("Client-Instance", descriptor.client_instance())
        .with_header("DACP-ID", descriptor.dacp_id())
        .with_header("Active-Remote", descriptor.active_remote())
}

fn parse_transport_port(transport: &str, field_name: &str) -> Result<u16, AirPlayError> {
    let value = transport
        .split(';')
        .find_map(|part| {
            let (name, value) = part.split_once('=')?;
            (name.trim() == field_name).then_some(value.trim())
        })
        .ok_or_else(|| AirPlayError::Protocol {
            message: format!("Transport header is missing {field_name}"),
        })?;

    value.parse::<u16>().map_err(|_| AirPlayError::Protocol {
        message: format!("Transport header field {field_name} is not a valid port: {value}"),
    })
}

fn parse_session_timeout_secs(session_header: &str) -> Option<u64> {
    session_header.split(';').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name.trim().eq_ignore_ascii_case("timeout"))
            .then(|| value.trim())?
            .parse::<u64>()
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        RtspHeaders, RtspMethod, RtspRequest, RtspResponse, SetupTransport, build_announce_request,
        build_auth_setup_request, build_info_request, build_keepalive_request,
        build_options_request, build_pair_setup_request, build_pair_verify_request,
        build_record_request, build_session_uri, build_setup_request, build_teardown_request,
        parse_setup_reply,
    };
    use crate::audio::AudioFormat;
    use crate::receiver::{
        AirPlayGeneration, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::transport::{
        AirPlayError, CodecDescription, RAOP_STARTUP_LATENCY_FRAMES, RAOP_STARTUP_LATENCY_MILLIS,
        SessionDescriptor,
    };

    fn build_descriptor() -> SessionDescriptor {
        SessionDescriptor::new(
            Receiver {
                id: String::from("speaker-id"),
                name: String::from("Speaker"),
                host: String::from("speaker.local"),
                port: 7000,
                generation: AirPlayGeneration::AirPlay1,
                pairing_id: None,
                receiver_public_key: None,
                receiver_kind: ReceiverKind::ClassicRaop,
                support: DeviceSupport::Supported,
                capabilities: ReceiverCapabilities::default(),
            },
            AudioFormat::default(),
        )
    }

    #[test]
    fn request_builder_keeps_method_uri_and_headers() {
        let request = RtspRequest::new(RtspMethod::Announce, "rtsp://speaker.local/1")
            .with_header("CSeq", "1")
            .with_body("v=0");

        assert_eq!(request.method.as_str(), "ANNOUNCE");
        assert_eq!(request.uri, "rtsp://speaker.local/1");
        assert_eq!(request.headers.get("cseq"), Some("1"));
        assert_eq!(request.body, b"v=0");
    }

    #[test]
    fn request_encode_emits_rtsp_wire_format() {
        let request = RtspRequest::new(RtspMethod::Options, "*").with_header("CSeq", "1");

        assert_eq!(request.encode(), b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\n");
    }

    #[test]
    fn headers_lookup_is_case_insensitive() {
        let mut headers = RtspHeaders::new();
        headers.insert("Session", "abc");

        assert_eq!(headers.get("session"), Some("abc"));
    }

    #[test]
    fn success_response_reports_success_status() {
        let response = RtspResponse::success(200);

        assert!(response.is_success());
        assert_eq!(response.status.reason_phrase, "OK");
    }

    #[test]
    fn response_parser_extracts_status_headers_and_body() {
        let response =
            RtspResponse::parse("RTSP/1.0 200 OK\r\nSession: 1\r\nCSeq: 2\r\n\r\nbody").unwrap();

        assert_eq!(response.status.code, 200);
        assert_eq!(response.headers.get("session"), Some("1"));
        assert_eq!(response.body, b"body");
    }

    #[test]
    fn build_session_uri_targets_device_endpoint_and_stream_session_id() {
        let descriptor = build_descriptor();

        assert_eq!(
            build_session_uri(&descriptor),
            format!(
                "rtsp://speaker.local:7000/{}",
                descriptor.stream_session_id()
            )
        );
    }

    #[test]
    fn options_request_contains_cseq() {
        let descriptor = build_descriptor();
        let request = build_options_request(&descriptor, 7);

        assert_eq!(request.method, RtspMethod::Options);
        assert_eq!(request.uri, "*");
        assert_eq!(request.headers.get("CSeq"), Some("7"));
        assert_eq!(request.headers.get("User-Agent"), Some("Rairstream/0.1"));
        assert_eq!(
            request.headers.get("Client-Instance"),
            Some(descriptor.client_instance().as_str())
        );
        assert_eq!(
            request.headers.get("DACP-ID"),
            Some(descriptor.dacp_id().as_str())
        );
        assert_eq!(
            request.headers.get("Active-Remote"),
            Some(descriptor.active_remote().as_str())
        );
    }

    #[test]
    fn info_request_targets_modern_info_endpoint() {
        let descriptor = build_descriptor();
        let request = build_info_request(&descriptor, 12);

        assert_eq!(request.method, RtspMethod::Get);
        assert_eq!(request.uri, "/info");
        assert_eq!(request.headers.get("CSeq"), Some("12"));
        assert_eq!(request.headers.get("Content-Length"), Some("0"));
    }

    #[test]
    fn pair_requests_attach_binary_body_and_content_headers() {
        let descriptor = build_descriptor();
        let pair_setup =
            build_pair_setup_request(&descriptor, 13, "application/octet-stream", [1_u8, 2, 3]);
        let pair_verify =
            build_pair_verify_request(&descriptor, 14, "application/octet-stream", [4_u8, 5]);
        let auth_setup = build_auth_setup_request(&descriptor, 15, [6_u8; 33]);

        assert_eq!(pair_setup.method, RtspMethod::Post);
        assert_eq!(pair_setup.uri, "/pair-setup");
        assert_eq!(
            pair_setup.headers.get("Content-Type"),
            Some("application/octet-stream")
        );
        assert_eq!(pair_setup.headers.get("Content-Length"), Some("3"));
        assert_eq!(pair_setup.body, [1_u8, 2, 3]);

        assert_eq!(pair_verify.method, RtspMethod::Post);
        assert_eq!(pair_verify.uri, "/pair-verify");
        assert_eq!(pair_verify.headers.get("Content-Length"), Some("2"));
        assert_eq!(pair_verify.body, [4_u8, 5]);

        assert_eq!(auth_setup.method, RtspMethod::Post);
        assert_eq!(auth_setup.uri, "/auth-setup");
        assert_eq!(auth_setup.headers.get("Content-Length"), Some("33"));
        assert_eq!(
            auth_setup.headers.get("Content-Type"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn announce_request_contains_pcm_sdp() {
        let descriptor = build_descriptor();
        let request = build_announce_request(&descriptor, 8, &CodecDescription::pcm_stereo());
        let body = request.body_text().unwrap();

        assert_eq!(request.method, RtspMethod::Announce);
        assert_eq!(
            request.uri,
            format!(
                "rtsp://speaker.local:7000/{}",
                descriptor.stream_session_id()
            )
        );
        assert_eq!(request.headers.get("Content-Type"), Some("application/sdp"));
        assert_eq!(
            request.headers.get("DACP-ID"),
            Some(descriptor.dacp_id().as_str())
        );
        assert_eq!(
            request.headers.get("Active-Remote"),
            Some(descriptor.active_remote().as_str())
        );
        assert!(body.contains(&format!(
            "o=Rairstream {} 0 IN IP4",
            descriptor.stream_session_id()
        )));
        let sender_ip = super::resolve_sender_ip(&descriptor.device.host, descriptor.device.port)
            .unwrap_or_else(|| String::from("0.0.0.0"));
        assert!(body.contains(&format!("c=IN IP4 {sender_ip}")));
        assert!(!body.contains("c=IN IP4 speaker.local"));
        assert!(body.contains("a=rtpmap:96 L16/44100/2"));
        assert!(body.contains(&format!("a=min-latency:{RAOP_STARTUP_LATENCY_FRAMES}")));
        assert!(body.contains("m=audio 0 RTP/AVP 96"));
    }

    #[test]
    fn announce_request_min_latency_matches_current_baseline_millis() {
        assert_eq!(RAOP_STARTUP_LATENCY_MILLIS, 250);
        assert_eq!(RAOP_STARTUP_LATENCY_FRAMES, 11_025);
        assert_eq!(
            RAOP_STARTUP_LATENCY_FRAMES,
            RAOP_STARTUP_LATENCY_MILLIS * 44_100 / 1_000
        );
    }

    #[test]
    fn setup_request_contains_transport_header() {
        let descriptor = build_descriptor();
        let request = build_setup_request(
            &descriptor,
            9,
            SetupTransport {
                control_port: 6001,
                timing_port: 6002,
            },
        );

        assert_eq!(request.method, RtspMethod::Setup);
        assert_eq!(
            request.headers.get("DACP-ID"),
            Some(descriptor.dacp_id().as_str())
        );
        assert_eq!(
            request.headers.get("Active-Remote"),
            Some(descriptor.active_remote().as_str())
        );
        assert_eq!(
            request.headers.get("Transport"),
            Some(
                "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port=6001;timing_port=6002"
            )
        );
    }

    #[test]
    fn record_request_contains_session_and_rtp_info() {
        let descriptor = build_descriptor();
        let request = build_record_request(&descriptor, 10, "abc", 11, 12);

        assert_eq!(request.method, RtspMethod::Record);
        assert_eq!(request.headers.get("Session"), Some("abc"));
        assert_eq!(
            request.headers.get("DACP-ID"),
            Some(descriptor.dacp_id().as_str())
        );
        assert_eq!(
            request.headers.get("Active-Remote"),
            Some(descriptor.active_remote().as_str())
        );
        assert_eq!(request.headers.get("RTP-Info"), Some("seq=11;rtptime=12"));
    }

    #[test]
    fn teardown_request_contains_session() {
        let descriptor = build_descriptor();
        let request = build_teardown_request(&descriptor, 11, "abc");

        assert_eq!(request.method, RtspMethod::Teardown);
        assert_eq!(request.headers.get("Session"), Some("abc"));
        assert_eq!(
            request.headers.get("DACP-ID"),
            Some(descriptor.dacp_id().as_str())
        );
        assert_eq!(
            request.headers.get("Active-Remote"),
            Some(descriptor.active_remote().as_str())
        );
    }

    #[test]
    fn parse_setup_reply_extracts_ports_and_session() {
        let response = RtspResponse::parse(
            "RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5000;control_port=5001;timing_port=5002\r\nSession: deadbeef;timeout=60\r\n\r\n",
        )
        .unwrap();

        let reply = parse_setup_reply(&response).unwrap();

        assert_eq!(reply.session_id, "deadbeef");
        assert_eq!(reply.session_timeout_secs, Some(60));
        assert_eq!(reply.server_port, 5000);
        assert_eq!(reply.control_port, 5001);
        assert_eq!(reply.timing_port, 5002);
    }

    #[test]
    fn keepalive_request_uses_options_star_and_session() {
        let descriptor = build_descriptor();
        let request = build_keepalive_request(&descriptor, 12, "abc");

        assert_eq!(request.method, RtspMethod::Options);
        assert_eq!(request.uri, "*");
        assert_eq!(request.headers.get("CSeq"), Some("12"));
        assert_eq!(request.headers.get("Session"), Some("abc"));
    }

    #[test]
    fn parse_setup_reply_rejects_missing_transport() {
        let response = RtspResponse::parse("RTSP/1.0 200 OK\r\nSession: 1\r\n\r\n").unwrap();
        let error = parse_setup_reply(&response).unwrap_err();

        assert!(matches!(error, AirPlayError::Protocol { .. }));
    }
}
