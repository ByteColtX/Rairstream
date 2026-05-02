use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverAuthFlow {
    #[default]
    Modern,
    LegacyPin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverCredentials {
    #[serde(default)]
    pub auth_flow: ReceiverAuthFlow,
    pub controller_pairing_id: String,
    pub controller_ltpk_hex: String,
    pub controller_ltsk_hex: String,
    pub receiver_pairing_id: String,
    pub receiver_ltpk_hex: String,
}
