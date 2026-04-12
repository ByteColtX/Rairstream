//! 配置层：保存用户选择与后续扩展所需的最小设置。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 桌面端持久化配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub auto_reconnect: bool,
    pub preferred_device_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            preferred_device_id: None,
        }
    }
}

/// 配置层错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("当前配置尚未接入持久化存储")]
    PersistenceNotReady,
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn default_config_enables_auto_reconnect() {
        let config = AppConfig::default();

        assert!(config.auto_reconnect);
        assert!(config.preferred_device_id.is_none());
    }
}
