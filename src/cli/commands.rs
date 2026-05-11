use std::io::{self, Write};

use crate::app::AppFacade;
use crate::discovery::MdnsDiscoveryService;
use crate::error::RairstreamError;

use super::output::{
    print_help, print_inspect, print_paired_list, print_paired_removed, print_paired_saved,
    print_pairing_pin_requested, print_play_capture_started, print_play_capture_stopped,
    print_play_file_completed, print_receivers,
};
use super::parse::{CliCommand, CliOptions};

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
        CliCommand::PlayFile { path, selectors } => {
            facade.play_file(&path, &selectors)?;
            print_play_file_completed(&path, &selectors);
            Ok(())
        }
        CliCommand::PlayCapture { selectors } => {
            let session = facade.play_capture(&selectors)?;
            print_play_capture_started(&selectors);
            wait_for_ctrl_c()?;
            facade.stop_capture(session)?;
            print_play_capture_stopped(&selectors);
            Ok(())
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

fn wait_for_ctrl_c() -> Result<(), RairstreamError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(std::io::Error::other)?;
    receiver.recv().map_err(std::io::Error::other)?;
    Ok(())
}
