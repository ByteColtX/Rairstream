use crate::config::{AppConfig, CachedReceiver};
use crate::receiver::Receiver;

pub fn cache_receivers(config: &mut AppConfig, receivers: &[Receiver]) {
    for receiver in receivers {
        config.upsert_receiver_cache(CachedReceiver {
            id: receiver.id.clone(),
            name: receiver.name.clone(),
            host: receiver.host.clone(),
            port: receiver.port,
            receiver_kind: receiver.receiver_kind,
        });
    }
}
