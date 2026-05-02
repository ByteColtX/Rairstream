pub mod commands;
pub mod events;
mod facade;
pub mod models;

pub use facade::AppFacade;
pub use models::{AppState, InspectResult, PairedReceiverEntry, SessionState};

pub use crate::error::RairstreamError;
pub use crate::receiver::{
    AirPlayGeneration, CodecKind, DeviceSupport, PairingRequirement, Receiver,
    ReceiverCapabilities, ReceiverKind, UnsupportedReason,
};
