use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::model::AppConfig;

const CONFIG_DIR_NAME: &str = "Rairstream";
const CONFIG_FILE_NAME: &str = "config.json";

pub fn default_config_path() -> PathBuf {
    default_config_dir().join(CONFIG_FILE_NAME)
}

pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            serde_json::from_str(&contents).map_err(|error| ConfigError::DeserializeFailed {
                message: error.to_string(),
            })
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(error) => Err(ConfigError::ReadFailed {
            message: error.to_string(),
        }),
    }
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| ConfigError::WriteFailed {
            message: error.to_string(),
        })?;
    }

    let contents =
        serde_json::to_string_pretty(config).map_err(|error| ConfigError::SerializeFailed {
            message: error.to_string(),
        })?;
    fs::write(path, contents).map_err(|error| ConfigError::WriteFailed {
        message: error.to_string(),
    })
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

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {message}")]
    ReadFailed { message: String },
    #[error("failed to parse config: {message}")]
    DeserializeFailed { message: String },
    #[error("failed to serialize config: {message}")]
    SerializeFailed { message: String },
    #[error("failed to write config: {message}")]
    WriteFailed { message: String },
}
