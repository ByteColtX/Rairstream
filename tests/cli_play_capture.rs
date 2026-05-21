use rairstream::cli::{CliCommand, parse_cli};
use rairstream::session::LatencyProfile;

#[test]
fn parse_play_capture_command_with_multiple_devices() {
    let cli = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--device"),
        String::from("Bedroom"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayCapture {
            selectors: vec![String::from("Office"), String::from("Bedroom")],
            latency_profile: LatencyProfile::safe(),
        }
    );
}

#[test]
fn parse_play_capture_command_with_custom_zero_buffer() {
    let cli = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--buffer-ms"),
        String::from("0"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayCapture {
            selectors: vec![String::from("Office")],
            latency_profile: LatencyProfile::custom(0),
        }
    );
}

#[test]
fn parse_play_capture_custom_latency_requires_tuning_flag() {
    let error = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--latency"),
        String::from("custom"),
    ])
    .unwrap_err();

    assert!(error.to_string().contains("--latency custom requires"));
}

#[test]
fn parse_play_capture_command_with_custom_packet_frames() {
    let cli = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--buffer-ms"),
        String::from("25"),
        String::from("--packet-frames"),
        String::from("128"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayCapture {
            selectors: vec![String::from("Office")],
            latency_profile: LatencyProfile::custom_with_packet_frames(25, 128),
        }
    );
}

#[test]
fn parse_play_capture_packet_frames_overrides_profile_packet_size() {
    let cli = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--latency"),
        String::from("realtime"),
        String::from("--packet-frames"),
        String::from("64"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayCapture {
            selectors: vec![String::from("Office")],
            latency_profile: LatencyProfile::custom_with_packet_frames(50, 64),
        }
    );
}

#[test]
fn parse_play_capture_rejects_zero_packet_frames() {
    let error = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--packet-frames"),
        String::from("0"),
    ])
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("--packet-frames must be greater")
    );
}
