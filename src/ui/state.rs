use crate::app::{AppState, SessionState, SpeakerDevice};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayAppState {
    pub app_state: AppState,
    pub devices: Vec<SpeakerDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayDeviceMenuItem {
    pub device_id: String,
    pub label: String,
    pub enabled: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuModel {
    pub status_label: String,
    pub refresh_enabled: bool,
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
    let empty_label = if device_items.is_empty() {
        Some(String::from("未发现可用设备"))
    } else {
        None
    };

    TrayMenuModel {
        status_label: build_status_label(state),
        refresh_enabled,
        device_items,
        empty_label,
    }
}

fn build_status_label(state: &TrayAppState) -> String {
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
    use crate::app::{AirPlayGeneration, AppState, SessionState, SpeakerDevice};

    fn build_device(id: &str, name: &str) -> SpeakerDevice {
        SpeakerDevice {
            id: id.to_string(),
            name: name.to_string(),
            host: String::from("192.168.1.10"),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
        }
    }

    #[test]
    fn test_build_tray_menu_model_without_devices_shows_placeholder() {
        let model = build_tray_menu_model(&TrayAppState::default());

        assert_eq!(model.status_label, "Rairstream：空闲");
        assert!(model.refresh_enabled);
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
        };

        let model = build_tray_menu_model(&state);

        assert_eq!(model.status_label, "Rairstream：正在连接 Kitchen");
        assert_eq!(model.device_items[0].label, "● Kitchen（连接中）");
    }

    #[test]
    fn test_build_tray_menu_model_disables_actions_while_discovering() {
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
}
