use crate::audio::AudioCaptureError;
use crate::config::ConfigError;
use crate::transport::AirPlayError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RairstreamError {
    #[error("unsupported on {os}: {feature}")]
    UnsupportedPlatform {
        os: &'static str,
        feature: &'static str,
    },
    #[error("no AirPlay or RAOP receivers discovered")]
    NoReceiversDiscovered,
    #[error("no receiver matched selector `{selector}`")]
    ReceiverNotFound { selector: String },
    #[error("selector `{selector}` matched multiple receivers: {matches:?}")]
    AmbiguousReceiver {
        selector: String,
        matches: Vec<String>,
    },
    #[error("invalid command line: {message}")]
    InvalidCli { message: String },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("playback failed: {message}")]
    Playback { message: String },
    #[error("pairing failed: {message}")]
    Pairing { message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Audio(#[from] AudioCaptureError),
    #[error(transparent)]
    Transport(#[from] AirPlayError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}
