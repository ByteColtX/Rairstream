use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::pairing::ReceiverCredentials;
use crate::receiver::TransportProfile;

pub const DEFAULT_SENDER_VOLUME_PERCENT: u16 = 100;
pub const MAX_SENDER_VOLUME_PERCENT: u16 = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedReceiver {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub transport_profile: TransportProfile,
    #[doc(hidden)]
    #[serde(default)]
    pub receiver_kind: TransportProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_sender_volume_percent")]
    pub sender_volume_percent: u16,
    #[serde(default)]
    pub paired_receivers: HashMap<String, ReceiverCredentials>,
    #[serde(default)]
    pub receiver_cache: HashMap<String, CachedReceiver>,
    #[serde(default)]
    pub tray_selected_receiver_ids: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sender_volume_percent: DEFAULT_SENDER_VOLUME_PERCENT,
            paired_receivers: HashMap::new(),
            receiver_cache: HashMap::new(),
            tray_selected_receiver_ids: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn set_sender_volume_percent(&mut self, percent: u16) {
        self.sender_volume_percent = percent.min(MAX_SENDER_VOLUME_PERCENT);
    }

    pub fn upsert_paired_receiver(
        &mut self,
        device_id: impl Into<String>,
        receiver_credentials: ReceiverCredentials,
    ) {
        self.paired_receivers
            .insert(device_id.into(), receiver_credentials);
    }

    pub fn remove_paired_receiver(&mut self, device_id: &str) {
        self.paired_receivers.remove(device_id);
    }

    pub fn upsert_receiver_cache(&mut self, receiver: CachedReceiver) {
        self.receiver_cache.insert(receiver.id.clone(), receiver);
    }

    pub fn set_tray_selected_receiver_ids(&mut self, receiver_ids: Vec<String>) {
        self.tray_selected_receiver_ids = receiver_ids;
    }
}

const fn default_sender_volume_percent() -> u16 {
    DEFAULT_SENDER_VOLUME_PERCENT
}
