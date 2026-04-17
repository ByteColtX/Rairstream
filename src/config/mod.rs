//! 配置层：保存用户选择与后续扩展所需的最小设置。

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const CONFIG_DIR_NAME: &str = "Rairstream";
const CONFIG_FILE_NAME: &str = "config.json";
const DEFAULT_SENDER_VOLUME_PERCENT: u16 = 100;
pub const MAX_SENDER_VOLUME_PERCENT: u16 = 400;

/// 已保存接收端凭据对应的认证恢复路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverAuthFlow {
    #[default]
    Modern,
    LegacyPin,
}

fn is_default_receiver_auth_flow(flow: &ReceiverAuthFlow) -> bool {
    *flow == ReceiverAuthFlow::Modern
}

const fn default_sender_volume_percent() -> u16 {
    DEFAULT_SENDER_VOLUME_PERCENT
}

fn deserialize_sender_volume_percent<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let percent = u16::deserialize(deserializer)?;
    Ok(clamp_sender_volume_percent(percent))
}

const fn clamp_sender_volume_percent(percent: u16) -> u16 {
    if percent > MAX_SENDER_VOLUME_PERCENT {
        MAX_SENDER_VOLUME_PERCENT
    } else {
        percent
    }
}

/// `AirPlay` Receiver 接收端配对记录所需的长期身份材料。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverCredentials {
    #[serde(default, skip_serializing_if = "is_default_receiver_auth_flow")]
    pub auth_flow: ReceiverAuthFlow,
    pub controller_pairing_id: String,
    pub controller_ltpk_hex: String,
    pub controller_ltsk_hex: String,
    pub receiver_pairing_id: String,
    pub receiver_ltpk_hex: String,
}

/// 桌面端持久化配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub auto_reconnect: bool,
    #[serde(default)]
    pub launch_at_startup: bool,
    pub preferred_device_id: Option<String>,
    #[serde(
        default = "default_sender_volume_percent",
        deserialize_with = "deserialize_sender_volume_percent"
    )]
    pub sender_volume_percent: u16,
    #[serde(default)]
    pub sender_muted: bool,
    #[serde(default)]
    pub paired_receivers: HashMap<String, ReceiverCredentials>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            launch_at_startup: false,
            preferred_device_id: None,
            sender_volume_percent: DEFAULT_SENDER_VOLUME_PERCENT,
            sender_muted: false,
            paired_receivers: HashMap::new(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from_path(&default_config_path())
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                let mut value: serde_json::Value =
                    serde_json::from_str(&contents).map_err(|error| {
                        ConfigError::DeserializeFailed {
                            message: error.to_string(),
                        }
                    })?;
                migrate_missing_receiver_auth_flow_to_legacy_pin(&mut value);
                let mut config: Self = serde_json::from_value(value).map_err(|error| {
                    ConfigError::DeserializeFailed {
                        message: error.to_string(),
                    }
                })?;
                config.sender_volume_percent =
                    clamp_sender_volume_percent(config.sender_volume_percent);
                Ok(config)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(ConfigError::ReadFailed {
                message: error.to_string(),
            }),
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to_path(&default_config_path())
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), ConfigError> {
        let mut config = self.clone();
        config.sender_volume_percent = clamp_sender_volume_percent(config.sender_volume_percent);
        let mut value =
            serde_json::to_value(config).map_err(|error| ConfigError::SerializeFailed {
                message: error.to_string(),
            })?;
        persist_receiver_auth_flow(&mut value);
        let contents =
            serde_json::to_string_pretty(&value).map_err(|error| ConfigError::SerializeFailed {
                message: error.to_string(),
            })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ConfigError::WriteFailed {
                message: error.to_string(),
            })?;
        }
        fs::write(path, contents).map_err(|error| ConfigError::WriteFailed {
            message: error.to_string(),
        })
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

    pub fn set_preferred_device_id(&mut self, device_id: Option<String>) {
        self.preferred_device_id = device_id;
    }

    pub fn set_launch_at_startup(&mut self, enabled: bool) {
        self.launch_at_startup = enabled;
    }

    pub fn set_sender_volume_percent(&mut self, percent: u16) {
        self.sender_volume_percent = clamp_sender_volume_percent(percent);
    }

    pub fn set_sender_muted(&mut self, muted: bool) {
        self.sender_muted = muted;
    }
}

fn migrate_missing_receiver_auth_flow_to_legacy_pin(value: &mut serde_json::Value) {
    let Some(paired_receivers) = value
        .get_mut("paired_receivers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    for credentials in paired_receivers.values_mut() {
        let Some(credentials_object) = credentials.as_object_mut() else {
            continue;
        };
        if credentials_object.contains_key("auth_flow") {
            continue;
        }
        credentials_object.insert(
            String::from("auth_flow"),
            serde_json::Value::String(String::from("legacy_pin")),
        );
    }
}

fn persist_receiver_auth_flow(value: &mut serde_json::Value) {
    let Some(paired_receivers) = value
        .get_mut("paired_receivers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    for credentials in paired_receivers.values_mut() {
        let Some(credentials_object) = credentials.as_object_mut() else {
            continue;
        };
        if credentials_object.contains_key("auth_flow") {
            continue;
        }
        credentials_object.insert(
            String::from("auth_flow"),
            serde_json::Value::String(String::from("modern")),
        );
    }
}

fn default_config_path() -> PathBuf {
    default_config_dir().join(CONFIG_FILE_NAME)
}

fn default_config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(CONFIG_DIR_NAME);
    }

    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join(CONFIG_DIR_NAME.to_ascii_lowercase());
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join(CONFIG_DIR_NAME.to_ascii_lowercase());
    }

    PathBuf::from(".").join(".rairstream")
}

/// 配置层错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("读取配置失败: {message}")]
    ReadFailed { message: String },
    #[error("解析配置失败: {message}")]
    DeserializeFailed { message: String },
    #[error("序列化配置失败: {message}")]
    SerializeFailed { message: String },
    #[error("写入配置失败: {message}")]
    WriteFailed { message: String },
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, DEFAULT_SENDER_VOLUME_PERCENT, ReceiverAuthFlow, ReceiverCredentials};

    #[test]
    fn default_config_enables_auto_reconnect() {
        let config = AppConfig::default();

        assert!(config.auto_reconnect);
        assert!(config.preferred_device_id.is_none());
        assert_eq!(config.sender_volume_percent, DEFAULT_SENDER_VOLUME_PERCENT);
        assert!(config.paired_receivers.is_empty());
    }

    #[test]
    fn receiver_credentials_preserve_identity_material() {
        let credentials = ReceiverCredentials {
            auth_flow: ReceiverAuthFlow::Modern,
            controller_pairing_id: String::from("controller-id"),
            controller_ltpk_hex: String::from("aa"),
            controller_ltsk_hex: String::from("bb"),
            receiver_pairing_id: String::from("receiver-id"),
            receiver_ltpk_hex: String::from("cc"),
        };

        assert_eq!(credentials.controller_pairing_id, "controller-id");
        assert_eq!(credentials.receiver_pairing_id, "receiver-id");
    }

    #[test]
    fn set_sender_volume_percent_clamps_out_of_range_values() {
        let mut config = AppConfig::default();

        config.set_sender_volume_percent(401);

        assert_eq!(config.sender_volume_percent, 400);
    }

    #[test]
    fn remove_paired_receiver_deletes_existing_entry() {
        let mut config = AppConfig::default();
        config.upsert_paired_receiver(
            "receiver-1",
            ReceiverCredentials {
                auth_flow: ReceiverAuthFlow::Modern,
                controller_pairing_id: String::from("controller-id"),
                controller_ltpk_hex: String::from("11"),
                controller_ltsk_hex: String::from("22"),
                receiver_pairing_id: String::from("receiver-id"),
                receiver_ltpk_hex: String::from("33"),
            },
        );

        config.remove_paired_receiver("receiver-1");

        assert!(config.paired_receivers.is_empty());
    }
}
