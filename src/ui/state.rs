use crate::app::{AppState, SessionState, SpeakerDevice};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayAppState {
    pub app_state: AppState,
    pub devices: Vec<SpeakerDevice>,
    pub auto_reconnect: bool,
    pub launch_at_startup: bool,
    pub sender_volume_percent: u16,
    pub sender_muted: bool,
    pub last_error: Option<String>,
}

impl Default for TrayAppState {
    fn default() -> Self {
        Self {
            app_state: AppState::default(),
            devices: Vec::new(),
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayDeviceMenuItem {
    pub device_id: String,
    pub label: String,
    pub enabled: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayVolumeMenuItem {
    pub percent: u16,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuModel {
    pub status_label: String,
    pub refresh_enabled: bool,
    pub auto_reconnect_label: String,
    pub launch_at_startup_label: String,
    pub mute_label: String,
    pub muted: bool,
    pub volume_items: Vec<TrayVolumeMenuItem>,
    pub volume_warning_label: Option<String>,
    pub device_items: Vec<TrayDeviceMenuItem>,
    pub empty_label: Option<String>,
}

#[must_use]
pub fn build_tray_menu_model(state: &TrayAppState) -> TrayMenuModel {
    let refresh_enabled = !matches!(state.app_state.active_session, SessionState::Discovering);
    let device_items = state
        .devices
        .iter()
        .map(|device| build_device_menu_item(state, device, refresh_enabled))
        .collect::<Vec<_>>();
    let volume_items = build_volume_menu_items(state.sender_volume_percent);
    let volume_warning_label = build_volume_warning_label(state);
    let empty_label = if device_items.is_empty() {
        Some(String::from("未发现可用设备"))
    } else {
        None
    };

    TrayMenuModel {
        status_label: build_status_label(state),
        refresh_enabled,
        auto_reconnect_label: build_toggle_label("自动重连", state.auto_reconnect),
        launch_at_startup_label: build_toggle_label("开机启动", state.launch_at_startup),
        mute_label: if state.sender_muted {
            String::from("取消静音")
        } else {
            String::from("静音")
        },
        muted: state.sender_muted,
        volume_items,
        volume_warning_label,
        device_items,
        empty_label,
    }
}

fn build_toggle_label(title: &str, enabled: bool) -> String {
    if enabled {
        format!("{title}：开")
    } else {
        format!("{title}：关")
    }
}

fn build_volume_menu_items(selected_percent: u16) -> Vec<TrayVolumeMenuItem> {
    [0_u16, 25, 50, 75, 100, 125, 150, 200, 300, 400]
        .into_iter()
        .map(|percent| TrayVolumeMenuItem {
            percent,
            label: if percent == selected_percent {
                format!("● {percent}%")
            } else {
                format!("{percent}%")
            },
            selected: percent == selected_percent,
        })
        .collect()
}

fn build_volume_warning_label(state: &TrayAppState) -> Option<String> {
    if !state.sender_muted && state.sender_volume_percent > 100 {
        Some(String::from("音量已提升超过 100%，高电平时可能出现失真"))
    } else {
        None
    }
}

fn build_status_label(state: &TrayAppState) -> String {
    if let Some(last_error) = state.last_error.as_deref() {
        return format!("Rairstream：{last_error}");
    }

    match &state.app_state.active_session {
        SessionState::Idle => match state.app_state.selected_device_id.as_deref() {
            Some(device_id) => match resolve_device_name(&state.devices, device_id) {
                Some(device_name) => format!("Rairstream：已选择 {device_name}"),
                None => String::from("Rairstream：空闲"),
            },
            None => String::from("Rairstream：空闲"),
        },
        SessionState::Discovering => String::from("Rairstream：正在刷新设备"),
        SessionState::Connecting { device_id } => {
            let device_name = resolve_device_name(&state.devices, device_id).unwrap_or(device_id);
            format!("Rairstream：正在连接 {device_name}")
        }
        SessionState::AwaitingPairing { device_id } => {
            let device_name = resolve_device_name(&state.devices, device_id).unwrap_or(device_id);
            format!("Rairstream：等待配对 {device_name}")
        }
        SessionState::Authenticating { device_id } => {
            let device_name = resolve_device_name(&state.devices, device_id).unwrap_or(device_id);
            format!("Rairstream：正在认证 {device_name}")
        }
        SessionState::Streaming { device_id } => {
            let device_name = resolve_device_name(&state.devices, device_id).unwrap_or(device_id);
            format!("Rairstream：正在串流 {device_name}")
        }
    }
}

fn build_device_menu_item(
    state: &TrayAppState,
    device: &SpeakerDevice,
    refresh_enabled: bool,
) -> TrayDeviceMenuItem {
    let selected = state.app_state.selected_device_id.as_deref() == Some(device.id.as_str());
    let prefix = if selected { "● " } else { "" };
    let label = match &state.app_state.active_session {
        SessionState::Connecting { device_id } if device_id == &device.id => {
            format!("{prefix}{}（连接中）", device.name)
        }
        SessionState::AwaitingPairing { device_id } if device_id == &device.id => {
            format!("{prefix}{}（等待配对）", device.name)
        }
        SessionState::Authenticating { device_id } if device_id == &device.id => {
            format!("{prefix}{}（认证中）", device.name)
        }
        SessionState::Streaming { device_id } if device_id == &device.id => {
            format!("{prefix}{}（串流中）", device.name)
        }
        _ => format!("{prefix}{}", device.name),
    };

    TrayDeviceMenuItem {
        device_id: device.id.clone(),
        label,
        enabled: refresh_enabled,
        selected,
    }
}

fn resolve_device_name<'a>(devices: &'a [SpeakerDevice], device_id: &str) -> Option<&'a str> {
    devices
        .iter()
        .find(|device| device.id == device_id)
        .map(|device| device.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::{TrayAppState, build_tray_menu_model};
    use crate::app::{
        AirPlayGeneration, AppState, DeviceSupport, ReceiverKind, SessionState, SpeakerDevice,
    };

    fn build_device(id: &str, name: &str) -> SpeakerDevice {
        SpeakerDevice {
            id: id.to_string(),
            name: name.to_string(),
            host: String::from("192.168.1.10"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            pairing_id: None,
            receiver_public_key: None,
            receiver_kind: ReceiverKind::ClassicRaop,
            support: DeviceSupport::Supported,
        }
    }

    #[test]
    fn test_build_tray_menu_model_without_devices_shows_placeholder() {
        let model = build_tray_menu_model(&TrayAppState::default());

        assert_eq!(model.status_label, "Rairstream：空闲");
        assert!(model.refresh_enabled);
        assert_eq!(model.auto_reconnect_label, "自动重连：开");
        assert_eq!(model.launch_at_startup_label, "开机启动：关");
        assert_eq!(model.volume_items.len(), 10);
        assert_eq!(
            model
                .volume_items
                .iter()
                .map(|item| item.percent)
                .collect::<Vec<_>>(),
            vec![0, 25, 50, 75, 100, 125, 150, 200, 300, 400]
        );
        assert!(model.volume_items[4].selected);
        assert_eq!(model.volume_warning_label, None);
        assert!(model.device_items.is_empty());
        assert_eq!(model.empty_label.as_deref(), Some("未发现可用设备"));
    }

    #[test]
    fn test_build_tray_menu_model_marks_selected_device() {
        let state = TrayAppState {
            app_state: AppState {
                selected_device_id: Some(String::from("living-room")),
                active_session: SessionState::Idle,
            },
            devices: vec![build_device("living-room", "Living Room")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.status_label, "Rairstream：已选择 Living Room");
        assert_eq!(model.device_items.len(), 1);
        assert!(model.device_items[0].selected);
        assert_eq!(model.device_items[0].label, "● Living Room");
    }

    #[test]
    fn test_build_tray_menu_model_shows_connecting_status() {
        let state = TrayAppState {
            app_state: AppState {
                selected_device_id: Some(String::from("kitchen")),
                active_session: SessionState::Connecting {
                    device_id: String::from("kitchen"),
                },
            },
            devices: vec![build_device("kitchen", "Kitchen")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.status_label, "Rairstream：正在连接 Kitchen");
        assert_eq!(model.device_items[0].label, "● Kitchen（连接中）");
    }

    #[test]
    fn test_build_tray_menu_model_shows_pairing_and_authenticating_status() {
        let pairing_state = TrayAppState {
            app_state: AppState {
                selected_device_id: Some(String::from("kitchen")),
                active_session: SessionState::AwaitingPairing {
                    device_id: String::from("kitchen"),
                },
            },
            devices: vec![build_device("kitchen", "Kitchen")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        };
        let authenticating_state = TrayAppState {
            app_state: AppState {
                selected_device_id: Some(String::from("kitchen")),
                active_session: SessionState::Authenticating {
                    device_id: String::from("kitchen"),
                },
            },
            devices: vec![build_device("kitchen", "Kitchen")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        };

        let pairing_model = build_tray_menu_model(&pairing_state);
        let authenticating_model = build_tray_menu_model(&authenticating_state);

        assert_eq!(pairing_model.status_label, "Rairstream：等待配对 Kitchen");
        assert_eq!(pairing_model.device_items[0].label, "● Kitchen（等待配对）");
        assert_eq!(
            authenticating_model.status_label,
            "Rairstream：正在认证 Kitchen"
        );
        assert_eq!(
            authenticating_model.device_items[0].label,
            "● Kitchen（认证中）"
        );
    }

    #[test]
    fn test_build_tray_menu_model_disables_actions_while_discovering() {
        let state = TrayAppState {
            app_state: AppState {
                selected_device_id: None,
                active_session: SessionState::Discovering,
            },
            devices: vec![build_device("office", "Office")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.status_label, "Rairstream：正在刷新设备");
        assert!(!model.refresh_enabled);
        assert!(!model.device_items[0].enabled);
    }

    #[test]
    fn test_build_tray_menu_model_prioritizes_last_error() {
        let state = TrayAppState {
            app_state: AppState {
                selected_device_id: Some(String::from("kitchen")),
                active_session: SessionState::Authenticating {
                    device_id: String::from("kitchen"),
                },
            },
            devices: vec![build_device("kitchen", "Kitchen")],
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 100,
            sender_muted: false,
            last_error: Some(String::from("Kitchen 认证失败，请重新配对后再试")),
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(
            model.status_label,
            "Rairstream：Kitchen 认证失败，请重新配对后再试"
        );
    }

    #[test]
    fn test_build_tray_menu_model_marks_selected_volume_preset() {
        let state = TrayAppState {
            app_state: AppState::default(),
            devices: Vec::new(),
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 50,
            sender_muted: false,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.volume_items.len(), 10);
        assert_eq!(model.volume_items[2].percent, 50);
        assert!(model.volume_items[2].selected);
        assert_eq!(model.volume_items[2].label, "● 50%");
        assert_eq!(model.volume_warning_label, None);
    }

    #[test]
    fn test_build_tray_menu_model_includes_boosted_volume_presets() {
        let model = build_tray_menu_model(&TrayAppState::default());

        assert_eq!(model.volume_items[5].percent, 125);
        assert_eq!(model.volume_items[5].label, "125%");
        assert!(!model.volume_items[5].selected);
        assert_eq!(model.volume_items[6].percent, 150);
        assert_eq!(model.volume_items[6].label, "150%");
        assert!(!model.volume_items[6].selected);
        assert_eq!(model.volume_items[7].percent, 200);
        assert_eq!(model.volume_items[7].label, "200%");
        assert!(!model.volume_items[7].selected);
        assert_eq!(model.volume_items[8].percent, 300);
        assert_eq!(model.volume_items[8].label, "300%");
        assert!(!model.volume_items[8].selected);
        assert_eq!(model.volume_items[9].percent, 400);
        assert_eq!(model.volume_items[9].label, "400%");
        assert!(!model.volume_items[9].selected);
    }

    #[test]
    fn test_build_tray_menu_model_shows_volume_warning_for_boosted_unmuted_volume() {
        let state = TrayAppState {
            app_state: AppState::default(),
            devices: Vec::new(),
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 125,
            sender_muted: false,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(
            model.volume_warning_label.as_deref(),
            Some("音量已提升超过 100%，高电平时可能出现失真")
        );
    }

    #[test]
    fn test_build_tray_menu_model_hides_volume_warning_for_muted_boosted_volume() {
        let state = TrayAppState {
            app_state: AppState::default(),
            devices: Vec::new(),
            auto_reconnect: true,
            launch_at_startup: false,
            sender_volume_percent: 125,
            sender_muted: true,
            last_error: None,
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.volume_warning_label, None);
    }
}
