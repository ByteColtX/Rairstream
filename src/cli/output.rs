//! CLI 输出格式化与终端打印。
use super::parse::{CLI_COMMAND_USAGE, CLI_USAGE, CLI_USAGE_HEADER};
use crate::app::{InspectResult, PairedReceiverEntry};
use crate::error::RairstreamError;
use crate::pairing::ReceiverAuthFlow;
use crate::receiver::{
    AuthMethod, CodecKind, PairingRequirement, RaopEncryptionType, Receiver, SupportLevel,
    SupportReason, TransportProfile,
};
use std::{
    env,
    fmt::Write as _,
    io::{self, IsTerminal, Write},
    path::Path,
};

pub fn print_receivers(receivers: &[Receiver]) {
    write_stdout(&format_receivers(receivers, Theme::stdout()));
}

pub fn print_inspect(result: &InspectResult) {
    write_stdout(&format_inspect(result, Theme::stdout()));
}

pub fn print_paired_list(entries: &[PairedReceiverEntry]) {
    write_stdout(&format_paired_list(entries, Theme::stdout()));
}

pub fn print_paired_saved(entry: &PairedReceiverEntry) {
    write_stdout(&format_paired_saved(entry, Theme::stdout()));
}

pub fn print_paired_removed(entry: &PairedReceiverEntry) {
    write_stdout(&format_paired_removed(entry, Theme::stdout()));
}

pub fn print_pairing_pin_requested(receiver_name: &str) {
    write_stdout(&format_pairing_pin_requested(
        receiver_name,
        Theme::stdout(),
    ));
}

pub fn print_play_file_completed(path: &Path, selectors: &[String]) {
    write_stdout(&format_play_file_completed(
        path,
        selectors,
        Theme::stdout(),
    ));
}

pub fn print_play_capture_started(selectors: &[String]) {
    write_stdout(&format_play_capture_started(selectors, Theme::stdout()));
}

pub fn print_play_capture_stopped(selectors: &[String]) {
    write_stdout(&format_play_capture_stopped(selectors, Theme::stdout()));
}

pub fn print_error(error: &RairstreamError) {
    write_stderr(&format_error(error, Theme::stderr()));
}

pub fn print_error_message(title: &str, message: &str) {
    write_stderr(&render_status_block(
        Theme::stderr(),
        Tone::Error,
        title,
        vec![message.to_string()],
    ));
}

#[derive(Clone, Copy)]
struct Theme {
    color: bool,
}

impl Theme {
    fn stdout() -> Self {
        Self {
            color: colors_enabled(io::stdout().is_terminal()),
        }
    }

    fn stderr() -> Self {
        Self {
            color: colors_enabled(io::stderr().is_terminal()),
        }
    }

    #[cfg(test)]
    const fn plain() -> Self {
        Self { color: false }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }

    fn title(self, text: &str) -> String {
        self.paint("1", text)
    }

    fn section_title(self, text: &str) -> String {
        self.paint("1;36", text)
    }

    fn badge(self, tone: Tone, text: &str) -> String {
        self.paint(tone.color_code(), text)
    }

    fn status_header(self, tone: Tone, title: &str) -> String {
        format!("{} {}", self.badge(tone, tone.icon()), self.title(title))
    }
}

#[derive(Clone, Copy)]
enum Tone {
    Info,
    Success,
    Warning,
    Error,
}

impl Tone {
    const fn icon(self) -> &'static str {
        match self {
            Self::Info => "ℹ",
            Self::Success => "✓",
            Self::Warning => "⚠",
            Self::Error => "✕",
        }
    }

    const fn color_code(self) -> &'static str {
        match self {
            Self::Info => "1;36",
            Self::Success => "1;32",
            Self::Warning => "1;33",
            Self::Error => "1;31",
        }
    }
}

fn format_receivers(receivers: &[Receiver], theme: Theme) -> String {
    if receivers.is_empty() {
        return render_status_block(
            theme,
            Tone::Info,
            "No Receivers Found",
            vec![String::from(
                "No AirPlay or RAOP receivers were discovered.",
            )],
        );
    }

    receivers
        .iter()
        .map(|receiver| format_receiver(receiver, theme))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_inspect(result: &InspectResult, theme: Theme) -> String {
    let receiver = &result.receiver;
    let mut sections = Vec::new();

    sections.push((
        "Summary",
        render_fields(vec![
            ("ID:", receiver.id.clone()),
            ("Endpoint:", receiver.endpoint()),
            (
                "Profile:",
                format_transport_profile(receiver.transport_profile).to_string(),
            ),
            ("Support:", format_support_level(&receiver.support_level)),
            (
                "Auth:",
                format_auth_method(receiver.auth_method).to_string(),
            ),
        ]),
    ));

    let mut identity_fields = vec![
        ("Source version:", receiver.source_version.to_string()),
        ("Status flags:", format!("0x{:X}", receiver.status_flags)),
    ];
    push_optional_field(&mut identity_fields, "Model:", receiver.model.clone());
    sections.push(("Identity", render_fields(identity_fields)));

    sections.push((
        "Capabilities",
        render_fields(vec![
            ("Features:", receiver.features.to_txt_value()),
            ("Codecs:", format_codecs(&receiver.capabilities.codecs)),
            (
                "PTP:",
                yes_no(receiver.capabilities.supports_ptp).to_string(),
            ),
            (
                "Multiroom:",
                yes_no(receiver.capabilities.supports_multiroom).to_string(),
            ),
        ]),
    ));

    let mut pairing_fields = vec![
        (
            "Requirement:",
            format_pairing_requirement(receiver.capabilities.pairing).to_string(),
        ),
        (
            "Stored credentials:",
            yes_no(result.has_stored_credentials).to_string(),
        ),
    ];
    push_optional_field(
        &mut pairing_fields,
        "Pairing identity:",
        receiver.pairing_identity.clone(),
    );
    push_optional_field(
        &mut pairing_fields,
        "System pairing identity:",
        receiver.system_pairing_identity.clone(),
    );
    sections.push(("Pairing", render_fields(pairing_fields)));

    let mut grouping_fields = Vec::new();
    push_optional_field(
        &mut grouping_fields,
        "Group name:",
        receiver.group_public_name.clone(),
    );
    if !grouping_fields.is_empty() {
        sections.push(("Grouping", render_fields(grouping_fields)));
    }

    let mut raop_fields = vec![
        ("Codecs:", format_codecs(&receiver.raop.codecs)),
        (
            "Encryption:",
            format_raop_encryption(&receiver.raop.encryption_types),
        ),
    ];
    push_optional_field(
        &mut raop_fields,
        "Transport:",
        receiver.raop.transport.clone(),
    );
    sections.push(("RAOP", render_fields(raop_fields)));

    let header = format_receiver_title(receiver, theme);
    render_card(&header, build_section_lines(theme, sections))
}

fn format_paired_list(entries: &[PairedReceiverEntry], theme: Theme) -> String {
    if entries.is_empty() {
        return render_status_block(
            theme,
            Tone::Info,
            "No Saved Pairings",
            vec![String::from("No paired receivers have been saved yet.")],
        );
    }

    entries
        .iter()
        .map(|entry| format_paired_entry(entry, theme, Tone::Info, "Saved Pairing"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_paired_saved(entry: &PairedReceiverEntry, theme: Theme) -> String {
    format_paired_entry(entry, theme, Tone::Success, "Pairing Saved")
}

fn format_paired_removed(entry: &PairedReceiverEntry, theme: Theme) -> String {
    format_paired_entry(entry, theme, Tone::Warning, "Pairing Removed")
}

fn format_pairing_pin_requested(receiver_name: &str, theme: Theme) -> String {
    render_status_block(
        theme,
        Tone::Warning,
        "Awaiting PIN",
        vec![
            format!("Receiver: {receiver_name}"),
            String::from("Hint: Enter the PIN shown on the receiver."),
        ],
    )
}

fn format_play_file_completed(path: &Path, selectors: &[String], theme: Theme) -> String {
    let mut lines = render_fields(vec![("Input:", path.display().to_string())]);
    lines.push(String::new());
    lines.extend(render_list("Devices", selectors));

    render_status_block(theme, Tone::Success, "Playback Completed", lines)
}

fn format_play_capture_started(selectors: &[String], theme: Theme) -> String {
    let mut lines = render_list("Devices", selectors);
    lines.push(String::new());
    lines.push(String::from("Hint: Press Ctrl+C to stop."));

    render_status_block(theme, Tone::Info, "Capture Streaming", lines)
}

fn format_play_capture_stopped(selectors: &[String], theme: Theme) -> String {
    render_status_block(
        theme,
        Tone::Info,
        "Capture Stopped",
        render_list("Devices", selectors),
    )
}

fn format_receiver(receiver: &Receiver, theme: Theme) -> String {
    let header = format_receiver_title(receiver, theme);
    render_card(
        &header,
        render_fields(vec![
            ("ID:", receiver.id.clone()),
            ("Endpoint:", receiver.endpoint()),
            (
                "Profile:",
                format_transport_profile(receiver.transport_profile).to_string(),
            ),
            (
                "Auth:",
                format_auth_method(receiver.auth_method).to_string(),
            ),
            (
                "Pairing:",
                format_pairing_requirement(receiver.capabilities.pairing).to_string(),
            ),
            ("Codecs:", format_codecs(&receiver.capabilities.codecs)),
        ]),
    )
}

fn format_paired_entry(
    entry: &PairedReceiverEntry,
    theme: Theme,
    tone: Tone,
    title: &str,
) -> String {
    render_status_block(
        theme,
        tone,
        title,
        render_fields(vec![
            (
                "Name:",
                entry
                    .display_name
                    .clone()
                    .unwrap_or_else(|| String::from("Unknown receiver")),
            ),
            ("ID:", entry.receiver_id.clone()),
            ("Auth flow:", format_auth_flow(&entry.auth_flow).to_string()),
        ]),
    )
}

fn format_error(error: &RairstreamError, theme: Theme) -> String {
    match error {
        RairstreamError::NoReceiversDiscovered => render_status_block(
            theme,
            Tone::Info,
            "No Receivers Found",
            vec![String::from(
                "No AirPlay or RAOP receivers were discovered.",
            )],
        ),
        RairstreamError::ReceiverNotFound { selector } => render_status_block(
            theme,
            Tone::Error,
            "Receiver Not Found",
            vec![
                format!("Selector: {selector}"),
                String::from("No receiver matched the requested selector."),
            ],
        ),
        RairstreamError::AmbiguousReceiver { selector, matches } => {
            let mut lines = vec![format!("Selector: {selector}"), String::new()];
            lines.extend(render_list("Matches", matches));
            render_status_block(theme, Tone::Error, "Ambiguous Device Selector", lines)
        }
        RairstreamError::InvalidCli { message } => format_invalid_cli(message, theme),
        RairstreamError::UnsupportedPlatform { os, feature } => render_status_block(
            theme,
            Tone::Error,
            "Unsupported Feature",
            vec![format!("Platform: {os}"), format!("Feature: {feature}")],
        ),
        RairstreamError::InvalidInput { message } => {
            render_status_block(theme, Tone::Error, "Invalid Input", vec![message.clone()])
        }
        RairstreamError::Playback { message } => {
            render_status_block(theme, Tone::Error, "Playback Failed", vec![message.clone()])
        }
        RairstreamError::Pairing { message } => {
            render_status_block(theme, Tone::Error, "Pairing Failed", vec![message.clone()])
        }
        _ => render_status_block(
            theme,
            Tone::Error,
            "Operation Failed",
            vec![error.to_string()],
        ),
    }
}

fn format_invalid_cli(message: &str, theme: Theme) -> String {
    let mut lines = Vec::new();

    if message != CLI_USAGE {
        lines.push(message.to_string());
        lines.push(String::new());
    }

    lines.push(theme.section_title("Usage"));
    lines.push(format!("  {CLI_USAGE_HEADER}"));
    lines.push(String::new());
    lines.push(theme.section_title("Commands"));
    lines.extend(CLI_COMMAND_USAGE.iter().map(|line| format!("  {line}")));

    render_status_block(theme, Tone::Error, "Command Line Error", lines)
}

fn build_section_lines(theme: Theme, sections: Vec<(&'static str, Vec<String>)>) -> Vec<String> {
    let mut body = Vec::new();
    for (title, lines) in sections {
        if lines.is_empty() {
            continue;
        }
        if !body.is_empty() {
            body.push(String::new());
        }
        body.push(theme.section_title(title));
        body.extend(lines.into_iter().map(|line| format!("  {line}")));
    }
    body
}

fn render_status_block(theme: Theme, tone: Tone, title: &str, body: Vec<String>) -> String {
    let header = theme.status_header(tone, title);
    render_card(&header, body)
}

fn render_card(header: &str, body: Vec<String>) -> String {
    let mut rendered = String::new();
    let _ = writeln!(rendered, "╭─ {header}");
    for line in body {
        if line.is_empty() {
            let _ = writeln!(rendered, "│");
        } else {
            let _ = writeln!(rendered, "│ {line}");
        }
    }
    rendered.push_str("╰─");
    rendered
}

fn render_fields(fields: Vec<(&'static str, String)>) -> Vec<String> {
    let width = fields
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0);
    fields
        .into_iter()
        .map(|(label, value)| format!("{label:<width$} {value}"))
        .collect()
}

fn render_list(label: &str, items: &[String]) -> Vec<String> {
    let mut lines = vec![format!("{label}:")];
    lines.extend(items.iter().map(|item| format!("  • {item}")));
    lines
}

fn push_optional_field(
    fields: &mut Vec<(&'static str, String)>,
    label: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value {
        fields.push((label, value));
    }
}

fn format_receiver_title(receiver: &Receiver, theme: Theme) -> String {
    let (tone, label) = support_badge(&receiver.support_level);
    format!(
        "{}  {}",
        theme.title(&receiver.name),
        theme.badge(tone, &label)
    )
}

fn support_badge(support_level: &SupportLevel) -> (Tone, String) {
    match support_level {
        SupportLevel::Supported => (Tone::Success, String::from("✓ Supported")),
        SupportLevel::Experimental { reason } => (
            Tone::Warning,
            format!("⚠ Experimental ({})", format_support_reason(*reason)),
        ),
        SupportLevel::Unsupported { reason } => (
            Tone::Error,
            format!("✕ Unsupported ({})", format_support_reason(*reason)),
        ),
    }
}

fn format_codecs(codecs: &[CodecKind]) -> String {
    if codecs.is_empty() {
        return String::from("None");
    }

    codecs
        .iter()
        .map(|codec| match codec {
            CodecKind::L16 => String::from("L16"),
            CodecKind::Alac => String::from("ALAC"),
            CodecKind::Aac => String::from("AAC"),
            CodecKind::AacEld => String::from("AAC ELD"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_raop_encryption(encryption_types: &[RaopEncryptionType]) -> String {
    if encryption_types.is_empty() {
        return String::from("None");
    }

    encryption_types
        .iter()
        .map(|encryption| match encryption {
            RaopEncryptionType::None => String::from("None"),
            RaopEncryptionType::Rsa => String::from("RSA"),
            RaopEncryptionType::FairPlay => String::from("FairPlay"),
            RaopEncryptionType::MfiSap => String::from("MFi-SAP"),
            RaopEncryptionType::FairPlaySapV25 => String::from("FairPlay-SAP v2.5"),
            RaopEncryptionType::Unknown(code) => format!("Unknown ({code})"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_transport_profile(profile: TransportProfile) -> &'static str {
    match profile {
        TransportProfile::Raop => "RAOP",
        TransportProfile::ModernAuthRaop => "Modern Auth RAOP",
    }
}

fn format_pairing_requirement(requirement: PairingRequirement) -> &'static str {
    match requirement {
        PairingRequirement::None => "None",
        PairingRequirement::LegacyPin => "Legacy PIN",
        PairingRequirement::PinOrCredentials => "PIN or credentials",
    }
}

fn format_auth_flow(auth_flow: &ReceiverAuthFlow) -> &'static str {
    match auth_flow {
        ReceiverAuthFlow::Modern => "Modern",
        ReceiverAuthFlow::LegacyPin => "Legacy PIN",
    }
}

fn format_auth_method(auth_method: AuthMethod) -> &'static str {
    match auth_method {
        AuthMethod::None => "None",
        AuthMethod::LegacyPin => "Legacy PIN",
        AuthMethod::HomeKitTransient => "HomeKit transient",
        AuthMethod::FairPlayRequired => "FairPlay required",
        AuthMethod::MfiRequired => "MFi required",
    }
}

fn format_support_level(support_level: &SupportLevel) -> String {
    match support_level {
        SupportLevel::Supported => String::from("Supported"),
        SupportLevel::Experimental { reason } => {
            format!("Experimental ({})", format_support_reason(*reason))
        }
        SupportLevel::Unsupported { reason } => {
            format!("Unsupported ({})", format_support_reason(*reason))
        }
    }
}

fn format_support_reason(reason: SupportReason) -> &'static str {
    match reason {
        SupportReason::FairPlayUnsupported => "FairPlay unsupported",
        SupportReason::MfiAuthenticationRequired => "MFi authentication required",
        SupportReason::PlatformCaptureUnsupported => "Platform capture unsupported",
        SupportReason::RuntimePathDisabled => "Runtime path disabled",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn colors_enabled(is_terminal: bool) -> bool {
    is_terminal && env::var_os("NO_COLOR").is_none()
}

fn write_stdout(message: &str) {
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(message.as_bytes());
    let _ = stdout.write_all(b"\n");
}

fn write_stderr(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(message.as_bytes());
    let _ = stderr.write_all(b"\n");
}

#[cfg(test)]
mod tests {
    use crate::app::{InspectResult, PairedReceiverEntry};
    use crate::error::RairstreamError;
    use crate::pairing::ReceiverAuthFlow;
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, CodecKind, Features, PairingRequirement, RaopEncryptionType,
        RaopMetadata, Receiver, ReceiverCapabilities, SupportLevel, SupportReason,
        TransportProfile, Version,
    };
    use std::path::Path;

    use super::{
        Theme, format_codecs, format_error, format_inspect, format_paired_removed,
        format_pairing_pin_requested, format_play_capture_started, format_receiver,
        format_receivers,
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

    fn plain_theme() -> Theme {
        Theme::plain()
    }

    fn assert_order(text: &str, earlier: &str, later: &str) {
        let earlier_index = text.find(earlier).unwrap();
        let later_index = text.find(later).unwrap();
        assert!(earlier_index < later_index);
    }

    #[test]
    fn format_receiver_renders_card_with_stable_field_order() {
        let card = format_receiver(&build_receiver(), plain_theme());

        assert!(card.contains("╭─ Living Room  ✓ Supported"));
        assert_order(&card, "ID:", "Endpoint:");
        assert_order(&card, "Endpoint:", "Profile:");
        assert_order(&card, "Profile:", "Auth:");
        assert_order(&card, "Auth:", "Pairing:");
        assert_order(&card, "Pairing:", "Codecs:");
        assert!(card.contains("Codecs:   L16, ALAC"));
    }

    #[test]
    fn format_receivers_uses_info_block_for_empty_results() {
        let rendered = format_receivers(&[], plain_theme());

        assert!(rendered.contains("No Receivers Found"));
        assert!(rendered.contains("No AirPlay or RAOP receivers were discovered."));
    }

    #[test]
    fn format_inspect_renders_sections_in_fixed_order() {
        let rendered = format_inspect(
            &InspectResult {
                receiver: build_receiver(),
                has_stored_credentials: true,
            },
            plain_theme(),
        );

        assert_order(&rendered, "│ Summary", "│ Identity");
        assert_order(&rendered, "│ Identity", "│ Capabilities");
        assert_order(&rendered, "│ Capabilities", "│ Pairing");
        assert_order(&rendered, "│ Pairing", "│ Grouping");
        assert_order(&rendered, "│ Grouping", "│ RAOP");
        assert!(rendered.contains("Stored credentials:"));
        assert!(rendered.contains("Requirement:"));
        assert!(rendered.contains("PIN or credentials"));
        assert!(rendered.contains("Encryption:"));
        assert!(rendered.contains("RSA"));
    }

    #[test]
    fn format_inspect_hides_missing_optional_fields() {
        let mut receiver = build_receiver();
        receiver.model = None;
        receiver.group_public_name = None;
        receiver.raop.transport = None;

        let rendered = format_inspect(
            &InspectResult {
                receiver,
                has_stored_credentials: false,
            },
            plain_theme(),
        );

        assert!(!rendered.contains("Model:"));
        assert!(!rendered.contains("│ Grouping"));
        assert!(!rendered.contains("Transport:"));
        assert!(rendered.contains("Stored credentials:"));
        assert!(rendered.contains("No"));
    }

    #[test]
    fn format_paired_action_uses_status_block() {
        let rendered = format_paired_removed(
            &PairedReceiverEntry {
                receiver_id: String::from("living-room"),
                display_name: Some(String::from("Living Room")),
                auth_flow: ReceiverAuthFlow::Modern,
            },
            plain_theme(),
        );

        assert!(rendered.contains("⚠ Pairing Removed"));
        assert!(rendered.contains("Name:      Living Room"));
        assert!(rendered.contains("Auth flow: Modern"));
    }

    #[test]
    fn format_pairing_pin_requested_reports_hint_block() {
        let rendered = format_pairing_pin_requested("Living Room", plain_theme());

        assert!(rendered.contains("⚠ Awaiting PIN"));
        assert!(rendered.contains("Receiver: Living Room"));
        assert!(rendered.contains("Hint: Enter the PIN shown on the receiver."));
    }

    #[test]
    fn format_play_capture_started_lists_devices_and_hint() {
        let rendered = format_play_capture_started(
            &[String::from("Office"), String::from("Bedroom")],
            plain_theme(),
        );

        assert!(rendered.contains("ℹ Capture Streaming"));
        assert!(rendered.contains("Devices:"));
        assert!(rendered.contains("• Office"));
        assert!(rendered.contains("• Bedroom"));
        assert!(rendered.contains("Hint: Press Ctrl+C to stop."));
    }

    #[test]
    fn format_invalid_cli_error_includes_usage_block() {
        let rendered = format_error(
            &RairstreamError::InvalidCli {
                message: String::from("unexpected argument `Living Room`"),
            },
            plain_theme(),
        );

        assert!(rendered.contains("✕ Command Line Error"));
        assert!(rendered.contains("unexpected argument `Living Room`"));
        assert!(rendered.contains("Usage"));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("inspect --device <selector>"));
    }

    #[test]
    fn format_no_receivers_error_uses_info_block() {
        let rendered = format_error(&RairstreamError::NoReceiversDiscovered, plain_theme());

        assert!(rendered.contains("ℹ No Receivers Found"));
        assert!(rendered.contains("No AirPlay or RAOP receivers were discovered."));
    }

    #[test]
    fn format_receiver_not_found_error_is_scannable() {
        let rendered = format_error(
            &RairstreamError::ReceiverNotFound {
                selector: String::from("Living Room"),
            },
            plain_theme(),
        );

        assert!(rendered.contains("✕ Receiver Not Found"));
        assert!(rendered.contains("Selector: Living Room"));
        assert!(rendered.contains("No receiver matched the requested selector."));
    }

    #[test]
    fn format_ambiguous_selector_lists_matches() {
        let rendered = format_error(
            &RairstreamError::AmbiguousReceiver {
                selector: String::from("Living Room"),
                matches: vec![String::from("Living Room"), String::from("Living Room TV")],
            },
            plain_theme(),
        );

        assert!(rendered.contains("✕ Ambiguous Device Selector"));
        assert!(rendered.contains("Matches:"));
        assert!(rendered.contains("• Living Room"));
        assert!(rendered.contains("• Living Room TV"));
    }

    #[test]
    fn format_play_file_completed_renders_input_path() {
        let rendered = super::format_play_file_completed(
            Path::new("song.m4a"),
            &[String::from("Living Room")],
            plain_theme(),
        );

        assert!(rendered.contains("✓ Playback Completed"));
        assert!(rendered.contains("Input: song.m4a"));
        assert!(rendered.contains("• Living Room"));
    }

    #[test]
    fn format_receiver_support_badge_uses_warning_and_reason() {
        let mut receiver = build_receiver();
        receiver.support_level = SupportLevel::Experimental {
            reason: SupportReason::RuntimePathDisabled,
        };

        let rendered = format_receiver(&receiver, plain_theme());

        assert!(rendered.contains("⚠ Experimental (Runtime path disabled)"));
    }

    #[test]
    fn format_codecs_distinguishes_aac_eld_from_aac() {
        assert_eq!(
            format_codecs(&[
                CodecKind::L16,
                CodecKind::Alac,
                CodecKind::Aac,
                CodecKind::AacEld,
            ]),
            "L16, ALAC, AAC, AAC ELD"
        );
    }
}
