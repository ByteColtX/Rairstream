use std::io::{self, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::app::AppFacade;
use crate::discovery::MdnsDiscoveryService;
use crate::error::RairstreamError;

use super::output::{
    print_help, print_inspect, print_paired_list, print_paired_removed, print_paired_saved,
    print_pairing_pin_requested, print_play_capture_started, print_play_capture_stopped,
    print_play_file_completed, print_receivers,
};
use super::parse::{CliCommand, CliOptions};

const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(250);

enum CaptureExit {
    UserRequestedStop,
    PlaybackFailed(RairstreamError),
}

pub fn run_cli(cli: CliOptions) -> Result<(), RairstreamError> {
    if cli.command == CliCommand::Help {
        print_help();
        return Ok(());
    }

    let mut facade = AppFacade::new(MdnsDiscoveryService::default())?;

    match cli.command {
        CliCommand::Help => unreachable!("help exits before app initialization"),
        CliCommand::Discover => {
            let receivers = facade.discover()?;
            print_receivers(&receivers);
            Ok(())
        }
        CliCommand::Inspect { selector } => {
            let result = facade.inspect(&selector)?;
            print_inspect(&result);
            Ok(())
        }
        CliCommand::Pair { selector, pin } => {
            let pin = if let Some(pin) = pin {
                pin
            } else {
                let receiver = facade.request_pairing_pin_display(&selector)?;
                print_pairing_pin_requested(&receiver.name);
                prompt_pairing_pin(&receiver.name)?
            };
            let result = facade.pair(&selector, &pin)?;
            print_paired_saved(&result);
            Ok(())
        }
        CliCommand::PairedList => {
            let entries = facade.paired_list();
            print_paired_list(&entries);
            Ok(())
        }
        CliCommand::PairedForget { selector } => {
            let entry = facade.paired_forget(&selector)?;
            print_paired_removed(&entry);
            Ok(())
        }
        CliCommand::PlayFile {
            path,
            selectors,
            codec_preference,
        } => {
            facade.play_file_with_codec_preference(&path, &selectors, codec_preference)?;
            print_play_file_completed(&path, &selectors);
            Ok(())
        }
        CliCommand::PlayCapture {
            selectors,
            codec_preference,
        } => {
            let session =
                facade.play_capture_with_codec_preference(&selectors, codec_preference)?;
            print_play_capture_started(&selectors);
            match wait_for_ctrl_c_or_capture_end(&session)? {
                CaptureExit::UserRequestedStop => {
                    facade.stop_capture(session)?;
                    print_play_capture_stopped(&selectors);
                    Ok(())
                }
                CaptureExit::PlaybackFailed(error) => {
                    stop_capture_after_failure(&mut facade, session, error)
                }
            }
        }
    }
}

fn prompt_pairing_pin(receiver_name: &str) -> Result<String, RairstreamError> {
    print!("Enter PIN shown on {receiver_name}: ");
    io::stdout().flush()?;
    let mut pin = String::new();
    io::stdin().read_line(&mut pin)?;
    let pin = pin.trim();
    if pin.is_empty() {
        return Err(RairstreamError::InvalidInput {
            message: String::from("PIN cannot be empty"),
        });
    }
    Ok(pin.to_string())
}

fn wait_for_ctrl_c_or_capture_end(
    session: &crate::session::PlaybackSession,
) -> Result<CaptureExit, RairstreamError> {
    let (sender, receiver) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(std::io::Error::other)?;

    loop {
        match receiver.recv_timeout(PLAYBACK_POLL_INTERVAL) {
            Ok(()) => return Ok(CaptureExit::UserRequestedStop),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(error) = session.transport_error() {
                    return Ok(CaptureExit::PlaybackFailed(error));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::other("control-c listener disconnected").into());
            }
        }
    }
}

fn stop_capture_after_failure(
    facade: &mut AppFacade<MdnsDiscoveryService>,
    session: crate::session::PlaybackSession,
    error: RairstreamError,
) -> Result<(), RairstreamError> {
    match facade.stop_capture(session) {
        Ok(()) => Err(error),
        Err(stop_error) => Err(RairstreamError::Playback {
            message: format!("{error}; cleanup failed: {stop_error}"),
        }),
    }
}
