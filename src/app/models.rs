use crate::pairing::ReceiverAuthFlow;
use crate::receiver::Receiver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Discovering,
    Pairing { receiver_id: String },
    Streaming { receiver_ids: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    pub session: SessionState,
    pub last_receivers: Vec<Receiver>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::Idle,
            last_receivers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectResult {
    pub receiver: Receiver,
    pub has_stored_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedReceiverEntry {
    pub receiver_id: String,
    pub display_name: Option<String>,
    pub auth_flow: ReceiverAuthFlow,
}
