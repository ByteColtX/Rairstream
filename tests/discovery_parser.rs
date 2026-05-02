use std::net::Ipv4Addr;

use rairstream::discovery::testing::{
    MdnsTestResolvedService, MdnsTestServiceKind, parse_test_services,
};
use rairstream::receiver::{AirPlayGeneration, CodecKind, PairingRequirement, ReceiverKind};

fn build_service(
    service_kind: MdnsTestServiceKind,
    fullname: &str,
    port: u16,
    ipv4_addresses: Vec<Ipv4Addr>,
    device_id: Option<&str>,
) -> MdnsTestResolvedService {
    MdnsTestResolvedService {
        service_kind,
        fullname: fullname.to_string(),
        port,
        ipv4_addresses,
        device_id: device_id.map(str::to_string),
        pairing_id: None,
        model_or_am: None,
        features: None,
        flags: None,
        srcvers: None,
        receiver_public_key: None,
        system_pairing_identity: None,
        raop_codecs: None,
        raop_encryption_types: None,
        raop_transport: None,
        raop_metadata_types: None,
        group_public_name: None,
        group_id: None,
        home_group_id: None,
        household_id: None,
        parent_group_id: None,
    }
}

#[test]
fn parser_marks_homepod_style_receiver_as_modern() {
    let mut receiver = build_service(
        MdnsTestServiceKind::AirPlay,
        "Bedroom HomePod._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 26)],
        Some("00:11:22:33:44:55"),
    );
    receiver.model_or_am = Some(String::from("AudioAccessory5,1"));
    receiver.receiver_public_key = Some(String::from("abcdef"));

    let receivers = parse_test_services(vec![receiver]);

    assert_eq!(receivers.len(), 1);
    assert_eq!(receivers[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
    assert!(receivers[0].capabilities.codecs.contains(&CodecKind::Alac));
    assert!(receivers[0].capabilities.supports_multiroom);
}

#[test]
fn parser_marks_mac_receiver_without_public_key_as_modern() {
    let mut receiver = build_service(
        MdnsTestServiceKind::AirPlay,
        "ByteColt's MacBook Pro._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 40)],
        Some("BA:A5:C0:8E:8E:31"),
    );
    receiver.model_or_am = Some(String::from("Mac16,10"));
    receiver.pairing_id = Some(String::from("AA:BB:CC:DD:EE:FF"));
    receiver.features = Some(String::from("0x4A7FCFD5,0x38174FDE"));
    receiver.flags = Some(String::from("0x204"));
    receiver.srcvers = Some(String::from("940.23.1"));

    let receivers = parse_test_services(vec![receiver]);

    assert_eq!(receivers.len(), 1);
    assert_eq!(receivers[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
    assert_eq!(receivers[0].generation, AirPlayGeneration::AirPlay2);
    assert_eq!(
        receivers[0].capabilities.pairing,
        PairingRequirement::PinOrCredentials
    );
}

#[test]
fn parser_marks_secure_airplay_service_without_model_as_modern() {
    let mut receiver = build_service(
        MdnsTestServiceKind::AirPlay,
        "Secure Receiver._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 41)],
        Some("10:20:30:40:50:60"),
    );
    receiver.pairing_id = Some(String::from("AA:BB:CC:DD:EE:FF"));
    receiver.receiver_public_key = Some(String::from("abcdef"));

    let receivers = parse_test_services(vec![receiver]);

    assert_eq!(receivers.len(), 1);
    assert_eq!(receivers[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
    assert_eq!(
        receivers[0].capabilities.pairing,
        PairingRequirement::PinOrCredentials
    );
}

#[test]
fn parser_deduplicates_airplay_without_device_id_using_raop_identity() {
    let mut airplay = build_service(
        MdnsTestServiceKind::AirPlay,
        "Living Room._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 42)],
        None,
    );
    airplay.model_or_am = Some(String::from("AppleTV11,1"));
    airplay.pairing_id = Some(String::from("AA:BB:CC:DD:EE:FF"));
    airplay.features = Some(String::from("0x4A7FCFD5,0x38174FDE"));

    let receivers = parse_test_services(vec![
        build_service(
            MdnsTestServiceKind::Raop,
            "00:11:22:33:44:55@Living Room._raop._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 42)],
            None,
        ),
        airplay,
    ]);

    assert_eq!(receivers.len(), 1);
    assert_eq!(receivers[0].id, "001122334455");
    assert_eq!(receivers[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
}

#[test]
fn parser_keeps_legacy_apple_tv_classified_as_classic() {
    let mut receiver = build_service(
        MdnsTestServiceKind::AirPlay,
        "Apple TV._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 43)],
        Some("00:AA:BB:CC:DD:EE"),
    );
    receiver.model_or_am = Some(String::from("AppleTV3,2"));
    receiver.features = Some(String::from("0x5A7FFFF7,0xE"));
    receiver.flags = Some(String::from("0x4"));
    receiver.srcvers = Some(String::from("220.68"));

    let receivers = parse_test_services(vec![receiver]);

    assert_eq!(receivers.len(), 1);
    assert_eq!(receivers[0].receiver_kind, ReceiverKind::ClassicRaop);
}
