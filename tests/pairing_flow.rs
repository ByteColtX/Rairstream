use std::time::{SystemTime, UNIX_EPOCH};

use rairstream::config::{AppConfig, load_config, save_config};
use rairstream::pairing::{ReceiverAuthFlow, ReceiverCredentials};

fn temp_config_path() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("rairstream-config-{unique}.json"))
}

#[test]
fn paired_credentials_round_trip_through_config_file() {
    let path = temp_config_path();
    let mut config = AppConfig::default();
    config.upsert_paired_receiver(
        "receiver-1",
        ReceiverCredentials {
            auth_flow: ReceiverAuthFlow::Modern,
            controller_pairing_id: String::from("controller"),
            controller_ltpk_hex: String::from("aa"),
            controller_ltsk_hex: String::from("bb"),
            receiver_pairing_id: String::from("receiver"),
            receiver_ltpk_hex: String::from("cc"),
        },
    );

    save_config(&path, &config).unwrap();
    let loaded = load_config(&path).unwrap();

    assert_eq!(loaded.paired_receivers.len(), 1);
    let _ = std::fs::remove_file(path);
}
