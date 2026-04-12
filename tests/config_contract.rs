use rairstream::config::AppConfig;
use serde_json::json;

#[test]
fn test_default_config_contract() {
    let config = AppConfig::default();

    assert!(config.auto_reconnect);
    assert!(config.preferred_device_id.is_none());
}

#[test]
fn test_config_serializes_expected_json_shape() {
    let config = AppConfig {
        auto_reconnect: false,
        preferred_device_id: Some(String::from("living-room")),
    };

    let value = serde_json::to_value(&config).expect("config should serialize to JSON value");

    assert_eq!(
        value,
        json!({
            "auto_reconnect": false,
            "preferred_device_id": "living-room"
        })
    );
}

#[test]
fn test_config_deserializes_explicit_values() {
    let config: AppConfig = serde_json::from_value(json!({
        "auto_reconnect": false,
        "preferred_device_id": "office-speaker"
    }))
    .expect("config should deserialize from explicit JSON values");

    assert!(!config.auto_reconnect);
    assert_eq!(
        config.preferred_device_id.as_deref(),
        Some("office-speaker")
    );
}

#[test]
fn test_config_rejects_missing_required_field() {
    let error = serde_json::from_value::<AppConfig>(json!({
        "preferred_device_id": null
    }))
    .expect_err("config should reject missing auto_reconnect field");

    assert!(error.to_string().contains("auto_reconnect"));
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
