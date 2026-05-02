mod capabilities;
pub mod selector;

use serde::{Deserialize, Serialize};

pub use capabilities::{CodecKind, PairingRequirement, ReceiverCapabilities};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirPlayGeneration {
    AirPlay1,
    AirPlay2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiverKind {
    ClassicRaop,
    ModernAirPlayAuth,
}

impl Default for ReceiverKind {
    fn default() -> Self {
        Self::ClassicRaop
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UnsupportedReason {
    #[default]
    AuthenticationRequiredReceiver,
    PlatformCaptureUnsupported,
    ExperimentalAirPlay2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DeviceSupport {
    #[default]
    Supported,
    Experimental {
        reason: UnsupportedReason,
    },
    Unsupported {
        reason: UnsupportedReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receiver {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub generation: AirPlayGeneration,
    pub pairing_id: Option<String>,
    pub receiver_public_key: Option<String>,
    pub receiver_kind: ReceiverKind,
    pub support: DeviceSupport,
    pub capabilities: ReceiverCapabilities,
}

impl Receiver {
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
