use crate::audio::AudioFormat;
use crate::crypto::CipherSuite;
use crate::receiver::{AuthMethod, CodecKind, Receiver, SupportLevel, TransportProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedTransport {
    Raop,
    AirPlay2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedTiming {
    Ntp,
    Ptp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    pub receiver_id: String,
    pub transport: PlannedTransport,
    pub codec: CipherSuite,
    pub input_format: AudioFormat,
    pub timing: PlannedTiming,
    pub auth_method: AuthMethod,
    pub support_level: SupportLevel,
}

#[must_use]
pub fn plan_session(receiver: &Receiver, input_format: AudioFormat) -> SessionPlan {
    let transport = match receiver.transport_profile {
        TransportProfile::Raop => PlannedTransport::Raop,
        TransportProfile::ModernAuthRaop => PlannedTransport::AirPlay2,
    };
    let codec = match receiver
        .capabilities
        .codecs
        .first()
        .copied()
        .unwrap_or(CodecKind::L16)
    {
        CodecKind::L16 => CipherSuite::L16,
        CodecKind::Alac => CipherSuite::Alac,
        CodecKind::Aac => CipherSuite::Aac,
        CodecKind::AacEld => CipherSuite::AacEld,
    };

    SessionPlan {
        receiver_id: receiver.id.clone(),
        transport,
        codec,
        input_format,
        timing: if receiver.capabilities.supports_ptp
            && receiver.transport_profile == TransportProfile::ModernAuthRaop
        {
            PlannedTiming::Ptp
        } else {
            PlannedTiming::Ntp
        },
        auth_method: receiver.auth_method,
        support_level: receiver.support_level.clone(),
    }
}
