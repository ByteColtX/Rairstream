use std::collections::HashMap;
use std::net::Ipv4Addr;

use crate::app::{AirPlayGeneration, DeviceSupport, ReceiverKind, SpeakerDevice};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MdnsServiceKind {
    Raop,
    AirPlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMdnsService {
    pub(crate) service_kind: MdnsServiceKind,
    pub(crate) fullname: String,
    pub(crate) port: u16,
    pub(crate) ipv4_addresses: Vec<Ipv4Addr>,
    pub(crate) device_id: Option<String>,
    pub(crate) pairing_id: Option<String>,
    pub(crate) model_or_am: Option<String>,
    pub(crate) features: Option<String>,
    pub(crate) flags: Option<String>,
    pub(crate) srcvers: Option<String>,
    pub(crate) receiver_public_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDeviceCandidate {
    id: String,
    name: String,
    host: String,
    port: u16,
    generation: AirPlayGeneration,
    has_airplay_service: bool,
    pairing_id: Option<String>,
    model_or_am: Option<String>,
    features: Option<String>,
    flags: Option<String>,
    srcvers: Option<String>,
    receiver_public_key: Option<String>,
}

pub(crate) fn parse_resolved_services(services: Vec<ResolvedMdnsService>) -> Vec<SpeakerDevice> {
    let mut candidates = HashMap::<String, ParsedDeviceCandidate>::new();

    for service in services {
        let Some(candidate) = parse_resolved_service(service) else {
            continue;
        };

        match candidates.get_mut(&candidate.id) {
            Some(existing) => merge_device_candidate(existing, candidate),
            None => {
                candidates.insert(candidate.id.clone(), candidate);
            }
        }
    }

    candidates
        .into_values()
        .map(|candidate| {
            let receiver_kind = determine_receiver_kind(&candidate);
            let support = determine_device_support(&candidate, receiver_kind);
            SpeakerDevice {
                id: candidate.id,
                name: candidate.name,
                host: candidate.host,
                port: candidate.port,
                generation: candidate.generation,
                pairing_id: candidate.pairing_id,
                receiver_public_key: candidate.receiver_public_key,
                receiver_kind,
                support,
            }
        })
        .collect()
}

fn parse_resolved_service(service: ResolvedMdnsService) -> Option<ParsedDeviceCandidate> {
    if service.port == 0 {
        return None;
    }

    let host = service
        .ipv4_addresses
        .into_iter()
        .next()
        .map(|address| address.to_string())?;

    let (raw_id, name) = match service.service_kind {
        MdnsServiceKind::Raop => parse_raop_service_name(&service.fullname)?,
        MdnsServiceKind::AirPlay => {
            let name = parse_airplay_service_name(&service.fullname)?;
            (
                service.device_id.and_then(|id| normalize_device_id(&id)),
                name,
            )
        }
    };

    if name.is_empty() {
        return None;
    }

    let id = raw_id.unwrap_or_else(|| format!("{}:{}", host, service.port));

    Some(ParsedDeviceCandidate {
        id,
        name,
        host,
        port: service.port,
        generation: AirPlayGeneration::AirPlay1,
        has_airplay_service: service.service_kind == MdnsServiceKind::AirPlay,
        pairing_id: service.pairing_id,
        model_or_am: service.model_or_am,
        features: service.features,
        flags: service.flags,
        srcvers: service.srcvers,
        receiver_public_key: service.receiver_public_key,
    })
}

fn merge_device_candidate(existing: &mut ParsedDeviceCandidate, incoming: ParsedDeviceCandidate) {
    if existing.host.is_empty() {
        existing.host.clone_from(&incoming.host);
    }
    if existing.name.is_empty() {
        existing.name.clone_from(&incoming.name);
    }
    if !existing.has_airplay_service && incoming.has_airplay_service {
        existing.host.clone_from(&incoming.host);
        existing.port = incoming.port;
        existing.name.clone_from(&incoming.name);
    }

    existing.has_airplay_service |= incoming.has_airplay_service;
    merge_option(&mut existing.pairing_id, incoming.pairing_id);
    merge_option(
        &mut existing.receiver_public_key,
        incoming.receiver_public_key,
    );
    merge_option(&mut existing.model_or_am, incoming.model_or_am);
    merge_option(&mut existing.features, incoming.features);
    merge_option(&mut existing.flags, incoming.flags);
    merge_option(&mut existing.srcvers, incoming.srcvers);
}

fn merge_option(target: &mut Option<String>, source: Option<String>) {
    if target.is_none() {
        *target = source;
    }
}

fn determine_receiver_kind(candidate: &ParsedDeviceCandidate) -> ReceiverKind {
    if candidate.has_airplay_service
        && candidate.receiver_public_key.is_some()
        && candidate
            .model_or_am
            .as_deref()
            .is_some_and(is_modern_apple_receiver_model)
        && [
            candidate.features.as_ref(),
            candidate.flags.as_ref(),
            candidate.srcvers.as_ref(),
        ]
        .into_iter()
        .flatten()
        .next()
        .is_some()
    {
        return ReceiverKind::ModernAirPlayAuth;
    }

    ReceiverKind::ClassicRaop
}

fn determine_device_support(
    _candidate: &ParsedDeviceCandidate,
    _receiver_kind: ReceiverKind,
) -> DeviceSupport {
    DeviceSupport::Supported
}

fn is_modern_apple_receiver_model(model: &str) -> bool {
    let normalized = model.trim();
    normalized.starts_with("Mac") || normalized.starts_with("AppleTV")
}

fn parse_raop_service_name(fullname: &str) -> Option<(Option<String>, String)> {
    let instance = extract_instance_name(fullname, "._raop._tcp.local.")?;
    let (raw_id, raw_name) = match instance.split_once('@') {
        Some((id, name)) => (normalize_device_id(id), name.trim()),
        None => (None, instance),
    };

    if raw_name.is_empty() {
        return None;
    }

    Some((raw_id, String::from(raw_name)))
}

fn parse_airplay_service_name(fullname: &str) -> Option<String> {
    let instance = extract_instance_name(fullname, "._airplay._tcp.local.")?;
    if instance.is_empty() {
        return None;
    }

    Some(String::from(instance))
}

fn extract_instance_name<'a>(fullname: &'a str, suffix: &str) -> Option<&'a str> {
    let instance = fullname.strip_suffix(suffix)?.trim();
    if instance.is_empty() {
        return None;
    }

    Some(instance)
}

fn normalize_device_id(raw_id: &str) -> Option<String> {
    let normalized: String = raw_id
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_uppercase())
        .collect();

    if normalized.is_empty() {
        return None;
    }

    Some(normalized)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{MdnsServiceKind, ResolvedMdnsService, parse_resolved_services};
    use crate::app::{DeviceSupport, ReceiverKind};

    fn build_service(
        service_kind: MdnsServiceKind,
        fullname: &str,
        port: u16,
        ipv4_addresses: Vec<Ipv4Addr>,
        device_id: Option<&str>,
    ) -> ResolvedMdnsService {
        ResolvedMdnsService {
            service_kind,
            fullname: String::from(fullname),
            port,
            ipv4_addresses,
            device_id: device_id.map(String::from),
            pairing_id: None,
            model_or_am: None,
            features: None,
            flags: None,
            srcvers: None,
            receiver_public_key: None,
        }
    }

    #[test]
    fn test_parse_resolved_services_extracts_friendly_name_and_target_ip_from_raop() {
        let devices = parse_resolved_services(vec![build_service(
            MdnsServiceKind::Raop,
            "001122334455@Living Room._raop._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            None,
        )]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "001122334455");
        assert_eq!(devices[0].name, "Living Room");
        assert_eq!(devices[0].host, "192.168.12.25");
        assert_eq!(devices[0].endpoint(), "192.168.12.25:7000");
    }

    #[test]
    fn test_parse_resolved_services_extracts_airplay_name_and_device_id() {
        let devices = parse_resolved_services(vec![build_service(
            MdnsServiceKind::AirPlay,
            "Bedroom Speaker._airplay._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            Some("00:11:22:33:44:55"),
        )]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "001122334455");
        assert_eq!(devices[0].name, "Bedroom Speaker");
        assert_eq!(devices[0].host, "192.168.12.25");
    }

    #[test]
    fn test_parse_resolved_services_falls_back_to_endpoint_without_device_id() {
        let devices = parse_resolved_services(vec![build_service(
            MdnsServiceKind::AirPlay,
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
    fn test_parse_resolved_services_skips_entries_without_ipv4_address() {
        let devices = parse_resolved_services(vec![build_service(
            MdnsServiceKind::Raop,
            "001122334455@Living Room._raop._tcp.local.",
            7000,
            Vec::new(),
            None,
        )]);

        assert!(devices.is_empty());
    }

    #[test]
    fn test_parse_resolved_services_skips_entries_without_valid_name() {
        let devices = parse_resolved_services(vec![build_service(
            MdnsServiceKind::Raop,
            "001122334455@._raop._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            None,
        )]);

        assert!(devices.is_empty());
    }

    #[test]
    fn test_parse_resolved_services_deduplicates_across_service_types() {
        let devices = parse_resolved_services(vec![
            build_service(
                MdnsServiceKind::Raop,
                "001122334455@Living Room._raop._tcp.local.",
                7000,
                vec![Ipv4Addr::new(192, 168, 12, 25)],
                None,
            ),
            build_service(
                MdnsServiceKind::AirPlay,
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
    fn test_parse_resolved_services_marks_modern_receiver_as_unsupported() {
        let mut modern_receiver = build_service(
            MdnsServiceKind::AirPlay,
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

        let devices = parse_resolved_services(vec![modern_receiver]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].receiver_kind, ReceiverKind::ModernAirPlayAuth);
        assert_eq!(devices[0].support, DeviceSupport::Supported);
    }

    #[test]
    fn test_parse_resolved_services_does_not_misclassify_partial_airplay_features() {
        let mut partial_airplay = build_service(
            MdnsServiceKind::AirPlay,
            "Living Room._airplay._tcp.local.",
            7000,
            vec![Ipv4Addr::new(192, 168, 12, 25)],
            Some("00:11:22:33:44:55"),
        );
        partial_airplay.model_or_am = Some(String::from("SpeakerV1"));
        partial_airplay.features = Some(String::from("0x1"));

        let devices = parse_resolved_services(vec![partial_airplay]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].support, DeviceSupport::Supported);
    }
}
