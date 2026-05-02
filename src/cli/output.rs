//! CLI 输出格式化与终端打印。
use crate::app::{InspectResult, PairedReceiverEntry};
use crate::pairing::ReceiverAuthFlow;
use crate::receiver::{
    AuthMethod, CodecKind, PairingRequirement, RaopEncryptionType, Receiver, SupportLevel,
    SupportReason, TransportProfile,
};
use std::path::Path;

pub fn print_receivers(receivers: &[Receiver]) {
    if receivers.is_empty() {
        println!("No receivers discovered.");
        return;
    }

    for receiver in receivers {
        println!("{}", format_receiver(receiver));
    }
}

pub fn print_inspect(result: &InspectResult) {
    println!("{}", format_inspect(result));
}

pub fn print_paired_list(entries: &[PairedReceiverEntry]) {
    if entries.is_empty() {
        println!("No paired receivers.");
        return;
    }

    for entry in entries {
        println!("{}", format_paired(entry, None));
    }
}

pub fn print_paired_saved(entry: &PairedReceiverEntry) {
    println!("{}", format_paired(entry, Some("saved")));
}

pub fn print_paired_removed(entry: &PairedReceiverEntry) {
    println!("{}", format_paired(entry, Some("removed")));
}

pub fn print_pairing_pin_requested(receiver_name: &str) {
    println!("{}", format_pairing_pin_requested(receiver_name));
}

pub fn print_play_file_completed(path: &Path, selectors: &[String]) {
    println!(
        "playback | mode=file | status=completed | input={} | devices={}",
        quoted_value(&path.display().to_string()),
        format_selector_list(selectors),
    );
}

pub fn print_play_capture_started(selectors: &[String]) {
    println!(
        "playback | mode=capture | status=streaming | devices={} | hint=\"press Ctrl+C to stop\"",
        format_selector_list(selectors),
    );
}

pub fn print_play_capture_stopped(selectors: &[String]) {
    println!(
        "playback | mode=capture | status=stopped | devices={}",
        format_selector_list(selectors),
    );
}

fn format_receiver(receiver: &Receiver) -> String {
    format!(
        "receiver | name={} | id={} | endpoint={} | profile={} | support={} | auth={} | pairing={} | codecs={}",
        quoted_value(&receiver.name),
        receiver.id,
        receiver.endpoint(),
        format_transport_profile(receiver.transport_profile),
        format_support_level(&receiver.support_level),
        format_auth_method(receiver.auth_method),
        format_pairing_requirement(receiver.capabilities.pairing),
        format_codecs(&receiver.capabilities.codecs),
    )
}

fn format_inspect(result: &InspectResult) -> String {
    let receiver = &result.receiver;
    let mut fields = vec![
        format_receiver(receiver),
        format!(
            "stored_credentials={}",
            yes_no(result.has_stored_credentials)
        ),
        format!(
            "model={}",
            quoted_optional(receiver.model.as_deref(), "<unknown model>")
        ),
        format!("source_version={}", receiver.source_version),
        format!("features={}", receiver.features.to_txt_value()),
        format!("status_flags=0x{:X}", receiver.status_flags),
        format!(
            "ptp={}",
            format_runtime_capability(receiver.capabilities.supports_ptp)
        ),
        format!(
            "multiroom={}",
            format_runtime_capability(receiver.capabilities.supports_multiroom)
        ),
        format!(
            "pairing_identity={}",
            quoted_optional(receiver.pairing_identity.as_deref(), "<none>")
        ),
        format!(
            "system_pairing_identity={}",
            quoted_optional(receiver.system_pairing_identity.as_deref(), "<none>")
        ),
        format!(
            "group_name={}",
            quoted_optional(receiver.group_public_name.as_deref(), "<none>")
        ),
        format!("raop_codecs={}", format_codecs(&receiver.raop.codecs)),
        format!(
            "raop_encryption={}",
            format_raop_encryption(&receiver.raop.encryption_types)
        ),
    ];

    if let Some(transport) = receiver.raop.transport.as_deref() {
        fields.push(format!("raop_transport={}", quoted_value(transport)));
    }

    fields.join(" | ")
}

fn format_paired(entry: &PairedReceiverEntry, status: Option<&str>) -> String {
    let prefix = match status {
        Some(status) => format!("paired | status={status}"),
        None => String::from("paired"),
    };

    format!(
        "{prefix} | name={} | id={} | auth_flow={}",
        quoted_value(
            entry
                .display_name
                .as_deref()
                .unwrap_or("<unknown receiver>"),
        ),
        entry.receiver_id,
        format_auth_flow(&entry.auth_flow),
    )
}

fn format_pairing_pin_requested(receiver_name: &str) -> String {
    format!(
        "pairing | status=awaiting_pin | name={} | hint=\"enter the PIN shown on the receiver\"",
        quoted_value(receiver_name),
    )
}

fn format_codecs(codecs: &[CodecKind]) -> String {
    if codecs.is_empty() {
        return String::from("<none>");
    }

    codecs
        .iter()
        .map(|codec| match codec {
            CodecKind::L16 => "l16",
            CodecKind::Alac => "alac",
            CodecKind::Aac => "aac",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_raop_encryption(encryption_types: &[RaopEncryptionType]) -> String {
    if encryption_types.is_empty() {
        return String::from("<none>");
    }

    encryption_types
        .iter()
        .map(|encryption| match encryption {
            RaopEncryptionType::None => String::from("none"),
            RaopEncryptionType::Rsa => String::from("rsa"),
            RaopEncryptionType::FairPlay => String::from("fairplay"),
            RaopEncryptionType::MfiSap => String::from("mfi_sap"),
            RaopEncryptionType::FairPlaySapV25 => String::from("fairplay_sap_v25"),
            RaopEncryptionType::Unknown(code) => format!("unknown({code})"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_transport_profile(profile: TransportProfile) -> &'static str {
    match profile {
        TransportProfile::Raop => "raop",
        TransportProfile::ModernAuthRaop => "modern_auth_raop",
    }
}

fn format_pairing_requirement(requirement: PairingRequirement) -> &'static str {
    match requirement {
        PairingRequirement::None => "none",
        PairingRequirement::LegacyPin => "legacy_pin",
        PairingRequirement::PinOrCredentials => "pin_or_credentials",
    }
}

fn format_auth_flow(auth_flow: &ReceiverAuthFlow) -> &'static str {
    match auth_flow {
        ReceiverAuthFlow::Modern => "modern",
        ReceiverAuthFlow::LegacyPin => "legacy_pin",
    }
}

fn format_auth_method(auth_method: AuthMethod) -> &'static str {
    match auth_method {
        AuthMethod::None => "none",
        AuthMethod::LegacyPin => "legacy_pin",
        AuthMethod::HomeKitTransient => "homekit_transient",
        AuthMethod::FairPlayRequired => "fairplay_required",
        AuthMethod::MfiRequired => "mfi_required",
    }
}

fn format_support_level(support_level: &SupportLevel) -> String {
    match support_level {
        SupportLevel::Supported => String::from("supported"),
        SupportLevel::Experimental { reason } => {
            format!("experimental({})", format_support_reason(*reason))
        }
        SupportLevel::Unsupported { reason } => {
            format!("unsupported({})", format_support_reason(*reason))
        }
    }
}

fn format_support_reason(reason: SupportReason) -> &'static str {
    match reason {
        SupportReason::FairPlayUnsupported => "fairplay_unsupported",
        SupportReason::MfiAuthenticationRequired => "mfi_authentication_required",
        SupportReason::PlatformCaptureUnsupported => "platform_capture_unsupported",
        SupportReason::RuntimePathDisabled => "runtime_path_disabled",
    }
}

fn format_runtime_capability(enabled: bool) -> &'static str {
    if enabled {
        "yes(runtime_path_disabled)"
    } else {
        "no"
    }
}

fn quoted_value(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn quoted_optional(value: Option<&str>, fallback: &str) -> String {
    quoted_value(value.unwrap_or(fallback))
}

fn format_selector_list(selectors: &[String]) -> String {
    selectors
        .iter()
        .map(|selector| quoted_value(selector))
        .collect::<Vec<_>>()
        .join(",")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use crate::app::{InspectResult, PairedReceiverEntry};
    use crate::pairing::ReceiverAuthFlow;
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, CodecKind, Features, PairingRequirement, RaopEncryptionType,
        RaopMetadata, Receiver, ReceiverCapabilities, SupportLevel, TransportProfile, Version,
    };

    use super::{
        format_inspect, format_paired, format_pairing_pin_requested, format_receiver,
        format_selector_list,
    };

    fn build_receiver() -> Receiver {
        Receiver {
            id: String::from("living-room"),
            name: String::from("Living Room"),
            host: String::from("192.168.1.20"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay2,
            transport_profile: TransportProfile::ModernAuthRaop,
            support_level: SupportLevel::Supported,
            auth_method: AuthMethod::HomeKitTransient,
            model: Some(String::from("Mac16,10")),
            source_version: Version::new(940, 23, 1),
            features: Features::from_txt_value("0x4A7FCFD5,0x38174FDE").unwrap(),
            status_flags: 0x204,
            pairing_identity: Some(String::from("receiver-pairing-id")),
            system_pairing_identity: Some(String::from("system-pairing-id")),
            receiver_public_key: None,
            group_id: Some(String::from("group-id")),
            group_public_name: Some(String::from("Everywhere")),
            capabilities: ReceiverCapabilities {
                codecs: vec![CodecKind::L16, CodecKind::Alac],
                pairing: PairingRequirement::PinOrCredentials,
                supports_multiroom: true,
                supports_ptp: true,
                supports_retransmit: true,
            },
            raop: RaopMetadata {
                port: Some(7000),
                codecs: vec![CodecKind::L16, CodecKind::Alac],
                encryption_types: vec![RaopEncryptionType::Rsa],
                transport: Some(String::from("UDP")),
                metadata_types: vec![0, 1, 2],
                digest_auth: false,
                ..RaopMetadata::default()
            },
            ..Receiver::default()
        }
        .with_compat_fields()
    }

    #[test]
    fn format_receiver_uses_stable_field_order() {
        assert_eq!(
            format_receiver(&build_receiver()),
            "receiver | name=\"Living Room\" | id=living-room | endpoint=192.168.1.20:7000 | profile=modern_auth_raop | support=supported | auth=homekit_transient | pairing=pin_or_credentials | codecs=l16,alac"
        );
    }

    #[test]
    fn format_inspect_appends_inspection_fields() {
        let line = format_inspect(&InspectResult {
            receiver: build_receiver(),
            has_stored_credentials: true,
        });

        assert!(line.contains("stored_credentials=yes"));
        assert!(line.contains("model=\"Mac16,10\""));
        assert!(line.contains("source_version=940.23.1"));
        assert!(line.contains("ptp=yes(runtime_path_disabled)"));
        assert!(line.contains("multiroom=yes(runtime_path_disabled)"));
        assert!(line.contains("raop_encryption=rsa"));
    }

    #[test]
    fn format_paired_includes_action_status() {
        let line = format_paired(
            &PairedReceiverEntry {
                receiver_id: String::from("living-room"),
                display_name: Some(String::from("Living Room")),
                auth_flow: ReceiverAuthFlow::Modern,
            },
            Some("removed"),
        );

        assert_eq!(
            line,
            "paired | status=removed | name=\"Living Room\" | id=living-room | auth_flow=modern"
        );
    }

    #[test]
    fn format_selector_list_quotes_each_selector() {
        assert_eq!(
            format_selector_list(&[String::from("Living Room"), String::from("Kitchen"),]),
            "\"Living Room\",\"Kitchen\""
        );
    }

    #[test]
    fn format_pairing_pin_requested_reports_waiting_status_line() {
        assert_eq!(
            format_pairing_pin_requested("Living Room"),
            "pairing | status=awaiting_pin | name=\"Living Room\" | hint=\"enter the PIN shown on the receiver\""
        );
    }
}
