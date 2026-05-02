mod connect;
pub mod group;
pub mod planner;
mod raop;
mod stream;
pub(crate) mod transport;

use std::fmt::Write;

use crate::audio::AudioFormat;
use crate::pairing::ReceiverCredentials;
use crate::receiver::Receiver;
use thiserror::Error;

pub use connect::{pair_receiver_with_pin, request_pairing_pin_display};
pub use planner::{PlannedTiming, PlannedTransport, SessionPlan, plan_session};
pub use raop::{RaopConnection, RaopSession, RaopSessionState};
pub use stream::{PlaybackSession, play_capture, play_file};
pub use transport::{
    ModernAirPlayConnection, ModernAirPlaySession, PreparedSession, SessionConnection,
};

/// 建立会话前需要的最小上下文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    pub device: Receiver,
    pub input_format: AudioFormat,
    pub frames_per_packet: usize,
    pub sender_volume_percent: u16,
    pub receiver_credentials: Option<ReceiverCredentials>,
}

impl SessionDescriptor {
    #[must_use]
    pub fn new(device: Receiver, input_format: AudioFormat) -> Self {
        Self {
            device,
            input_format,
            frames_per_packet: crate::audio::RAOP_FRAMES_PER_PACKET,
            sender_volume_percent: 100,
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
        self.input_format.block_align_bytes().map_err(|error| {
            AirPlayError::UnsupportedAudioFormat {
                message: error.to_string(),
            }
        })?;

        if self.frames_per_packet == 0 {
            return Err(AirPlayError::InvalidSession {
                message: String::from("frames per packet must be greater than 0"),
            });
        }

        if !(100..=400).contains(&self.sender_volume_percent) {
            return Err(AirPlayError::InvalidSession {
                message: String::from("sender volume percent must be between 100 and 400"),
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

/// AirPlay 会话阶段的错误集合。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AirPlayError {
    #[error("unsupported audio format: {message}")]
    UnsupportedAudioFormat { message: String },
    #[error(
        "target receiver requires authentication or pairing before session setup (common on macOS AirPlay Receiver / Apple TV / protected receivers)"
    )]
    AuthenticationRequired,
    #[error("AirPlay Receiver responded to control probing but pairing must be completed first")]
    PairingRequired,
    #[error("AirPlay Receiver requires usable credentials or a saved pairing record")]
    CredentialsMissing,
    #[error("AirPlay Receiver authentication failed: {message}")]
    AuthenticationFailed { message: String },
    #[error("RTSP protocol error: {message}")]
    Protocol { message: String },
    #[error("failed to connect to receiver: {message}")]
    ConnectionFailed { message: String },
    #[error("invalid session parameters: {message}")]
    InvalidSession { message: String },
    #[error("session is not ready for playback")]
    NotReady,
}

#[cfg(test)]
mod tests {
    use super::{AirPlayError, SessionDescriptor};
    use crate::audio::AudioFormat;
    use crate::audio::{
        CodecDescription, RAOP_FRAMES_PER_PACKET, RAOP_SAMPLE_RATE_HZ, RAOP_STARTUP_LATENCY_FRAMES,
        RAOP_STARTUP_LATENCY_MILLIS,
    };
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::session::RaopSession;

    fn build_device() -> Receiver {
        Receiver {
            id: String::from("speaker"),
            name: String::from("Speaker"),
            host: String::from("127.0.0.1"),
            port: 7000,
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
    fn session_descriptor_defaults_to_raop_packet_size() {
        let descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());

        assert_eq!(descriptor.frames_per_packet, RAOP_FRAMES_PER_PACKET);
        assert_eq!(descriptor.sender_volume_percent, 100);
    }

    #[test]
    fn session_descriptor_accepts_sender_volume_percent_up_to_400() {
        let mut descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());
        descriptor.sender_volume_percent = 400;

        descriptor.validate().unwrap();
        assert_eq!(descriptor.sender_volume_percent, 400);
    }

    #[test]
    fn session_descriptor_rejects_sender_volume_percent_below_100() {
        let mut descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());
        descriptor.sender_volume_percent = 99;

        assert!(matches!(
            descriptor.validate().unwrap_err(),
            AirPlayError::InvalidSession { .. }
        ));
    }

    #[test]
    fn session_descriptor_rejects_sender_volume_percent_above_400() {
        let mut descriptor = SessionDescriptor::new(build_device(), AudioFormat::default());
        descriptor.sender_volume_percent = 401;

        assert!(matches!(
            descriptor.validate().unwrap_err(),
            AirPlayError::InvalidSession { .. }
        ));
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
    fn startup_latency_baseline_matches_current_raop_profile() {
        assert_eq!(RAOP_STARTUP_LATENCY_FRAMES, 11_025);
        assert_eq!(RAOP_STARTUP_LATENCY_MILLIS, 250);
    }

    #[test]
    fn startup_latency_baseline_is_derived_from_frames_and_sample_rate() {
        assert_eq!(
            RAOP_STARTUP_LATENCY_MILLIS,
            RAOP_STARTUP_LATENCY_FRAMES * 1_000 / RAOP_SAMPLE_RATE_HZ
        );
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

        assert!(message.contains("authentication or pairing"));
        assert!(message.contains("macOS AirPlay Receiver"));
    }

    #[test]
    fn airplay_receiver_auth_errors_expose_distinct_messages() {
        assert!(
            AirPlayError::PairingRequired
                .to_string()
                .contains("pairing")
        );
        assert!(
            AirPlayError::CredentialsMissing
                .to_string()
                .contains("credentials")
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
