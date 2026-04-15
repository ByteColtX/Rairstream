use std::net::Ipv4Addr;

use rairstream::app::{DeviceSupport, ReceiverKind};
use rairstream::discovery::testing::{
    MdnsTestResolvedService, MdnsTestServiceKind, parse_test_services,
};

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
    }
}

#[test]
fn test_parse_raop_service_extracts_normalized_device_id_and_name() {
    let devices = parse_test_services(vec![build_service(
        MdnsTestServiceKind::Raop,
        "00:11:22:33:44:55@Living Room._raop._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 25)],
        None,
    )]);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "001122334455");
    assert_eq!(devices[0].name, "Living Room");
    assert_eq!(devices[0].host, "192.168.12.25");
}

#[test]
fn test_parse_airplay_service_uses_device_id_when_available() {
    let devices = parse_test_services(vec![build_service(
        MdnsTestServiceKind::AirPlay,
        "Bedroom Speaker._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 26)],
        Some("00:11:22:33:44:55"),
    )]);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "001122334455");
    assert_eq!(devices[0].name, "Bedroom Speaker");
}

#[test]
fn test_parse_airplay_service_falls_back_to_endpoint_when_device_id_missing() {
    let devices = parse_test_services(vec![build_service(
        MdnsTestServiceKind::AirPlay,
        "Kitchen Speaker._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 30)],
        None,
    )]);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "192.168.12.30:7000");
    assert_eq!(devices[0].name, "Kitchen Speaker");
}

#[test]
fn test_parse_services_filter_invalid_entries() {
    let devices = parse_test_services(vec![
        build_service(
            MdnsTestServiceKind::Raop,
            "001122334455@Living Room._raop._tcp.local.",
            0,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            None,
        ),
        build_service(
            MdnsTestServiceKind::AirPlay,
            "Bedroom Speaker._airplay._tcp.local.",
            7000,
            Vec::new(),
            Some("00:11:22:33:44:55"),
        ),
        build_service(
            MdnsTestServiceKind::Raop,
            "001122334455@._raop._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            None,
        ),
    ]);

    assert!(devices.is_empty());
}

#[test]
fn test_parse_services_deduplicate_across_raop_and_airplay() {
    let devices = parse_test_services(vec![
        build_service(
            MdnsTestServiceKind::Raop,
            "001122334455@Living Room._raop._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            None,
        ),
        build_service(
            MdnsTestServiceKind::AirPlay,
            "Living Room._airplay._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 26)],
            Some("00:11:22:33:44:55"),
        ),
    ]);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].host, "192.168.12.26");
}

#[test]
fn test_parse_services_marks_modern_receiver_as_unsupported() {
    let mut modern_receiver = build_service(
        MdnsTestServiceKind::AirPlay,
        "ByteColt's Appleseed._airplay._tcp.local.",
        7000,
        vec![Ipv4Addr::new(192, 168, 12, 25)],
        Some("BA:A5:C0:8E:8E:31"),
    );
    modern_receiver.model_or_am = Some(String::from("Mac16,10"));
    modern_receiver.features = Some(String::from("0x4A7FCFD5,0x38174FDE"));
    modern_receiver.flags = Some(String::from("0x204"));
    modern_receiver.srcvers = Some(String::from("940.23.1"));
    modern_receiver.receiver_public_key = Some(String::from("abcdef"));

    let devices = parse_test_services(vec![modern_receiver]);

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
    assert_eq!(devices[0].support, DeviceSupport::Supported);
}
