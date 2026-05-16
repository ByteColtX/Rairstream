use crate::audio::{AudioFormat, SendCodec, SendCodecPreference};
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
    pub codec: SendCodec,
    pub input_format: AudioFormat,
    pub timing: PlannedTiming,
    pub auth_method: AuthMethod,
    pub support_level: SupportLevel,
}

#[must_use]
pub fn plan_session(receiver: &Receiver, input_format: AudioFormat) -> SessionPlan {
    plan_session_with_codec_preference(receiver, input_format, SendCodecPreference::Auto)
}

#[must_use]
pub fn plan_session_with_codec_preference(
    receiver: &Receiver,
    input_format: AudioFormat,
    codec_preference: SendCodecPreference,
) -> SessionPlan {
    let transport = match receiver.transport_profile {
        TransportProfile::Raop => PlannedTransport::Raop,
        TransportProfile::ModernAuthRaop => PlannedTransport::AirPlay2,
    };
    let codec = select_send_codec(receiver, codec_preference);

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

#[must_use]
pub fn select_send_codec(receiver: &Receiver, codec_preference: SendCodecPreference) -> SendCodec {
    let supports_alac = receiver.capabilities.codecs.contains(&CodecKind::Alac);
    if codec_preference != SendCodecPreference::PcmL16 && supports_alac {
        SendCodec::Alac
    } else {
        SendCodec::PcmL16
    }
}
