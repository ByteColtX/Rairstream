use crate::config::AppConfig;
use crate::pairing::ReceiverCredentials;

pub fn get_paired_receiver<'a>(
    config: &'a AppConfig,
    receiver_id: &str,
) -> Option<&'a ReceiverCredentials> {
    config.paired_receivers.get(receiver_id)
}
