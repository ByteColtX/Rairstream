use rairstream::audio::AudioFormat;
use rairstream::receiver::{
    AirPlayGeneration, CodecKind, DeviceSupport, PairingRequirement, Receiver,
    ReceiverCapabilities, ReceiverKind,
};
use rairstream::session::{PlannedTransport, plan_session};

#[test]
fn planner_routes_modern_receiver_to_airplay2() {
    let receiver = Receiver {
        id: String::from("receiver-1"),
        name: String::from("Living Room"),
        host: String::from("192.168.1.10"),
        port: 7000,
        generation: AirPlayGeneration::AirPlay2,
        pairing_id: Some(String::from("pairing-id")),
        receiver_public_key: Some(String::from("public-key")),
        receiver_kind: ReceiverKind::ModernAirPlayAuth,
        support: DeviceSupport::Supported,
        capabilities: ReceiverCapabilities {
            codecs: vec![CodecKind::Alac, CodecKind::Aac, CodecKind::L16],
            pairing: PairingRequirement::PinOrCredentials,
            supports_multiroom: true,
            supports_ptp: true,
            supports_retransmit: true,
        },
    };

    let plan = plan_session(&receiver, AudioFormat::default());

    assert_eq!(plan.transport, PlannedTransport::AirPlay2);
}
