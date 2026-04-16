use super::TrayUiError;
use super::controller::TrayController;
use super::state::TrayMenuModel;
use crate::app::{RairstreamError, SessionCoordinator, SessionState};
use crate::config::AppConfig;
use crate::discovery::DiscoveryService;
use inputbox::{InputBox, InputMode};
use std::collections::HashMap;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tracing::{error, info, warn};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayAction {
    RefreshDevices,
    ToggleAutoReconnect,
    ToggleLaunchAtStartup,
    SelectDevice(String),
    ToggleSenderMute,
    SetSenderVolume(u8),
    StopStreaming,
    ShowAbout,
    Quit,
}

struct TrayMenuState {
    menu: Menu,
    action_map: HashMap<MenuId, TrayAction>,
}

impl TrayMenuState {
    fn new(
        model: &TrayMenuModel,
        controller: &TrayController<impl crate::app::SessionControlService>,
    ) -> Result<Self, TrayUiError> {
        let mut state = Self {
            menu: Menu::new(),
            action_map: HashMap::new(),
        };
        state.sync(model, controller)?;
        Ok(state)
    }

    fn sync(
        &mut self,
        model: &TrayMenuModel,
        controller: &TrayController<impl crate::app::SessionControlService>,
    ) -> Result<(), TrayUiError> {
        clear_menu(&self.menu);
        rebuild_menu(&self.menu, &mut self.action_map, model, controller)
    }
}

pub fn run_tray_app<D>(
    coordinator: SessionCoordinator<D>,
    config: AppConfig,
) -> Result<(), TrayUiError>
where
    D: DiscoveryService + 'static,
{
    if !cfg!(target_os = "windows") {
        return Err(TrayUiError::UnsupportedPlatform);
    }

    let mut controller = TrayController::new(coordinator, config);
    let initial_model = controller.initialize();
    let mut menu_state = TrayMenuState::new(&initial_model, &controller)?;
    let icon = build_icon()?;
    let mut event_loop_builder = EventLoopBuilder::<MenuEvent>::with_user_event();
    let event_loop = event_loop_builder.build();
    let proxy = event_loop.create_proxy();

    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(event);
    }));

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip(initial_model.status_label.clone())
        .with_menu(Box::new(menu_state.menu.clone()))
        .with_menu_on_left_click(true)
        .with_icon(icon)
        .build()
        .map_err(|error| TrayUiError::CreateTrayIcon {
            message: error.to_string(),
        })?;

    event_loop.run(move |event, _window_target, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::UserEvent(menu_event) = event {
            handle_menu_event(
                &mut controller,
                &tray_icon,
                &mut menu_state,
                &menu_event.id,
                control_flow,
            );
        }
    })
}

fn handle_menu_event(
    controller: &mut TrayController<impl crate::app::SessionControlService>,
    tray_icon: &TrayIcon,
    menu_state: &mut TrayMenuState,
    menu_id: &MenuId,
    control_flow: &mut ControlFlow,
) {
    let model = match menu_state.action_map.get(menu_id).cloned() {
        Some(TrayAction::RefreshDevices) => {
            info!("托盘菜单触发刷新设备");
            controller.refresh_devices()
        }
        Some(TrayAction::ToggleAutoReconnect) => match controller.toggle_auto_reconnect() {
            Ok(model) => model,
            Err(error) => {
                warn!(error = %error, "切换自动重连失败，保留当前菜单状态");
                controller.handle_error(None, &error);
                controller.menu_model()
            }
        },
        Some(TrayAction::ToggleLaunchAtStartup) => match controller.toggle_launch_at_startup() {
            Ok(model) => model,
            Err(error) => {
                warn!(error = %error, "切换开机启动失败，保留当前菜单状态");
                controller.handle_error(None, &error);
                controller.menu_model()
            }
        },
        Some(TrayAction::SelectDevice(device_id)) => handle_select_device(controller, &device_id),
        Some(TrayAction::ToggleSenderMute) => match controller.toggle_sender_mute() {
            Ok(model) => model,
            Err(error) => {
                warn!(error = %error, "切换发送端静音失败，保留当前菜单状态");
                controller.handle_error(None, &error);
                controller.menu_model()
            }
        },
        Some(TrayAction::SetSenderVolume(percent)) => match controller.set_sender_volume(percent) {
            Ok(model) => model,
            Err(error) => {
                warn!(percent, error = %error, "设置发送端音量失败，保留当前菜单状态");
                controller.handle_error(None, &error);
                controller.menu_model()
            }
        },
        Some(TrayAction::StopStreaming) => {
            info!("托盘菜单触发停止串流");
            match controller.stop_streaming() {
                Ok(model) => model,
                Err(error) => {
                    warn!(error = %error, "停止串流失败，保留当前菜单状态");
                    controller.menu_model()
                }
            }
        }
        Some(TrayAction::ShowAbout) => {
            info!("托盘菜单触发关于");
            if let Err(error) = show_about_dialog() {
                warn!(error = %error, "显示关于弹窗失败，保留当前菜单状态");
                controller.handle_error(
                    None,
                    &RairstreamError::InvalidConfiguration {
                        message: error.to_string(),
                    },
                );
                let model = controller.menu_model();
                if let Err(error) = sync_menu_model(tray_icon, menu_state, &model, controller) {
                    error!(error = %error, "应用托盘菜单状态失败，准备退出事件循环");
                    *control_flow = ControlFlow::ExitWithCode(1);
                }
            }
            return;
        }
        Some(TrayAction::Quit) => {
            info!("托盘菜单触发退出");
            if let Err(error) = controller.stop_streaming() {
                warn!(error = %error, "退出前停止串流失败");
            }
            *control_flow = ControlFlow::Exit;
            return;
        }
        None => return,
    };

    if let Err(error) = sync_menu_model(tray_icon, menu_state, &model, controller) {
        error!(error = %error, "应用托盘菜单状态失败，准备退出事件循环");
        *control_flow = ControlFlow::ExitWithCode(1);
    }
}

fn handle_select_device(
    controller: &mut TrayController<impl crate::app::SessionControlService>,
    device_id: &str,
) -> TrayMenuModel {
    info!(device_id, "托盘菜单触发设备选择");
    match controller.select_device(device_id) {
        Ok(model) => {
            let awaiting_pairing_device_id = match &controller.state().app_state.active_session {
                SessionState::AwaitingPairing { device_id } => Some(device_id.clone()),
                _ => None,
            };
            match awaiting_pairing_device_id {
                Some(awaiting_pairing_device_id) => {
                    handle_pairing_prompt(controller, &awaiting_pairing_device_id)
                }
                None => model,
            }
        }
        Err(error) => {
            warn!(device_id, error = %error, "处理设备选择失败，保留当前菜单状态");
            controller.handle_error(Some(device_id), &error);
            controller.menu_model()
        }
    }
}

fn handle_pairing_prompt(
    controller: &mut TrayController<impl crate::app::SessionControlService>,
    device_id: &str,
) -> TrayMenuModel {
    match prompt_pairing_pin(controller, device_id) {
        Ok(Some(pin)) => match controller.submit_pairing_pin(device_id, &pin) {
            Ok(model) => model,
            Err(error) => {
                warn!(device_id, error = %error, "提交配对 PIN 失败，保留当前菜单状态");
                controller.handle_error(Some(device_id), &error);
                controller.menu_model()
            }
        },
        Ok(None) => match controller.cancel_pairing(device_id) {
            Ok(model) => model,
            Err(error) => {
                warn!(device_id, error = %error, "取消配对输入失败，保留当前菜单状态");
                controller.handle_error(Some(device_id), &error);
                controller.menu_model()
            }
        },
        Err(error) => {
            warn!(device_id, error = %error, "拉起 PIN 输入框失败，保留当前菜单状态");
            controller.handle_error(
                Some(device_id),
                &RairstreamError::InvalidConfiguration {
                    message: error.to_string(),
                },
            );
            controller.menu_model()
        }
    }
}

fn clear_menu(menu: &Menu) {
    while menu.remove_at(0).is_some() {}
}

fn rebuild_menu(
    menu: &Menu,
    action_map: &mut HashMap<MenuId, TrayAction>,
    model: &TrayMenuModel,
    controller: &TrayController<impl crate::app::SessionControlService>,
) -> Result<(), TrayUiError> {
    action_map.clear();

    append_static_item(menu, model.status_label.as_str(), false)?;
    append_separator(menu)?;

    append_action_item(
        menu,
        action_map,
        "刷新设备",
        model.refresh_enabled,
        TrayAction::RefreshDevices,
    )?;
    append_action_item(
        menu,
        action_map,
        model.auto_reconnect_label.as_str(),
        true,
        TrayAction::ToggleAutoReconnect,
    )?;
    append_action_item(
        menu,
        action_map,
        model.launch_at_startup_label.as_str(),
        true,
        TrayAction::ToggleLaunchAtStartup,
    )?;

    if matches!(
        controller.state().app_state.active_session,
        SessionState::Streaming { .. }
    ) {
        append_action_item(
            menu,
            action_map,
            "停止串流",
            true,
            TrayAction::StopStreaming,
        )?;
    }

    append_static_item(menu, "发送音量", false)?;
    append_action_item(
        menu,
        action_map,
        model.mute_label.as_str(),
        true,
        TrayAction::ToggleSenderMute,
    )?;
    for volume_item in &model.volume_items {
        append_action_item(
            menu,
            action_map,
            volume_item.label.as_str(),
            true,
            TrayAction::SetSenderVolume(volume_item.percent),
        )?;
    }

    append_separator(menu)?;

    match model.empty_label.as_deref() {
        Some(empty_label) => append_static_item(menu, empty_label, false)?,
        None => {
            for device_item in &model.device_items {
                append_action_item(
                    menu,
                    action_map,
                    device_item.label.as_str(),
                    device_item.enabled,
                    TrayAction::SelectDevice(device_item.device_id.clone()),
                )?;
            }
        }
    }

    append_separator(menu)?;
    append_action_item(menu, action_map, "关于", true, TrayAction::ShowAbout)?;
    append_action_item(menu, action_map, "退出", true, TrayAction::Quit)?;

    Ok(())
}

fn append_static_item(menu: &Menu, label: &str, enabled: bool) -> Result<(), TrayUiError> {
    let item = MenuItem::new(label, enabled, None);
    menu.append(&item)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;
    Ok(())
}

fn append_action_item(
    menu: &Menu,
    action_map: &mut HashMap<MenuId, TrayAction>,
    label: &str,
    enabled: bool,
    action: TrayAction,
) -> Result<(), TrayUiError> {
    let item = MenuItem::new(label, enabled, None);
    action_map.insert(item.id().clone(), action);
    menu.append(&item)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;
    Ok(())
}

fn append_separator(menu: &Menu) -> Result<(), TrayUiError> {
    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;
    Ok(())
}

fn sync_menu_model(
    tray_icon: &TrayIcon,
    menu_state: &mut TrayMenuState,
    model: &TrayMenuModel,
    controller: &TrayController<impl crate::app::SessionControlService>,
) -> Result<(), TrayUiError> {
    menu_state.sync(model, controller)?;
    tray_icon
        .set_tooltip(Some(model.status_label.as_str()))
        .map_err(|error| TrayUiError::UpdateTooltip {
            message: error.to_string(),
        })?;
    Ok(())
}

fn prompt_pairing_pin(
    controller: &TrayController<impl crate::app::SessionControlService>,
    device_id: &str,
) -> Result<Option<String>, TrayUiError> {
    let state = controller.state();
    let device_name = state
        .devices
        .iter()
        .find(|device| device.id == device_id)
        .map_or(device_id, |device| device.name.as_str());
    InputBox::new()
        .title("Rairstream 配对")
        .prompt(format!("请输入 {device_name} 当前显示的 AirPlay PIN"))
        .ok_label("继续配对")
        .cancel_label("取消")
        .show()
        .map_err(|error| TrayUiError::Initialize {
            message: error.to_string(),
        })
}

fn show_about_dialog() -> Result<(), TrayUiError> {
    InputBox::new()
        .title("关于 Rairstream")
        .prompt("软件基本信息")
        .default_text(build_about_message())
        .mode(InputMode::Multiline)
        .width(360)
        .height(180)
        .ok_label("关闭")
        .cancel_label("取消")
        .show()
        .map(|_| ())
        .map_err(|error| TrayUiError::Initialize {
            message: error.to_string(),
        })
}

fn build_about_message() -> String {
    format_about_message(
        env!("CARGO_PKG_VERSION"),
        option_env!("RAIRSTREAM_GIT_COMMIT").unwrap_or("unknown"),
    )
}

fn format_about_message(version: &str, commit_hash: &str) -> String {
    format!("Rairstream\n版本：{version}\n提交：{commit_hash}")
}

fn build_icon() -> Result<Icon, TrayUiError> {
    const ICON_WIDTH: u32 = 16;
    const ICON_HEIGHT: u32 = 16;
    let mut rgba = Vec::with_capacity((ICON_WIDTH * ICON_HEIGHT * 4) as usize);

    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            let is_border = x == 0 || y == 0 || x == ICON_WIDTH - 1 || y == ICON_HEIGHT - 1;
            let (red, green, blue, alpha) = if is_border {
                (30, 144, 255, 255)
            } else {
                (16, 78, 139, 255)
            };

            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
    }

    Icon::from_rgba(rgba, ICON_WIDTH, ICON_HEIGHT).map_err(|error| TrayUiError::CreateTrayIcon {
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::format_about_message;

    #[test]
    fn test_format_about_message_contains_version_and_commit() {
        let message = format_about_message("0.1.0", "abc1234");

        assert_eq!(message, "Rairstream\n版本：0.1.0\n提交：abc1234");
    }

    #[test]
    fn test_format_about_message_supports_unknown_commit() {
        let message = format_about_message("0.1.0", "unknown");

        assert_eq!(message, "Rairstream\n版本：0.1.0\n提交：unknown");
    }
}
