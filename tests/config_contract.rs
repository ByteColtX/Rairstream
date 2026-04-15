use rairstream::config::{AppConfig, ReceiverAuthFlow, ReceiverCredentials};
use serde_json::json;

fn build_receiver_credentials() -> ReceiverCredentials {
    ReceiverCredentials {
        auth_flow: ReceiverAuthFlow::Modern,
        controller_pairing_id: String::from("controller-id"),
        controller_ltpk_hex: String::from("11"),
        controller_ltsk_hex: String::from("22"),
        receiver_pairing_id: String::from("receiver-id"),
        receiver_ltpk_hex: String::from("33"),
    }
}

#[test]
fn test_config_serializes_legacy_pin_auth_flow_when_non_default() {
    let mut config = AppConfig {
        auto_reconnect: true,
        preferred_device_id: None,
        paired_receivers: std::collections::HashMap::new(),
    };
    config.paired_receivers.insert(
        String::from("receiver-legacy"),
        ReceiverCredentials {
            auth_flow: ReceiverAuthFlow::LegacyPin,
            controller_pairing_id: String::from("controller-id"),
            controller_ltpk_hex: String::from("11"),
            controller_ltsk_hex: String::from("22"),
            receiver_pairing_id: String::from("receiver-id"),
            receiver_ltpk_hex: String::from("33"),
        },
    );

    let value = serde_json::to_value(&config).expect("config should serialize to JSON value");

    assert_eq!(
        value,
        json!({
            "auto_reconnect": true,
            "preferred_device_id": null,
            "paired_receivers": {
                "receiver-legacy": {
                    "auth_flow": "legacy_pin",
                    "controller_pairing_id": "controller-id",
                    "controller_ltpk_hex": "11",
                    "controller_ltsk_hex": "22",
                    "receiver_pairing_id": "receiver-id",
                    "receiver_ltpk_hex": "33"
                }
            }
        })
    );
}

#[test]
fn test_default_config_contract() {
    let config = AppConfig::default();

    assert!(config.auto_reconnect);
    assert!(config.preferred_device_id.is_none());
    assert!(config.paired_receivers.is_empty());
}

#[test]
fn test_config_serializes_expected_json_shape() {
    let mut config = AppConfig {
        auto_reconnect: false,
        preferred_device_id: Some(String::from("living-room")),
        paired_receivers: std::collections::HashMap::new(),
    };
    config
        .paired_receivers
        .insert(String::from("receiver-1"), build_receiver_credentials());

    let value = serde_json::to_value(&config).expect("config should serialize to JSON value");

    assert_eq!(
        value,
        json!({
            "auto_reconnect": false,
            "preferred_device_id": "living-room",
            "paired_receivers": {
                "receiver-1": {
                    "controller_pairing_id": "controller-id",
                    "controller_ltpk_hex": "11",
                    "controller_ltsk_hex": "22",
                    "receiver_pairing_id": "receiver-id",
                    "receiver_ltpk_hex": "33"
                }
            }
        })
    );
}

#[test]
fn test_config_deserializes_explicit_values() {
    let config: AppConfig = serde_json::from_value(json!({
        "auto_reconnect": false,
        "preferred_device_id": "office-speaker",
        "paired_receivers": {
            "receiver-1": {
                "controller_pairing_id": "controller-id",
                "controller_ltpk_hex": "11",
                "controller_ltsk_hex": "22",
                "receiver_pairing_id": "receiver-id",
                "receiver_ltpk_hex": "33"
            }
        }
    }))
    .expect("config should deserialize from explicit JSON values");

    assert!(!config.auto_reconnect);
    assert_eq!(
        config.preferred_device_id.as_deref(),
        Some("office-speaker")
    );
    let credentials = config
        .paired_receivers
        .get("receiver-1")
        .expect("receiver-1 credentials should exist");
    assert_eq!(credentials.auth_flow, ReceiverAuthFlow::Modern);
    assert_eq!(credentials.controller_pairing_id, "controller-id");
    assert_eq!(credentials.receiver_pairing_id, "receiver-id");
}

#[test]
fn test_config_load_migrates_missing_auth_flow_to_legacy_pin() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rairstream-config-migrate-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir(&temp_dir).expect("temp dir should be creatable");
    let path = temp_dir.join("config.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "auto_reconnect": true,
            "preferred_device_id": null,
            "paired_receivers": {
                "receiver-legacy": {
                    "controller_pairing_id": "controller-id",
                    "controller_ltpk_hex": "11",
                    "controller_ltsk_hex": "22",
                    "receiver_pairing_id": "receiver-id",
                    "receiver_ltpk_hex": "33"
                }
            }
        }))
        .expect("config JSON should serialize"),
    )
    .expect("legacy config should be writable");

    let loaded = AppConfig::load_from_path(&path).expect("config should load from temp path");
    let credentials = loaded
        .paired_receivers
        .get("receiver-legacy")
        .expect("receiver-legacy credentials should exist");

    assert_eq!(credentials.auth_flow, ReceiverAuthFlow::LegacyPin);

    std::fs::remove_file(&path).expect("temp config file should be removable");
    std::fs::remove_dir(&temp_dir).expect("temp config dir should be removable");
}

#[test]
fn test_config_rejects_missing_required_field() {
    let error = serde_json::from_value::<AppConfig>(json!({
        "preferred_device_id": null,
        "paired_receivers": {}
    }))
    .expect_err("config should reject missing auto_reconnect field");

    assert!(error.to_string().contains("auto_reconnect"));
}

#[test]
fn test_config_defaults_missing_paired_receivers_to_empty_map() {
    let config: AppConfig = serde_json::from_value(json!({
        "auto_reconnect": true,
        "preferred_device_id": null
    }))
    .expect("config should default missing paired_receivers");

    assert!(config.paired_receivers.is_empty());
}

#[test]
fn test_config_load_save_round_trip_preserves_paired_receivers() {
    let temp_dir = std::env::temp_dir().join(format!(
        "rairstream-config-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos()
    ));
    let path = temp_dir.join("config.json");
    let mut config = AppConfig {
        auto_reconnect: false,
        preferred_device_id: Some(String::from("living-room")),
        paired_receivers: std::collections::HashMap::new(),
    };
    config.upsert_paired_receiver("receiver-1", build_receiver_credentials());

    config
        .save_to_path(&path)
        .expect("config should save to temp path");
    let loaded = AppConfig::load_from_path(&path).expect("config should load from temp path");

    assert_eq!(loaded, config);

    std::fs::remove_file(&path).expect("temp config file should be removable");
    std::fs::remove_dir(&temp_dir).expect("temp config dir should be removable");
}

#[test]
fn test_config_rejects_invalid_preferred_device_type() {
    let error = serde_json::from_value::<AppConfig>(json!({
        "auto_reconnect": true,
        "preferred_device_id": 42
    }))
    .expect_err("config should reject non-string preferred_device_id");

    assert!(error.to_string().contains("invalid type"));
}
