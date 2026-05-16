use rairstream::audio::{AudioFormat, SendCodec, SendCodecPreference};
use rairstream::receiver::{
    AirPlayGeneration, AuthMethod, CodecKind, DeviceSupport, PairingRequirement, Receiver,
    ReceiverCapabilities, ReceiverKind,
};
use rairstream::session::{PlannedTransport, plan_session, plan_session_with_codec_preference};

#[test]
fn planner_routes_modern_receiver_to_airplay2() {
    let receiver = Receiver {
        id: String::from("receiver-1"),
        name: String::from("Living Room"),
        host: String::from("192.168.1.10"),
        port: 7000,
        generation: AirPlayGeneration::AirPlay2,
        transport_profile: ReceiverKind::ModernAirPlayAuth,
        support_level: DeviceSupport::Supported,
        auth_method: AuthMethod::LegacyPin,
        pairing_identity: Some(String::from("pairing-id")),
        receiver_public_key: Some(String::from("public-key")),
        capabilities: ReceiverCapabilities {
            codecs: vec![CodecKind::Alac, CodecKind::Aac, CodecKind::L16],
            pairing: PairingRequirement::PinOrCredentials,
            supports_multiroom: true,
            supports_ptp: true,
            supports_retransmit: true,
        },
        ..Receiver::default()
    }
    .with_compat_fields();

    let plan = plan_session(&receiver, AudioFormat::default());

    assert_eq!(plan.transport, PlannedTransport::AirPlay2);
}

#[test]
fn planner_selects_alac_when_receiver_supports_it_in_auto_mode() {
    let receiver = Receiver {
        id: String::from("receiver-1"),
        name: String::from("Living Room"),
        host: String::from("192.168.1.10"),
        port: 7000,
        capabilities: ReceiverCapabilities {
            codecs: vec![CodecKind::Alac, CodecKind::L16],
            ..ReceiverCapabilities::default()
        },
        ..Receiver::default()
    }
    .with_compat_fields();

    let plan = plan_session(&receiver, AudioFormat::default());

    assert_eq!(plan.codec, SendCodec::Alac);
}

#[test]
fn planner_falls_back_to_pcm_when_alac_is_not_supported() {
    let receiver = Receiver {
        id: String::from("receiver-1"),
        name: String::from("Living Room"),
        host: String::from("192.168.1.10"),
        port: 7000,
        capabilities: ReceiverCapabilities {
            codecs: vec![CodecKind::L16],
            ..ReceiverCapabilities::default()
        },
        ..Receiver::default()
    }
    .with_compat_fields();

    let plan = plan_session_with_codec_preference(
        &receiver,
        AudioFormat::default(),
        SendCodecPreference::Alac,
    );

    assert_eq!(plan.codec, SendCodec::PcmL16);
}

#[test]
fn planner_allows_forcing_pcm_even_when_alac_is_supported() {
    let receiver = Receiver {
        id: String::from("receiver-1"),
        name: String::from("Living Room"),
        host: String::from("192.168.1.10"),
        port: 7000,
        capabilities: ReceiverCapabilities {
            codecs: vec![CodecKind::Alac, CodecKind::L16],
            ..ReceiverCapabilities::default()
        },
        ..Receiver::default()
    }
    .with_compat_fields();

    let plan = plan_session_with_codec_preference(
        &receiver,
        AudioFormat::default(),
        SendCodecPreference::PcmL16,
    );

    assert_eq!(plan.codec, SendCodec::PcmL16);
}
