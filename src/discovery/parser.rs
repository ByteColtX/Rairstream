use std::collections::HashSet;
use std::net::Ipv4Addr;

use crate::app::{AirPlayGeneration, SpeakerDevice};

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
}

pub(crate) fn parse_resolved_services(services: Vec<ResolvedMdnsService>) -> Vec<SpeakerDevice> {
    let mut seen_ids = HashSet::new();
    let mut devices = Vec::new();

    for service in services {
        let Some(device) = parse_resolved_service(service) else {
            continue;
        };

        if seen_ids.insert(device.id.clone()) {
            devices.push(device);
        }
    }

    devices
}

fn parse_resolved_service(service: ResolvedMdnsService) -> Option<SpeakerDevice> {
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

    Some(SpeakerDevice {
        id,
        name,
        host,
        port: service.port,
        generation: AirPlayGeneration::AirPlay1,
    })
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
        assert_eq!(devices[0].host, "192.168.12.25");
    }
}
