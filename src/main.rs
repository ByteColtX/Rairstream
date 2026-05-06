#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use rairstream::cli::{CliOptions, parse_cli, print_error, print_error_message, run_cli};
use rairstream::error::RairstreamError;
use rairstream::ui::tray::run as run_tray;
use tracing_subscriber::EnvFilter;

const TRAY_BACKGROUND_ENV: &str = "RAIRSTREAM_TRAY_BACKGROUND";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupMode {
    TrayBootstrap,
    Tray,
    Cli(CliOptions),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let startup_mode =
        match determine_startup_mode(&args, std::env::var_os(TRAY_BACKGROUND_ENV).is_some()) {
            Ok(mode) => mode,
            Err(error) => {
                print_error(&error);
                std::process::exit(2);
            }
        };

    match startup_mode {
        StartupMode::TrayBootstrap => {
            if let Err(error) = spawn_tray_background() {
                print_error_message("Startup Error", &error);
                std::process::exit(1);
            }
        }
        StartupMode::Tray => {
            if let Err(error) = init_logging("info") {
                print_error_message("Startup Error", &error);
                std::process::exit(2);
            }

            if let Err(error) = run_tray() {
                print_error_message("Tray Error", &error);
                std::process::exit(1);
            }
        }
        StartupMode::Cli(cli) => {
            if let Err(error) = init_logging(cli.log_filter()) {
                print_error_message("Startup Error", &error);
                std::process::exit(2);
            }

            if let Err(error) = run_cli(cli) {
                print_error(&error);
                std::process::exit(1);
            }
        }
    }
}

fn init_logging(directive: &str) -> Result<(), String> {
    let env_filter = EnvFilter::try_new(directive)
        .map_err(|error| format!("invalid log filter `{directive}`: {error}"))?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init()
        .map_err(|error| format!("failed to initialize logging: {error}"))
}

fn determine_startup_mode(
    args: &[String],
    tray_background_requested: bool,
) -> Result<StartupMode, RairstreamError> {
    #[cfg(target_os = "windows")]
    {
        if args.is_empty() {
            return Ok(if tray_background_requested {
                StartupMode::Tray
            } else {
                StartupMode::TrayBootstrap
            });
        }
    }

    Ok(StartupMode::Cli(parse_cli(args.iter().cloned())?))
}

#[cfg(target_os = "windows")]
fn spawn_tray_background() -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;

    std::process::Command::new(current_exe)
        .env(TRAY_BACKGROUND_ENV, "1")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("failed to start tray background process: {error}"))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn spawn_tray_background() -> Result<(), String> {
    Err(String::from("tray background bootstrap is unsupported"))
}

#[cfg(test)]
mod tests {
    use rairstream::cli::CliCommand;

    use super::{StartupMode, determine_startup_mode};

    #[cfg(target_os = "windows")]
    #[test]
    fn no_arguments_enter_tray_bootstrap_by_default() {
        let mode = determine_startup_mode(&[], false).unwrap();

        assert_eq!(mode, StartupMode::TrayBootstrap);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn tray_background_flag_enters_tray_runtime() {
        let mode = determine_startup_mode(&[], true).unwrap();

        assert_eq!(mode, StartupMode::Tray);
    }

    #[test]
    fn cli_arguments_still_parse_through_cli_mode() {
        let mode = determine_startup_mode(&[String::from("discover")], false).unwrap();

        assert!(matches!(
            mode,
            StartupMode::Cli(cli) if cli.command == CliCommand::Discover
        ));
    }
}
