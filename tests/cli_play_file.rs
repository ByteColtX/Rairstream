use std::path::PathBuf;

use rairstream::cli::{CliCommand, parse_cli};
use rairstream::session::LatencyProfile;

#[test]
fn parse_play_file_command_with_multiple_devices() {
    let cli = parse_cli([
        String::from("play"),
        String::from("file"),
        String::from("song.m4a"),
        String::from("--device"),
        String::from("Living Room"),
        String::from("--device"),
        String::from("Kitchen"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayFile {
            path: PathBuf::from("song.m4a"),
            selectors: vec![String::from("Living Room"), String::from("Kitchen")],
            latency_profile: LatencyProfile::safe(),
        }
    );
}

#[test]
fn parse_play_file_command_with_low_latency_profile() {
    let cli = parse_cli([
        String::from("play"),
        String::from("file"),
        String::from("song.m4a"),
        String::from("--device"),
        String::from("Living Room"),
        String::from("--latency"),
        String::from("low"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayFile {
            path: PathBuf::from("song.m4a"),
            selectors: vec![String::from("Living Room")],
            latency_profile: LatencyProfile::low(),
        }
    );
}
