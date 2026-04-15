use rairstream::app::{
    AirPlayGeneration, AppState, DeviceSupport, ReceiverKind, SessionState, SpeakerDevice,
};
use rairstream::ui::{TrayAppState, build_tray_menu_model};

fn build_device(id: &str, name: &str) -> SpeakerDevice {
    SpeakerDevice {
        id: id.to_string(),
        name: name.to_string(),
        host: String::from("192.168.1.20"),
        port: 7000,
        generation: AirPlayGeneration::AirPlay1,
        pairing_id: None,
        receiver_public_key: None,
        receiver_kind: ReceiverKind::ClassicRaop,
        support: DeviceSupport::Supported,
    }
}

#[test]
fn test_menu_model_without_devices_shows_placeholder() {
    let model = build_tray_menu_model(&TrayAppState::default());

    assert_eq!(model.status_label, "Rairstream：空闲");
    assert!(model.refresh_enabled);
    assert!(model.device_items.is_empty());
    assert_eq!(model.empty_label.as_deref(), Some("未发现可用设备"));
}

#[test]
fn test_menu_model_for_idle_selection_shows_selected_device_label() {
    let state = TrayAppState {
        app_state: AppState {
            selected_device_id: Some(String::from("living-room")),
            active_session: SessionState::Idle,
        },
        devices: vec![build_device("living-room", "Living Room")],
    };

    let model = build_tray_menu_model(&state);

    assert_eq!(model.status_label, "Rairstream：已选择 Living Room");
    assert!(model.device_items[0].selected);
    assert_eq!(model.device_items[0].label, "● Living Room");
}

#[test]
fn test_menu_model_for_connection_related_states_marks_device_status() {
    let connecting_state = TrayAppState {
        app_state: AppState {
            selected_device_id: Some(String::from("bedroom")),
            active_session: SessionState::Connecting {
                device_id: String::from("bedroom"),
            },
        },
        devices: vec![build_device("bedroom", "Bedroom")],
    };
    let pairing_state = TrayAppState {
        app_state: AppState {
            selected_device_id: Some(String::from("bedroom")),
            active_session: SessionState::AwaitingPairing {
                device_id: String::from("bedroom"),
            },
        },
        devices: vec![build_device("bedroom", "Bedroom")],
    };
    let authenticating_state = TrayAppState {
        app_state: AppState {
            selected_device_id: Some(String::from("bedroom")),
            active_session: SessionState::Authenticating {
                device_id: String::from("bedroom"),
            },
        },
        devices: vec![build_device("bedroom", "Bedroom")],
    };
    let streaming_state = TrayAppState {
        app_state: AppState {
            selected_device_id: Some(String::from("bedroom")),
            active_session: SessionState::Streaming {
                device_id: String::from("bedroom"),
            },
        },
        devices: vec![build_device("bedroom", "Bedroom")],
    };

    let connecting_model = build_tray_menu_model(&connecting_state);
    let pairing_model = build_tray_menu_model(&pairing_state);
    let authenticating_model = build_tray_menu_model(&authenticating_state);
    let streaming_model = build_tray_menu_model(&streaming_state);

    assert_eq!(
        connecting_model.status_label,
        "Rairstream：正在连接 Bedroom"
    );
    assert_eq!(
        connecting_model.device_items[0].label,
        "● Bedroom（连接中）"
    );
    assert_eq!(pairing_model.status_label, "Rairstream：等待配对 Bedroom");
    assert_eq!(pairing_model.device_items[0].label, "● Bedroom（等待配对）");
    assert_eq!(
        authenticating_model.status_label,
        "Rairstream：正在认证 Bedroom"
    );
    assert_eq!(
        authenticating_model.device_items[0].label,
        "● Bedroom（认证中）"
    );
    assert_eq!(streaming_model.status_label, "Rairstream：正在串流 Bedroom");
    assert_eq!(streaming_model.device_items[0].label, "● Bedroom（串流中）");
}

#[test]
fn test_menu_model_for_discovering_disables_refresh_and_device_actions() {
    let state = TrayAppState {
        app_state: AppState {
            selected_device_id: None,
            active_session: SessionState::Discovering,
        },
        devices: vec![build_device("office", "Office")],
    };

    let model = build_tray_menu_model(&state);

    assert_eq!(model.status_label, "Rairstream：正在刷新设备");
    assert!(!model.refresh_enabled);
    assert!(!model.device_items[0].enabled);
}
