use std::path::PathBuf;

use rairstream::audio::SendCodecPreference;
use rairstream::cli::{CliCommand, parse_cli};

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
            codec_preference: SendCodecPreference::Auto,
        }
    );
}

#[test]
fn parse_play_file_command_accepts_codec_preference() {
    let cli = parse_cli([
        String::from("play"),
        String::from("file"),
        String::from("song.m4a"),
        String::from("--codec"),
        String::from("alac"),
        String::from("--device"),
        String::from("Living Room"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayFile {
            path: PathBuf::from("song.m4a"),
            selectors: vec![String::from("Living Room")],
            codec_preference: SendCodecPreference::Alac,
        }
    );
}
