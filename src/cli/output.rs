use std::path::Path;

use crate::app::{InspectResult, PairedReceiverEntry};
use crate::pairing::ReceiverAuthFlow;
use crate::receiver::{
    CodecKind, DeviceSupport, PairingRequirement, Receiver, ReceiverKind, UnsupportedReason,
};

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
        "receiver | name={} | id={} | endpoint={} | kind={} | support={} | pairing={} | codecs={}",
        quoted_value(&receiver.name),
        receiver.id,
        receiver.endpoint(),
        format_receiver_kind(receiver.receiver_kind),
        format_support(&receiver.support),
        format_pairing_requirement(receiver.capabilities.pairing),
        format_codecs(&receiver.capabilities.codecs),
    )
}

fn format_inspect(result: &InspectResult) -> String {
    format!(
        "{} | multiroom={} | stored_credentials={}",
        format_receiver(&result.receiver),
        yes_no(result.receiver.capabilities.supports_multiroom),
        yes_no(result.has_stored_credentials),
    )
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

fn format_receiver_kind(kind: ReceiverKind) -> &'static str {
    match kind {
        ReceiverKind::ClassicRaop => "classic_raop",
        ReceiverKind::ModernAirPlayAuth => "modern_airplay_auth",
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

fn format_support(support: &DeviceSupport) -> String {
    match support {
        DeviceSupport::Supported => String::from("supported"),
        DeviceSupport::Experimental { reason } => {
            format!("experimental({})", format_unsupported_reason(*reason))
        }
        DeviceSupport::Unsupported { reason } => {
            format!("unsupported({})", format_unsupported_reason(*reason))
        }
    }
}

fn format_unsupported_reason(reason: UnsupportedReason) -> &'static str {
    match reason {
        UnsupportedReason::AuthenticationRequiredReceiver => "authentication_required_receiver",
        UnsupportedReason::PlatformCaptureUnsupported => "platform_capture_unsupported",
        UnsupportedReason::ExperimentalAirPlay2 => "experimental_airplay2",
    }
}

fn quoted_value(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
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
        AirPlayGeneration, CodecKind, DeviceSupport, PairingRequirement, Receiver,
        ReceiverCapabilities, ReceiverKind,
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
            pairing_id: Some(String::from("receiver-pairing-id")),
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ModernAirPlayAuth,
            support: DeviceSupport::Supported,
            capabilities: ReceiverCapabilities {
                codecs: vec![CodecKind::L16, CodecKind::Alac],
                pairing: PairingRequirement::PinOrCredentials,
                supports_multiroom: true,
                supports_ptp: true,
                supports_retransmit: true,
            },
        }
    }

    #[test]
    fn format_receiver_uses_stable_field_order() {
        assert_eq!(
            format_receiver(&build_receiver()),
            "receiver | name=\"Living Room\" | id=living-room | endpoint=192.168.1.20:7000 | kind=modern_airplay_auth | support=supported | pairing=pin_or_credentials | codecs=l16,alac"
        );
    }

    #[test]
    fn format_inspect_appends_inspection_fields() {
        let line = format_inspect(&InspectResult {
            receiver: build_receiver(),
            has_stored_credentials: true,
        });

        assert_eq!(
            line,
            "receiver | name=\"Living Room\" | id=living-room | endpoint=192.168.1.20:7000 | kind=modern_airplay_auth | support=supported | pairing=pin_or_credentials | codecs=l16,alac | multiroom=yes | stored_credentials=yes"
        );
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
