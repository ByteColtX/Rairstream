use rairstream::audio::SendCodecPreference;
use rairstream::cli::{CliCommand, parse_cli};

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
            codec_preference: SendCodecPreference::Auto,
        }
    );
}

#[test]
fn parse_play_capture_command_accepts_pcm_codec_preference() {
    let cli = parse_cli([
        String::from("play"),
        String::from("capture"),
        String::from("--device"),
        String::from("Office"),
        String::from("--codec"),
        String::from("pcm"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::PlayCapture {
            selectors: vec![String::from("Office")],
            codec_preference: SendCodecPreference::PcmL16,
        }
    );
}
