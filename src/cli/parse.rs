//! CLI 参数解析与基础校验。

use std::path::PathBuf;

use crate::error::RairstreamError;
use crate::session::LatencyProfile;

pub(crate) const CLI_USAGE: &str =
    "usage: rairstream [-h|--help] [-v|-vv] [--log-level <error|warn|info|debug|trace>] <command>";
pub(crate) const CLI_USAGE_HEADER: &str =
    "rairstream [-h|--help] [-v|-vv] [--log-level <error|warn|info|debug|trace>] <command>";
pub(crate) const CLI_COMMAND_USAGE: &[&str] = &[
    "discover",
    "inspect --device <selector>",
    "pair --device <selector> [--pin <PIN>]",
    "paired list",
    "paired forget --device <selector>",
    "play file <path> --device <selector>... [--latency <safe|normal|low|realtime|custom>] [--buffer-ms <ms>] [--packet-frames <frames>]",
    "play capture --device <selector>... [--latency <safe|normal|low|realtime|custom>] [--buffer-ms <ms>] [--packet-frames <frames>]",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Discover,
    Inspect {
        selector: String,
    },
    Pair {
        selector: String,
        pin: Option<String>,
    },
    PairedList,
    PairedForget {
        selector: String,
    },
    PlayFile {
        path: PathBuf,
        selectors: Vec<String>,
        latency_profile: LatencyProfile,
    },
    PlayCapture {
        selectors: Vec<String>,
        latency_profile: LatencyProfile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub command: CliCommand,
    pub log_level: Option<String>,
    pub verbosity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LatencySelection {
    Profile(LatencyProfile),
    Custom,
}

impl CliOptions {
    #[must_use]
    pub fn log_filter(&self) -> &str {
        self.log_level.as_deref().unwrap_or(match self.verbosity {
            0 => "info",
            1 => "debug",
            _ => "trace",
        })
    }
}

pub fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliOptions, RairstreamError> {
    let mut args = args.into_iter().peekable();
    let mut verbosity = 0_u8;
    let mut log_level = None;
    let mut positionals = Vec::new();
    let mut help_requested = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => help_requested = true,
            "-v" | "--verbose" => verbosity = verbosity.saturating_add(1),
            "-vv" => verbosity = verbosity.saturating_add(2),
            "--log-level" => {
                let value = args.next().ok_or_else(|| RairstreamError::InvalidCli {
                    message: String::from("--log-level requires a value"),
                })?;
                log_level = Some(parse_log_level(value)?);
            }
            _ if arg.starts_with("--log-level=") => {
                log_level = Some(parse_log_level(
                    arg.trim_start_matches("--log-level=").to_string(),
                )?);
            }
            _ => positionals.push(arg),
        }
    }

    let command = if help_requested {
        if positionals.is_empty() {
            CliCommand::Help
        } else {
            return Err(RairstreamError::InvalidCli {
                message: String::from("-h/--help cannot be combined with a command"),
            });
        }
    } else {
        parse_command(&positionals)?
    };
    Ok(CliOptions {
        command,
        log_level,
        verbosity,
    })
}

fn parse_log_level(value: String) -> Result<String, RairstreamError> {
    match value.as_str() {
        "error" | "warn" | "info" | "debug" | "trace" => Ok(value),
        _ => Err(RairstreamError::InvalidCli {
            message: format!("unsupported log level `{value}`"),
        }),
    }
}

fn parse_command(args: &[String]) -> Result<CliCommand, RairstreamError> {
    let Some((command, tail)) = args.split_first() else {
        return Err(usage_error());
    };

    match command.as_str() {
        "discover" => {
            ensure_no_remaining_args(tail)?;
            Ok(CliCommand::Discover)
        }
        "inspect" => {
            let (selector, tail) = parse_device_selector(tail)?;
            ensure_no_remaining_args(tail)?;
            Ok(CliCommand::Inspect { selector })
        }
        "pair" => {
            let (selector, tail) = parse_device_selector(tail)?;
            let pin = parse_optional_pin(tail)?;
            Ok(CliCommand::Pair { selector, pin })
        }
        "paired" => parse_paired_command(tail),
        "play" => parse_play_command(tail),
        _ => Err(usage_error()),
    }
}

fn parse_paired_command(args: &[String]) -> Result<CliCommand, RairstreamError> {
    let Some((subcommand, tail)) = args.split_first() else {
        return Err(usage_error());
    };

    match subcommand.as_str() {
        "list" => {
            ensure_no_remaining_args(tail)?;
            Ok(CliCommand::PairedList)
        }
        "forget" => {
            let (selector, tail) = parse_device_selector(tail)?;
            ensure_no_remaining_args(tail)?;
            Ok(CliCommand::PairedForget { selector })
        }
        _ => Err(usage_error()),
    }
}

fn parse_play_command(args: &[String]) -> Result<CliCommand, RairstreamError> {
    let Some((subcommand, tail)) = args.split_first() else {
        return Err(usage_error());
    };

    match subcommand.as_str() {
        "file" => {
            let Some((path, selectors)) = tail.split_first() else {
                return Err(usage_error());
            };
            let (selectors, latency_profile) = parse_play_selectors_and_latency(selectors)?;
            Ok(CliCommand::PlayFile {
                path: PathBuf::from(path),
                selectors,
                latency_profile,
            })
        }
        "capture" => {
            let (selectors, latency_profile) = parse_play_selectors_and_latency(tail)?;
            Ok(CliCommand::PlayCapture {
                selectors,
                latency_profile,
            })
        }
        _ => Err(usage_error()),
    }
}

fn parse_device_selector(args: &[String]) -> Result<(String, &[String]), RairstreamError> {
    let Some((flag, tail)) = args.split_first() else {
        return Err(usage_error());
    };

    if flag != "--device" {
        return Err(unexpected_argument_error(flag));
    }

    let Some((selector, remaining)) = tail.split_first() else {
        return Err(missing_value_error("--device"));
    };

    Ok((normalize_cli_text(selector, "selector")?, remaining))
}

fn parse_optional_pin(args: &[String]) -> Result<Option<String>, RairstreamError> {
    match args {
        [] => Ok(None),
        [flag] if flag == "--pin" => Err(missing_value_error("--pin")),
        [flag, pin] if flag == "--pin" => Ok(Some(normalize_cli_text(pin, "PIN")?)),
        [unexpected, ..] => Err(unexpected_argument_error(unexpected)),
    }
}

fn ensure_no_remaining_args(args: &[String]) -> Result<(), RairstreamError> {
    match args {
        [] => Ok(()),
        [unexpected, ..] => Err(unexpected_argument_error(unexpected)),
    }
}

fn normalize_cli_text(value: &str, label: &str) -> Result<String, RairstreamError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RairstreamError::InvalidCli {
            message: format!("{label} cannot be empty"),
        });
    }

    Ok(value.to_string())
}

fn parse_play_selectors_and_latency(
    args: &[String],
) -> Result<(Vec<String>, LatencyProfile), RairstreamError> {
    let mut selectors = Vec::new();
    let mut latency_selection = None;
    let mut custom_buffer_ms = None;
    let mut custom_packet_frames = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--device" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| RairstreamError::InvalidCli {
                        message: String::from("--device requires a value"),
                    })?;
                selectors.push(normalize_cli_text(value, "selector")?);
                index += 2;
            }
            "--latency" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| missing_value_error("--latency"))?;
                latency_selection = Some(parse_latency_selection(value)?);
                index += 2;
            }
            "--buffer-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| missing_value_error("--buffer-ms"))?;
                custom_buffer_ms = Some(parse_buffer_ms(value)?);
                index += 2;
            }
            "--packet-frames" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| missing_value_error("--packet-frames"))?;
                custom_packet_frames = Some(parse_packet_frames(value)?);
                index += 2;
            }
            unexpected if unexpected.starts_with("--latency=") => {
                latency_selection = Some(parse_latency_selection(
                    unexpected.trim_start_matches("--latency="),
                )?);
                index += 1;
            }
            unexpected if unexpected.starts_with("--buffer-ms=") => {
                custom_buffer_ms = Some(parse_buffer_ms(
                    unexpected.trim_start_matches("--buffer-ms="),
                )?);
                index += 1;
            }
            unexpected if unexpected.starts_with("--packet-frames=") => {
                custom_packet_frames = Some(parse_packet_frames(
                    unexpected.trim_start_matches("--packet-frames="),
                )?);
                index += 1;
            }
            unexpected => {
                return Err(RairstreamError::InvalidCli {
                    message: format!("unexpected argument `{unexpected}`"),
                });
            }
        }
    }

    if selectors.is_empty() {
        return Err(RairstreamError::InvalidCli {
            message: String::from("at least one --device selector is required"),
        });
    }

    let latency_profile = match (latency_selection, custom_buffer_ms, custom_packet_frames) {
        (Some(LatencySelection::Custom), None, None) => {
            return Err(RairstreamError::InvalidCli {
                message: String::from(
                    "--latency custom requires --buffer-ms <ms> or --packet-frames <frames>",
                ),
            });
        }
        (_, Some(buffer_ms), packet_frames) => LatencyProfile::custom_with_packet_frames(
            buffer_ms,
            packet_frames.unwrap_or(crate::audio::RAOP_FRAMES_PER_PACKET),
        ),
        (_, None, Some(packet_frames)) => {
            let buffer_ms = match latency_selection {
                Some(LatencySelection::Profile(profile)) => profile.buffer_ms(),
                Some(LatencySelection::Custom) | None => LatencyProfile::safe().buffer_ms(),
            };
            LatencyProfile::custom_with_packet_frames(buffer_ms, packet_frames)
        }
        (Some(LatencySelection::Profile(profile)), None, None) => profile,
        (None, None, None) => LatencyProfile::safe(),
    };

    Ok((selectors, latency_profile))
}

fn parse_latency_selection(value: &str) -> Result<LatencySelection, RairstreamError> {
    match value {
        "safe" => Ok(LatencySelection::Profile(LatencyProfile::safe())),
        "normal" => Ok(LatencySelection::Profile(LatencyProfile::normal())),
        "low" => Ok(LatencySelection::Profile(LatencyProfile::low())),
        "realtime" => Ok(LatencySelection::Profile(LatencyProfile::realtime())),
        "custom" => Ok(LatencySelection::Custom),
        _ => Err(RairstreamError::InvalidCli {
            message: format!("unsupported latency profile `{value}`"),
        }),
    }
}

fn parse_buffer_ms(value: &str) -> Result<u32, RairstreamError> {
    value
        .parse::<u32>()
        .map_err(|_| RairstreamError::InvalidCli {
            message: format!("--buffer-ms must be a non-negative integer, got `{value}`"),
        })
}

fn parse_packet_frames(value: &str) -> Result<usize, RairstreamError> {
    let frames = value
        .parse::<usize>()
        .map_err(|_| RairstreamError::InvalidCli {
            message: format!("--packet-frames must be a positive integer, got `{value}`"),
        })?;

    if frames == 0 {
        return Err(RairstreamError::InvalidCli {
            message: String::from("--packet-frames must be greater than 0"),
        });
    }

    Ok(frames)
}

fn usage_error() -> RairstreamError {
    RairstreamError::InvalidCli {
        message: String::from(CLI_USAGE),
    }
}

fn unexpected_argument_error(argument: &str) -> RairstreamError {
    RairstreamError::InvalidCli {
        message: format!("unexpected argument `{argument}`"),
    }
}

fn missing_value_error(flag: &str) -> RairstreamError {
    RairstreamError::InvalidCli {
        message: format!("{flag} requires a value"),
    }
}

#[cfg(test)]
mod tests {
    use super::{CliCommand, parse_cli};

    #[test]
    fn parse_help_short_flag() {
        let cli = parse_cli([String::from("-h")]).unwrap();

        assert_eq!(cli.command, CliCommand::Help);
    }

    #[test]
    fn parse_help_long_flag() {
        let cli = parse_cli([String::from("--help")]).unwrap();

        assert_eq!(cli.command, CliCommand::Help);
    }

    #[test]
    fn parse_help_rejects_command_combination() {
        let error = parse_cli([String::from("--help"), String::from("discover")]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid command line: -h/--help cannot be combined with a command"
        );
    }

    #[test]
    fn parse_play_capture_requires_device() {
        assert!(parse_cli([String::from("play"), String::from("capture")]).is_err());
    }

    #[test]
    fn parse_pair_with_pin() {
        let cli = parse_cli([
            String::from("pair"),
            String::from("--device"),
            String::from("Living Room"),
            String::from("--pin"),
            String::from("123456"),
        ])
        .unwrap();

        assert_eq!(
            cli.command,
            CliCommand::Pair {
                selector: String::from("Living Room"),
                pin: Some(String::from("123456")),
            }
        );
    }

    #[test]
    fn parse_inspect_accepts_device_flag() {
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
    fn parse_paired_forget_accepts_device_flag() {
        let cli = parse_cli([
            String::from("paired"),
            String::from("forget"),
            String::from("--device"),
            String::from("Living Room"),
        ])
        .unwrap();

        assert_eq!(
            cli.command,
            CliCommand::PairedForget {
                selector: String::from("Living Room"),
            }
        );
    }

    #[test]
    fn parse_rejects_blank_device_selector() {
        let error = parse_cli([
            String::from("play"),
            String::from("capture"),
            String::from("--device"),
            String::from("   "),
        ])
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid command line: selector cannot be empty"
        );
    }

    #[test]
    fn parse_rejects_legacy_positional_selector() {
        let error = parse_cli([String::from("pair"), String::from("Living Room")]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid command line: unexpected argument `Living Room`"
        );
    }
}
