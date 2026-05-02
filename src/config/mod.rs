mod file;
mod model;

pub use file::{ConfigError, default_config_path, load_config, save_config};
pub use model::{
    AppConfig, CachedReceiver, DEFAULT_SENDER_VOLUME_PERCENT, MAX_SENDER_VOLUME_PERCENT,
};
