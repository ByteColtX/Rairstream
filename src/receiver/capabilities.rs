use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecKind {
    L16,
    Alac,
    Aac,
    AacEld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingRequirement {
    None,
    LegacyPin,
    PinOrCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverCapabilities {
    pub codecs: Vec<CodecKind>,
    pub pairing: PairingRequirement,
    pub supports_multiroom: bool,
    pub supports_ptp: bool,
    pub supports_retransmit: bool,
}

impl Default for ReceiverCapabilities {
    fn default() -> Self {
        Self {
            codecs: vec![CodecKind::L16],
            pairing: PairingRequirement::None,
            supports_multiroom: false,
            supports_ptp: false,
            supports_retransmit: true,
        }
    }
}
