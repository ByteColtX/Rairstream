use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    Discover,
    Inspect {
        selector: String,
    },
    Pair {
        selector: String,
        pin: String,
    },
    PairedList,
    PairedForget {
        selector: String,
    },
    PlayFile {
        path: PathBuf,
        selectors: Vec<String>,
    },
    PlayCapture {
        selectors: Vec<String>,
    },
}
