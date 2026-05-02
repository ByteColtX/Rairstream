use rairstream::cli::{CliCommand, parse_cli};

#[test]
fn parse_discover_command() {
    let cli = parse_cli([String::from("discover")]).unwrap();

    assert_eq!(cli.command, CliCommand::Discover);
}
