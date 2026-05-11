#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
use native_dialog::{DialogBuilder, MessageLevel};
use rairstream::cli::{CliOptions, parse_cli, print_error, print_error_message, run_cli};
use rairstream::error::RairstreamError;
use rairstream::ui::tray::run as run_tray;
use tracing_subscriber::EnvFilter;

const TRAY_BACKGROUND_ENV: &str = "RAIRSTREAM_TRAY_BACKGROUND";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupMode {
    TrayBootstrap,
    Tray,
    Cli(CliOptions),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "windows")]
    if !args.is_empty() {
        attach_parent_console_for_cli();
    }

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
                #[cfg(target_os = "windows")]
                show_windows_startup_error(&error);
                print_error_message("Startup Error", &error);
                std::process::exit(1);
            }
        }
        StartupMode::Tray => {
            if let Err(error) = init_logging("info") {
                #[cfg(target_os = "windows")]
                show_windows_startup_error(&error);
                print_error_message("Startup Error", &error);
                std::process::exit(2);
            }

            if let Err(error) = run_tray() {
                #[cfg(target_os = "windows")]
                show_windows_startup_error(&error);
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
    #[cfg(not(target_os = "windows"))]
    let _ = tray_background_requested;

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

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn attach_parent_console_for_cli() {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };

    fn is_invalid_handle(handle: HANDLE) -> bool {
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[allow(unsafe_code)]
    // SAFETY: These calls only attach this Windows GUI-subsystem process to the
    // parent's console and replace missing standard handles with CONIN$/CONOUT$.
    // Existing valid handles are preserved so shell redirection keeps working.
    unsafe {
        let stdout_missing = is_invalid_handle(GetStdHandle(STD_OUTPUT_HANDLE));
        let stderr_missing = is_invalid_handle(GetStdHandle(STD_ERROR_HANDLE));
        let stdin_missing = is_invalid_handle(GetStdHandle(STD_INPUT_HANDLE));
        if !stdout_missing && !stderr_missing && !stdin_missing {
            return;
        }

        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 && GetLastError() != ERROR_ACCESS_DENIED {
            return;
        }

        if stdout_missing {
            let output = CreateFileW(
                wide("CONOUT$").as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut_handle(),
            );
            if !is_invalid_handle(output) {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, output);
            }
        }

        if stderr_missing {
            let output = CreateFileW(
                wide("CONOUT$").as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut_handle(),
            );
            if !is_invalid_handle(output) {
                let _ = SetStdHandle(STD_ERROR_HANDLE, output);
            }
        }

        if stdin_missing {
            let input = CreateFileW(
                wide("CONIN$").as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut_handle(),
            );
            if !is_invalid_handle(input) {
                let _ = SetStdHandle(STD_INPUT_HANDLE, input);
            }
        }
    }
}

#[cfg(target_os = "windows")]
const fn null_mut_handle() -> windows_sys::Win32::Foundation::HANDLE {
    std::ptr::null_mut()
}

#[cfg(not(target_os = "windows"))]
fn spawn_tray_background() -> Result<(), String> {
    Err(String::from("tray background bootstrap is unsupported"))
}

#[cfg(target_os = "windows")]
fn show_windows_startup_error(message: &str) {
    let _ = DialogBuilder::message()
        .set_level(MessageLevel::Error)
        .set_title("Rairstream")
        .set_text(message)
        .alert()
        .show();
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

    #[test]
    fn help_arguments_parse_through_cli_mode() {
        let mode = determine_startup_mode(&[String::from("--help")], false).unwrap();

        assert!(matches!(
            mode,
            StartupMode::Cli(cli) if cli.command == CliCommand::Help
        ));
    }
}
