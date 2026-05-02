use rairstream::cli::{CliCommand, parse_cli};

#[test]
fn parse_pair_command_with_pin() {
    let cli = parse_cli([
        String::from("pair"),
        String::from("--device"),
        String::from("Living Room"),
        String::from("--pin"),
        String::from("3939"),
    ])
    .unwrap();

    assert_eq!(
        cli.command,
        CliCommand::Pair {
            selector: String::from("Living Room"),
            pin: Some(String::from("3939")),
        }
    );
}

#[test]
fn parse_pair_rejects_legacy_positional_selector() {
    let error = parse_cli([String::from("pair"), String::from("Living Room")]).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid command line: unexpected argument `Living Room`"
    );
}
