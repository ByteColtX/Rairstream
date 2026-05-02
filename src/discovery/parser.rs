use crate::receiver::{
    AirPlayGeneration, CodecKind, DeviceSupport, PairingRequirement, Receiver,
    ReceiverCapabilities, ReceiverKind,
};

use super::{MdnsServiceKind, ResolvedMdnsService};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDeviceCandidate {
    id: String,
    id_is_fallback: bool,
    name: String,
    host: String,
    port: u16,
    has_airplay_service: bool,
    pairing_id: Option<String>,
    model_or_am: Option<String>,
    features: Option<String>,
    flags: Option<String>,
    srcvers: Option<String>,
    receiver_public_key: Option<String>,
}

pub(crate) fn parse_resolved_services(services: Vec<ResolvedMdnsService>) -> Vec<Receiver> {
    let mut candidates = Vec::<ParsedDeviceCandidate>::new();

    for service in services {
        let Some(candidate) = parse_resolved_service(service) else {
            continue;
        };

        match candidates
            .iter_mut()
            .find(|existing| candidates_refer_to_same_device(existing, &candidate))
        {
            Some(existing) => merge_device_candidate(existing, candidate),
            None => candidates.push(candidate),
        }
    }

    let mut receivers: Vec<Receiver> = candidates.into_iter().map(build_receiver).collect();
    receivers.sort_by(|left, right| left.name.cmp(&right.name));
    receivers
}

fn build_receiver(candidate: ParsedDeviceCandidate) -> Receiver {
    let receiver_kind = determine_receiver_kind(&candidate);
    let capabilities = determine_capabilities(&candidate, receiver_kind);
    let generation = match receiver_kind {
        ReceiverKind::ClassicRaop => AirPlayGeneration::AirPlay1,
        ReceiverKind::ModernAirPlayAuth => AirPlayGeneration::AirPlay2,
    };

    Receiver {
        id: candidate.id,
        name: candidate.name,
        host: candidate.host,
        port: candidate.port,
        generation,
        pairing_id: candidate.pairing_id,
        receiver_public_key: candidate.receiver_public_key,
        receiver_kind,
        support: DeviceSupport::Supported,
        capabilities,
    }
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

    let id_is_fallback = raw_id.is_none();
    let id = raw_id.unwrap_or_else(|| format!("{}:{}", host, service.port));

    Some(ParsedDeviceCandidate {
        id,
        id_is_fallback,
        name,
        host,
        port: service.port,
        has_airplay_service: service.service_kind == MdnsServiceKind::AirPlay,
        pairing_id: service.pairing_id,
        model_or_am: service.model_or_am,
        features: service.features,
        flags: service.flags,
        srcvers: service.srcvers,
        receiver_public_key: service.receiver_public_key,
    })
}

fn candidates_refer_to_same_device(
    existing: &ParsedDeviceCandidate,
    incoming: &ParsedDeviceCandidate,
) -> bool {
    if existing.id == incoming.id {
        return true;
    }

    if existing.pairing_id.is_some() && existing.pairing_id == incoming.pairing_id {
        return true;
    }

    if (existing.id_is_fallback || incoming.id_is_fallback)
        && existing.host == incoming.host
        && existing.port == incoming.port
    {
        return true;
    }

    existing.has_airplay_service != incoming.has_airplay_service
        && existing.host == incoming.host
        && normalized_receiver_name(&existing.name) == normalized_receiver_name(&incoming.name)
}

fn merge_device_candidate(existing: &mut ParsedDeviceCandidate, incoming: ParsedDeviceCandidate) {
    if existing.id_is_fallback && !incoming.id_is_fallback {
        existing.id.clone_from(&incoming.id);
        existing.id_is_fallback = false;
    }

    if !existing.has_airplay_service && incoming.has_airplay_service {
        existing.host.clone_from(&incoming.host);
        existing.port = incoming.port;
        existing.name.clone_from(&incoming.name);
    }

    existing.has_airplay_service |= incoming.has_airplay_service;
    existing.id_is_fallback &= incoming.id_is_fallback;
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
    if is_modern_receiver(candidate) {
        return ReceiverKind::ModernAirPlayAuth;
    }

    ReceiverKind::ClassicRaop
}

fn is_modern_receiver(candidate: &ParsedDeviceCandidate) -> bool {
    if candidate.has_airplay_service
        && candidate.pairing_id.is_some()
        && candidate.receiver_public_key.is_some()
    {
        return true;
    }

    let has_receiver_metadata =
        candidate.features.is_some() || candidate.flags.is_some() || candidate.srcvers.is_some();
    let has_auth_marker = candidate.pairing_id.is_some() || candidate.receiver_public_key.is_some();

    candidate.model_or_am.as_deref().is_some_and(|model| {
        is_modern_apple_receiver_model(model)
            && (candidate.has_airplay_service || has_auth_marker || has_receiver_metadata)
    })
}

fn determine_capabilities(
    candidate: &ParsedDeviceCandidate,
    receiver_kind: ReceiverKind,
) -> ReceiverCapabilities {
    match receiver_kind {
        ReceiverKind::ClassicRaop => ReceiverCapabilities::default(),
        ReceiverKind::ModernAirPlayAuth => ReceiverCapabilities {
            codecs: vec![CodecKind::L16, CodecKind::Alac, CodecKind::Aac],
            pairing: if candidate.pairing_id.is_some() {
                PairingRequirement::PinOrCredentials
            } else {
                PairingRequirement::LegacyPin
            },
            supports_multiroom: true,
            supports_ptp: true,
            supports_retransmit: true,
        },
    }
}

fn is_modern_apple_receiver_model(model: &str) -> bool {
    let normalized = model.trim().replace(' ', "");
    normalized.starts_with("Mac")
        || normalized.starts_with("AudioAccessory")
        || normalized.contains("HomePod")
        || normalized
            .strip_prefix("AppleTV")
            .and_then(parse_model_major_version)
            .is_some_and(|major_version| major_version >= 5)
}

fn parse_model_major_version(model_suffix: &str) -> Option<u16> {
    model_suffix.split_once(',')?.0.parse().ok()
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

fn normalized_receiver_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}
