//! `AirPlay` / `RAOP` 会话与传输边界。

mod codec;
mod packet;
mod rtsp;
mod session;
mod sink;

use std::fmt::Write;

use crate::app::SpeakerDevice;
use crate::audio::AudioFormat;
use crate::config::ReceiverCredentials;
use thiserror::Error;

pub use codec::{AudioResampler, CodecDescription};
pub use packet::{RaopPacketCounters, RaopSyncPacket, RtpAudioPacket};
pub use rtsp::{RtspHeaders, RtspMethod, RtspRequest, RtspResponse, RtspStatus};
pub use session::{
    ModernAirPlayConnection, ModernAirPlaySession, PreparedConnection, PreparedTransportSession,
    RaopConnection, RaopSession, RaopSessionState, RaopStreamTransport,
};
pub use sink::{RaopAudioSink, RaopSinkConfig};

pub const RAOP_SAMPLE_RATE_HZ: u32 = 44_100;
pub const RAOP_CHANNELS: u16 = 2;
pub const RAOP_BITS_PER_SAMPLE: u16 = 16;
pub const RAOP_FRAMES_PER_PACKET: usize = 352;
pub const RAOP_STARTUP_LATENCY_FRAMES: u32 = 11_025;

/// 流建立前需要的最小会话信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    pub device: SpeakerDevice,
    pub input_format: AudioFormat,
    pub frames_per_packet: usize,
    pub receiver_credentials: Option<ReceiverCredentials>,
}

impl SessionDescriptor {
    #[must_use]
    pub fn new(device: SpeakerDevice, input_format: AudioFormat) -> Self {
        Self {
            device,
            input_format,
            frames_per_packet: RAOP_FRAMES_PER_PACKET,
            receiver_credentials: None,
        }
    }

    #[must_use]
    pub fn with_receiver_credentials(mut self, receiver_credentials: ReceiverCredentials) -> Self {
        self.receiver_credentials = Some(receiver_credentials);
        self
    }

    #[must_use]
    pub fn client_instance(&self) -> String {
        format_identifier(self.stable_identifier_seed("client-instance"))
    }

    #[must_use]
    pub fn client_device_id(&self) -> String {
        format_device_id(self.stable_identifier_seed("client-device-id"))
    }

    #[must_use]
    pub fn dacp_id(&self) -> String {
        format_identifier(self.stable_identifier_seed("dacp-id"))
    }

    #[must_use]
    pub fn active_remote(&self) -> String {
        let token = u32::try_from(self.stable_identifier_seed("active-remote") & 0x7fff_ffff_u64)
            .unwrap_or(1)
            .max(1);
        token.to_string()
    }

    #[must_use]
    pub fn stream_session_id(&self) -> String {
        self.stable_identifier_seed("stream-session").to_string()
    }

    pub fn validate(&self) -> Result<(), AirPlayError> {
        codec::validate_input_format(self.input_format)?;

        if self.frames_per_packet == 0 {
            return Err(AirPlayError::InvalidSession {
                message: String::from("每个包的帧数必须大于 0"),
            });
        }

        Ok(())
    }

    fn stable_identifier_seed(&self, purpose: &str) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        hash_identifier_segment(&mut hash, purpose.as_bytes());
        hash_identifier_segment(&mut hash, self.device.id.as_bytes());
        hash_identifier_segment(&mut hash, self.device.host.as_bytes());
        hash_identifier_segment(&mut hash, &self.device.port.to_be_bytes());
        hash
    }
}

fn hash_identifier_segment(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3_u64);
    }

    *hash ^= u64::from(b'|');
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3_u64);
}

fn format_identifier(value: u64) -> String {
    let mut identifier = String::with_capacity(16);
    let _ = write!(identifier, "{value:016X}");
    identifier
}

fn format_device_id(value: u64) -> String {
    let bytes = value.to_be_bytes();
    let mut identifier = String::with_capacity(17);
    for (index, byte) in bytes[2..].iter().enumerate() {
        if index > 0 {
            identifier.push(':');
        }
        let _ = write!(identifier, "{byte:02X}");
    }
    identifier
}

/// `AirPlay` 传输层错误。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AirPlayError {
    #[error("音频格式不受支持: {message}")]
    UnsupportedAudioFormat { message: String },
    #[error(
        "目标设备在会话建立前要求认证或配对（常见于 macOS AirPlay Receiver / Apple TV / 受保护接收端）"
    )]
    AuthenticationRequired,
    #[error("AirPlay Receiver 接收端已响应控制探测，但仍需要先完成配对流程")]
    PairingRequired,
    #[error("AirPlay Receiver 接收端需要可用认证凭据或配对记录")]
    CredentialsMissing,
    #[error("AirPlay Receiver 认证失败: {message}")]
    AuthenticationFailed { message: String },
    #[error("RTSP 协议错误: {message}")]
    Protocol { message: String },
    #[error("连接设备失败: {message}")]
    ConnectionFailed { message: String },
    #[error("会话参数无效: {message}")]
    InvalidSession { message: String },
    #[error("会话尚未进入可播放状态")]
    NotReady,
}

#[cfg(test)]
mod tests {
    use super::{
        AirPlayError, CodecDescription, RAOP_FRAMES_PER_PACKET, RaopSession, SessionDescriptor,
    };
    use crate::app::{AirPlayGeneration, DeviceSupport, ReceiverKind, SpeakerDevice};
    use crate::audio::AudioFormat;

    fn build_device() -> SpeakerDevice {
        SpeakerDevice {
            id: String::from("speaker"),
            name: String::from("Speaker"),
            host: String::from("127.0.0.1"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ClassicRaop,
            support: DeviceSupport::Supported,
        }
    }

    #[test]
    fn session_descriptor_defaults_to_raop_packet_size() {
        let descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());

        assert_eq!(descriptor.frames_per_packet, RAOP_FRAMES_PER_PACKET);
    }

    #[test]
    fn session_descriptor_builds_stable_client_instance() {
        let descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());

        assert_eq!(descriptor.client_instance().len(), 16);
        assert_eq!(descriptor.client_instance(), descriptor.client_instance());
    }

    #[test]
    fn session_descriptor_builds_distinct_remote_control_identifiers() {
        let descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());

        assert_eq!(descriptor.dacp_id().len(), 16);
        assert!(
            descriptor
                .dacp_id()
                .chars()
                .all(|ch| ch.is_ascii_hexdigit())
        );
        assert_ne!(descriptor.client_instance(), descriptor.dacp_id());
        assert!(
            descriptor
                .active_remote()
                .parse::<u32>()
                .is_ok_and(|value| value > 0)
        );
    }

    #[test]
    fn session_descriptor_builds_stable_stream_session_id() {
        let descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());

        assert_eq!(
            descriptor.stream_session_id(),
            descriptor.stream_session_id()
        );
        assert!(descriptor.stream_session_id().parse::<u64>().is_ok());
    }

    #[test]
    fn session_descriptor_rejects_zero_frames_per_packet() {
        let mut descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());
        descriptor.frames_per_packet = 0;

        assert!(matches!(
            descriptor.validate().unwrap_err(),
            AirPlayError::InvalidSession { .. }
        ));
    }

    #[test]
    fn raop_session_reports_transport_name() {
        assert_eq!(RaopSession::transport_name(), "raop");
    }

    #[test]
    fn codec_description_exposes_pcm_rtpmap() {
        let codec = CodecDescription::pcm_stereo();

        assert_eq!(codec.rtpmap, "L16/44100/2");
    }

    #[test]
    fn authentication_required_error_mentions_pairing_requirement() {
        let message = AirPlayError::AuthenticationRequired.to_string();

        assert!(message.contains("认证或配对"));
        assert!(message.contains("macOS AirPlay Receiver"));
    }

    #[test]
    fn airplay_receiver_auth_errors_expose_distinct_messages() {
        assert!(
            AirPlayError::PairingRequired
                .to_string()
                .contains("配对流程")
        );
        assert!(
            AirPlayError::CredentialsMissing
                .to_string()
                .contains("认证凭据")
        );
        assert!(
            AirPlayError::AuthenticationFailed {
                message: String::from("forbidden")
            }
            .to_string()
            .contains("forbidden")
        );
    }
}
