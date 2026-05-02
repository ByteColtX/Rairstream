use crate::receiver::{
    AirPlayGeneration, AuthMethod, CodecKind, Features, PairingRequirement, RaopEncryptionType,
    RaopMetadata, Receiver, ReceiverCapabilities, SupportLevel, SupportReason, TransportProfile,
    Version,
};

use super::{MdnsServiceKind, ResolvedMdnsService};

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDeviceCandidate {
    id: String,
    id_is_fallback: bool,
    name: String,
    host: String,
    port: u16,
    has_airplay_service: bool,
    model: Option<String>,
    manufacturer: Option<String>,
    serial_number: Option<String>,
    source_version: Version,
    firmware_version: Option<String>,
    os_version: Option<String>,
    protocol_version: Option<String>,
    features: Features,
    required_sender_features: Features,
    status_flags: u64,
    requires_password: bool,
    access_control: Option<u8>,
    pairing_identity: Option<String>,
    system_pairing_identity: Option<String>,
    receiver_public_key: Option<String>,
    bluetooth_address: Option<String>,
    homekit_home_id: Option<String>,
    group_id: Option<String>,
    is_group_leader: bool,
    group_public_name: Option<String>,
    group_contains_discoverable_leader: bool,
    home_group_id: Option<String>,
    household_id: Option<String>,
    parent_group_id: Option<String>,
    parent_group_contains_discoverable_leader: bool,
    tight_sync_id: Option<String>,
    raop: RaopMetadata,
}

pub(crate) fn parse_resolved_services(services: &[ResolvedMdnsService]) -> Vec<Receiver> {
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
    let transport_profile = determine_transport_profile(&candidate);
    let auth_method = determine_auth_method(&candidate);
    let support_level = determine_support_level(auth_method);
    let capabilities = determine_capabilities(&candidate, auth_method, transport_profile);

    Receiver {
        id: candidate.id,
        name: candidate.name,
        host: candidate.host,
        port: candidate.port,
        generation: if transport_profile.is_modern() {
            AirPlayGeneration::AirPlay2
        } else {
            AirPlayGeneration::AirPlay1
        },
        transport_profile,
        support_level,
        auth_method,
        model: candidate.model,
        manufacturer: candidate.manufacturer,
        serial_number: candidate.serial_number,
        source_version: candidate.source_version,
        firmware_version: candidate.firmware_version,
        os_version: candidate.os_version,
        protocol_version: candidate.protocol_version,
        features: candidate.features,
        required_sender_features: candidate.required_sender_features,
        status_flags: candidate.status_flags,
        requires_password: candidate.requires_password,
        access_control: candidate.access_control,
        pairing_identity: candidate.pairing_identity,
        system_pairing_identity: candidate.system_pairing_identity,
        receiver_public_key: candidate.receiver_public_key,
        bluetooth_address: candidate.bluetooth_address,
        homekit_home_id: candidate.homekit_home_id,
        group_id: candidate.group_id,
        is_group_leader: candidate.is_group_leader,
        group_public_name: candidate.group_public_name,
        group_contains_discoverable_leader: candidate.group_contains_discoverable_leader,
        home_group_id: candidate.home_group_id,
        household_id: candidate.household_id,
        parent_group_id: candidate.parent_group_id,
        parent_group_contains_discoverable_leader: candidate
            .parent_group_contains_discoverable_leader,
        tight_sync_id: candidate.tight_sync_id,
        capabilities,
        raop: candidate.raop,
        ..Receiver::default()
    }
    .with_compat_fields()
}

fn parse_resolved_service(service: &ResolvedMdnsService) -> Option<ParsedDeviceCandidate> {
    if service.port == 0 {
        return None;
    }

    let host = service
        .ipv4_addresses
        .iter()
        .copied()
        .next()
        .map(|address| address.to_string())?;

    let (raw_id, name) = match service.service_kind {
        MdnsServiceKind::Raop => parse_raop_service_name(&service.fullname)?,
        MdnsServiceKind::AirPlay => (
            service.txt_value("deviceid").and_then(normalize_device_id),
            parse_airplay_service_name(&service.fullname)?,
        ),
    };

    if name.is_empty() {
        return None;
    }

    let features = parse_features(service);
    let raop_source_version = service
        .txt_value_any(&["vs", "vn"])
        .and_then(Version::parse)
        .unwrap_or_default();
    let raop = if service.service_kind == MdnsServiceKind::Raop {
        RaopMetadata {
            port: Some(service.port),
            model: service.txt_value_any(&["am", "model"]).map(String::from),
            source_version: raop_source_version,
            status_flags: service
                .txt_value("sf")
                .and_then(parse_u64ish)
                .unwrap_or_default(),
            requires_password: parse_boolish(service.txt_value("pw")),
            vodka_version: service.txt_value("vv").map(String::from),
            codecs: service
                .txt_value("cn")
                .map(parse_raop_codecs)
                .unwrap_or_default(),
            encryption_types: service
                .txt_value("et")
                .map(parse_raop_encryption_types)
                .unwrap_or_default(),
            transport: service.txt_value("tp").map(String::from),
            metadata_types: service
                .txt_value("md")
                .map(parse_byte_list)
                .unwrap_or_default(),
            digest_auth: parse_boolish(service.txt_value("da")),
        }
    } else {
        RaopMetadata::default()
    };

    let id_is_fallback = raw_id.is_none();
    let id = raw_id.unwrap_or_else(|| format!("{}:{}", host, service.port));

    Some(ParsedDeviceCandidate {
        id,
        id_is_fallback,
        name,
        host,
        port: service.port,
        has_airplay_service: service.service_kind == MdnsServiceKind::AirPlay,
        model: service.txt_value_any(&["model", "am"]).map(String::from),
        manufacturer: service.txt_value("manufacturer").map(String::from),
        serial_number: service.txt_value("serialnumber").map(String::from),
        source_version: service
            .txt_value_any(&["srcvers", "vs", "vn"])
            .and_then(Version::parse)
            .unwrap_or_default(),
        firmware_version: service.txt_value("fv").map(String::from),
        os_version: service.txt_value_any(&["osvers", "ov"]).map(String::from),
        protocol_version: service.txt_value("protovers").map(String::from),
        features,
        required_sender_features: service
            .txt_value("rsf")
            .and_then(Features::from_txt_value)
            .unwrap_or_default(),
        status_flags: parse_status_flags(service),
        requires_password: parse_boolish(service.txt_value("pw")),
        access_control: service.txt_value("acl").and_then(parse_u8ish),
        pairing_identity: service.txt_value("pi").map(String::from),
        system_pairing_identity: service.txt_value("psi").map(String::from),
        receiver_public_key: service.txt_value("pk").map(String::from),
        bluetooth_address: service.txt_value("btaddr").map(String::from),
        homekit_home_id: service.txt_value("hkid").map(String::from),
        group_id: service.txt_value("gid").map(String::from),
        is_group_leader: parse_boolish(service.txt_value("igl")),
        group_public_name: service.txt_value("gpn").map(String::from),
        group_contains_discoverable_leader: parse_boolish(service.txt_value("gcgl")),
        home_group_id: service.txt_value("hgid").map(String::from),
        household_id: service.txt_value("hmid").map(String::from),
        parent_group_id: service.txt_value("pgid").map(String::from),
        parent_group_contains_discoverable_leader: parse_boolish(service.txt_value("pgcgl")),
        tight_sync_id: service.txt_value("tsid").map(String::from),
        raop,
    })
}

fn parse_features(service: &ResolvedMdnsService) -> Features {
    let airplay_features = service
        .txt_value("features")
        .and_then(Features::from_txt_value)
        .unwrap_or_default();
    let raop_features = service
        .txt_value("ft")
        .and_then(Features::from_txt_value)
        .unwrap_or_default();

    airplay_features.union(raop_features)
}

fn parse_status_flags(service: &ResolvedMdnsService) -> u64 {
    service
        .txt_value("flags")
        .and_then(parse_u64ish)
        .unwrap_or_default()
        | service
            .txt_value("sf")
            .and_then(parse_u64ish)
            .unwrap_or_default()
}

fn candidates_refer_to_same_device(
    existing: &ParsedDeviceCandidate,
    incoming: &ParsedDeviceCandidate,
) -> bool {
    if existing.id == incoming.id {
        return true;
    }

    if existing.pairing_identity.is_some() && existing.pairing_identity == incoming.pairing_identity
    {
        return true;
    }

    if existing.system_pairing_identity.is_some()
        && existing.system_pairing_identity == incoming.system_pairing_identity
    {
        return true;
    }

    if existing.receiver_public_key.is_some()
        && existing.receiver_public_key == incoming.receiver_public_key
    {
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
    existing.features = existing.features.union(incoming.features);
    existing.required_sender_features = existing
        .required_sender_features
        .union(incoming.required_sender_features);
    existing.status_flags |= incoming.status_flags;
    existing.requires_password |= incoming.requires_password;
    existing.is_group_leader |= incoming.is_group_leader;
    existing.group_contains_discoverable_leader |= incoming.group_contains_discoverable_leader;
    existing.parent_group_contains_discoverable_leader |=
        incoming.parent_group_contains_discoverable_leader;
    existing.source_version = existing.source_version.max(incoming.source_version);
    merge_option(&mut existing.model, incoming.model);
    merge_option(&mut existing.manufacturer, incoming.manufacturer);
    merge_option(&mut existing.serial_number, incoming.serial_number);
    merge_option(&mut existing.firmware_version, incoming.firmware_version);
    merge_option(&mut existing.os_version, incoming.os_version);
    merge_option(&mut existing.protocol_version, incoming.protocol_version);
    merge_option(&mut existing.access_control, incoming.access_control);
    merge_option(&mut existing.pairing_identity, incoming.pairing_identity);
    merge_option(
        &mut existing.system_pairing_identity,
        incoming.system_pairing_identity,
    );
    merge_option(
        &mut existing.receiver_public_key,
        incoming.receiver_public_key,
    );
    merge_option(&mut existing.bluetooth_address, incoming.bluetooth_address);
    merge_option(&mut existing.homekit_home_id, incoming.homekit_home_id);
    merge_option(&mut existing.group_id, incoming.group_id);
    merge_option(&mut existing.group_public_name, incoming.group_public_name);
    merge_option(&mut existing.home_group_id, incoming.home_group_id);
    merge_option(&mut existing.household_id, incoming.household_id);
    merge_option(&mut existing.parent_group_id, incoming.parent_group_id);
    merge_option(&mut existing.tight_sync_id, incoming.tight_sync_id);
    merge_option(&mut existing.raop.port, incoming.raop.port);
    merge_option(&mut existing.raop.model, incoming.raop.model);
    existing.raop.source_version = existing
        .raop
        .source_version
        .max(incoming.raop.source_version);
    existing.raop.status_flags |= incoming.raop.status_flags;
    existing.raop.requires_password |= incoming.raop.requires_password;
    merge_option(
        &mut existing.raop.vodka_version,
        incoming.raop.vodka_version,
    );
    merge_vec(&mut existing.raop.codecs, incoming.raop.codecs);
    merge_vec(
        &mut existing.raop.encryption_types,
        incoming.raop.encryption_types,
    );
    merge_option(&mut existing.raop.transport, incoming.raop.transport);
    merge_vec(
        &mut existing.raop.metadata_types,
        incoming.raop.metadata_types,
    );
    existing.raop.digest_auth |= incoming.raop.digest_auth;
}

fn merge_option<T>(target: &mut Option<T>, source: Option<T>) {
    if target.is_none() {
        *target = source;
    }
}

fn merge_vec<T>(target: &mut Vec<T>, source: Vec<T>)
where
    T: PartialEq,
{
    for value in source {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn determine_transport_profile(candidate: &ParsedDeviceCandidate) -> TransportProfile {
    if is_modern_receiver(candidate) {
        TransportProfile::ModernAuthRaop
    } else {
        TransportProfile::Raop
    }
}

fn determine_auth_method(candidate: &ParsedDeviceCandidate) -> AuthMethod {
    let advertised = candidate.features.auth_method();
    if advertised != AuthMethod::None {
        return advertised;
    }

    let required = candidate.required_sender_features.auth_method();
    if required != AuthMethod::None {
        return required;
    }

    if candidate.pairing_identity.is_some()
        || candidate.system_pairing_identity.is_some()
        || candidate.receiver_public_key.is_some()
    {
        AuthMethod::LegacyPin
    } else {
        AuthMethod::None
    }
}

fn determine_support_level(auth_method: AuthMethod) -> SupportLevel {
    match auth_method {
        AuthMethod::FairPlayRequired => SupportLevel::Unsupported {
            reason: SupportReason::FairPlayUnsupported,
        },
        AuthMethod::MfiRequired => SupportLevel::Unsupported {
            reason: SupportReason::MfiAuthenticationRequired,
        },
        AuthMethod::None | AuthMethod::LegacyPin | AuthMethod::HomeKitTransient => {
            SupportLevel::Supported
        }
    }
}

fn determine_capabilities(
    candidate: &ParsedDeviceCandidate,
    auth_method: AuthMethod,
    transport_profile: TransportProfile,
) -> ReceiverCapabilities {
    let codecs = if candidate.raop.codecs.is_empty() {
        if transport_profile.is_modern() {
            vec![CodecKind::L16, CodecKind::Alac, CodecKind::Aac]
        } else {
            vec![CodecKind::L16]
        }
    } else {
        candidate.raop.codecs.clone()
    };

    ReceiverCapabilities {
        codecs,
        pairing: match auth_method {
            AuthMethod::None => PairingRequirement::None,
            AuthMethod::LegacyPin => {
                if candidate.pairing_identity.is_some()
                    || candidate.system_pairing_identity.is_some()
                    || candidate.receiver_public_key.is_some()
                {
                    PairingRequirement::PinOrCredentials
                } else {
                    PairingRequirement::LegacyPin
                }
            }
            AuthMethod::HomeKitTransient
            | AuthMethod::FairPlayRequired
            | AuthMethod::MfiRequired => PairingRequirement::PinOrCredentials,
        },
        supports_multiroom: candidate.features.supports_ptp()
            || candidate.group_id.is_some()
            || candidate.is_group_leader
            || candidate.group_public_name.is_some()
            || candidate.group_contains_discoverable_leader
            || candidate.home_group_id.is_some()
            || candidate.household_id.is_some()
            || candidate.parent_group_id.is_some()
            || candidate.parent_group_contains_discoverable_leader
            || candidate.tight_sync_id.is_some()
            || transport_profile.is_modern(),
        supports_ptp: candidate.features.supports_ptp(),
        supports_retransmit: candidate.features.supports_rfc2198_redundancy()
            || transport_profile == TransportProfile::Raop
            || transport_profile.is_modern(),
    }
}

fn is_modern_receiver(candidate: &ParsedDeviceCandidate) -> bool {
    if candidate.has_airplay_service
        && candidate.pairing_identity.is_some()
        && candidate.receiver_public_key.is_some()
    {
        return true;
    }

    let features = candidate.features.union(candidate.required_sender_features);
    let has_auth_marker = candidate.pairing_identity.is_some()
        || candidate.system_pairing_identity.is_some()
        || candidate.receiver_public_key.is_some();
    let has_modern_features = features.supports_buffered_audio()
        || features.supports_homekit_pairing()
        || features.supports_transient_pairing()
        || features.supports_unified_pair_mfi()
        || features.supports_ptp();

    candidate.model.as_deref().is_some_and(|model| {
        is_modern_apple_receiver_model(model)
            && (candidate.has_airplay_service || has_auth_marker || has_modern_features)
    }) || (candidate.has_airplay_service && (has_auth_marker || has_modern_features))
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
        None
    } else {
        Some(normalized)
    }
}

fn normalized_receiver_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn parse_u64ish(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);

    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        u64::from_str_radix(normalized, 16).ok()
    } else {
        trimmed
            .parse::<u64>()
            .ok()
            .or_else(|| u64::from_str_radix(normalized, 16).ok())
    }
}

fn parse_u8ish(value: &str) -> Option<u8> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);

    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        u8::from_str_radix(normalized, 16).ok()
    } else {
        trimmed
            .parse::<u8>()
            .ok()
            .or_else(|| u8::from_str_radix(normalized, 16).ok())
    }
}

fn parse_byte_list(value: &str) -> Vec<u8> {
    value
        .split([',', ' '])
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u8>().ok())
        .collect()
}

fn parse_raop_codecs(value: &str) -> Vec<CodecKind> {
    parse_byte_list(value)
        .into_iter()
        .filter_map(|codec| match codec {
            0 => Some(CodecKind::L16),
            1 => Some(CodecKind::Alac),
            2 | 3 => Some(CodecKind::Aac),
            _ => None,
        })
        .collect()
}

fn parse_raop_encryption_types(value: &str) -> Vec<RaopEncryptionType> {
    parse_byte_list(value)
        .into_iter()
        .map(RaopEncryptionType::from_code)
        .collect()
}

fn parse_boolish(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value.trim(), "" | "1" | "true" | "yes"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;

    use super::{MdnsServiceKind, ResolvedMdnsService, parse_boolish, parse_resolved_services};
    use crate::receiver::{
        AuthMethod, Features, PairingRequirement, SupportLevel, SupportReason, TransportProfile,
        Version,
    };

    fn service(
        service_kind: MdnsServiceKind,
        fullname: &str,
        address: [u8; 4],
        port: u16,
        txt_records: &[(&str, &str)],
    ) -> ResolvedMdnsService {
        ResolvedMdnsService {
            service_kind,
            fullname: String::from(fullname),
            port,
            ipv4_addresses: vec![Ipv4Addr::from(address)],
            txt_records: txt_records
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), String::from(*value)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn merges_airplay_and_raop_metadata_for_modern_receiver() {
        let receivers = parse_resolved_services(&[
            service(
                MdnsServiceKind::Raop,
                "5855CA1AE288@Living Room._raop._tcp.local.",
                [192, 168, 1, 20],
                7001,
                &[
                    ("ft", "0x445F8A00,0x1C340"),
                    ("am", "AppleTV5,3"),
                    ("vs", "366.1"),
                    ("sf", "0x4"),
                    ("pw", "0"),
                    ("cn", "0,1,2"),
                    ("et", "0,1,3"),
                    ("tp", "TCP,UDP"),
                    ("md", "0,1,2"),
                    ("da", "1"),
                    ("vv", "2"),
                ],
            ),
            service(
                MdnsServiceKind::AirPlay,
                "Living Room._airplay._tcp.local.",
                [192, 168, 1, 20],
                7000,
                &[
                    ("deviceid", "58:55:CA:1A:E2:88"),
                    ("features", "0x40000A00,0x80300"),
                    ("rsf", "0x80"),
                    ("model", "AppleTV5,3"),
                    ("srcvers", "366.0"),
                    ("flags", "0x200"),
                    ("pw", "1"),
                    ("acl", "1"),
                    ("fv", "20.1"),
                    ("osvers", "14.4"),
                    ("protovers", "1.1"),
                    ("manufacturer", "Apple"),
                    ("serialNumber", "SN1234"),
                    ("btaddr", "01:23:45:67:89:AB"),
                    ("pi", "pair-id"),
                    ("psi", "system-id"),
                    ("pk", "receiver-pk"),
                    ("hkid", "home-id"),
                    ("gid", "group-id"),
                    ("igl", "1"),
                    ("gpn", "Everywhere"),
                    ("gcgl", "1"),
                    ("hgid", "home-group"),
                    ("hmid", "household-id"),
                    ("pgid", "parent-group"),
                    ("pgcgl", "1"),
                    ("tsid", "tight-sync"),
                ],
            ),
        ]);

        assert_eq!(receivers.len(), 1);
        let receiver = &receivers[0];

        assert_eq!(receiver.id, "5855CA1AE288");
        assert_eq!(receiver.name, "Living Room");
        assert_eq!(receiver.transport_profile, TransportProfile::ModernAuthRaop);
        assert_eq!(receiver.auth_method, AuthMethod::MfiRequired);
        assert_eq!(receiver.source_version, Version::new(366, 1, 0));
        assert_eq!(receiver.status_flags, 0x204);
        assert_eq!(receiver.model.as_deref(), Some("AppleTV5,3"));
        assert_eq!(receiver.manufacturer.as_deref(), Some("Apple"));
        assert_eq!(receiver.serial_number.as_deref(), Some("SN1234"));
        assert_eq!(receiver.required_sender_features.raw(), 0x80);
        assert!(receiver.requires_password);
        assert_eq!(receiver.access_control, Some(1));
        assert_eq!(receiver.pairing_identity.as_deref(), Some("pair-id"));
        assert_eq!(receiver.pairing_id.as_deref(), Some("pair-id"));
        assert_eq!(receiver.group_id.as_deref(), Some("group-id"));
        assert!(receiver.is_group_leader);
        assert!(receiver.group_contains_discoverable_leader);
        assert!(receiver.parent_group_contains_discoverable_leader);
        assert_eq!(receiver.tight_sync_id.as_deref(), Some("tight-sync"));
        assert_eq!(receiver.raop.port, Some(7001));
        assert_eq!(receiver.raop.model.as_deref(), Some("AppleTV5,3"));
        assert_eq!(receiver.raop.source_version, Version::new(366, 1, 0));
        assert_eq!(receiver.raop.status_flags, 0x4);
        assert!(!receiver.raop.requires_password);
        assert_eq!(receiver.raop.codecs.len(), 3);
        assert_eq!(receiver.raop.encryption_types.len(), 3);
        assert_eq!(receiver.raop.transport.as_deref(), Some("TCP,UDP"));
        assert_eq!(receiver.raop.metadata_types, vec![0, 1, 2]);
        assert!(receiver.raop.digest_auth);
        assert_eq!(receiver.raop.vodka_version.as_deref(), Some("2"));
        assert_eq!(receiver.receiver_kind, receiver.transport_profile);
        assert_eq!(receiver.support, receiver.support_level);
    }

    #[test]
    fn required_sender_features_can_mark_receiver_unsupported() {
        let receivers = parse_resolved_services(&[service(
            MdnsServiceKind::AirPlay,
            "Conference Room._airplay._tcp.local.",
            [192, 168, 1, 21],
            7000,
            &[
                ("deviceid", "00:11:22:33:44:55"),
                ("model", "AppleTV5,3"),
                ("rsf", "0x4000"),
                ("pk", "receiver-pk"),
            ],
        )]);

        assert_eq!(receivers.len(), 1);
        let receiver = &receivers[0];

        assert_eq!(receiver.transport_profile, TransportProfile::ModernAuthRaop);
        assert_eq!(receiver.auth_method, AuthMethod::FairPlayRequired);
        assert_eq!(
            receiver.required_sender_features.raw(),
            Features::AUTHENTICATION_FAIRPLAY
        );
        assert_eq!(
            receiver.capabilities.pairing,
            PairingRequirement::PinOrCredentials
        );
        assert!(matches!(
            receiver.support_level,
            SupportLevel::Unsupported {
                reason: SupportReason::FairPlayUnsupported
            }
        ));
    }

    #[test]
    fn empty_txt_flag_counts_as_true() {
        assert!(parse_boolish(Some("")));
    }
}
