use crate::config::{AppConfig, CachedReceiver};
use crate::receiver::Receiver;

pub fn cache_receivers(config: &mut AppConfig, receivers: &[Receiver]) {
    for receiver in receivers {
        config.upsert_receiver_cache(CachedReceiver {
            id: receiver.id.clone(),
            name: receiver.name.clone(),
            host: receiver.host.clone(),
            port: receiver.port,
            transport_profile: receiver.transport_profile,
            receiver_kind: receiver.transport_profile,
        });
    }
}
