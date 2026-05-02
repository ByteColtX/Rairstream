use crate::audio::AudioFormat;
use crate::crypto::CipherSuite;
use crate::receiver::{CodecKind, Receiver, ReceiverKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedTransport {
    Raop,
    AirPlay2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    pub receiver_id: String,
    pub transport: PlannedTransport,
    pub codec: CipherSuite,
    pub input_format: AudioFormat,
}

pub fn plan_session(receiver: &Receiver, input_format: AudioFormat) -> SessionPlan {
    let transport = match receiver.receiver_kind {
        ReceiverKind::ClassicRaop => PlannedTransport::Raop,
        ReceiverKind::ModernAirPlayAuth => PlannedTransport::AirPlay2,
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
    };

    SessionPlan {
        receiver_id: receiver.id.clone(),
        transport,
        codec,
        input_format,
    }
}
