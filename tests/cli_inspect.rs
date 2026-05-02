use rairstream::cli::{CliCommand, parse_cli};

#[test]
fn parse_inspect_command_requires_device_flag() {
    let cli = parse_cli([
        String::from("inspect"),
        String::from("--device"),
        String::from("Living Room"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::Inspect {
            selector: String::from("Living Room"),
        }
    );
}

#[test]
fn parse_inspect_rejects_legacy_positional_selector() {
    let error = parse_cli([String::from("inspect"), String::from("Living Room")]).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid command line: unexpected argument `Living Room`"
    );
}
