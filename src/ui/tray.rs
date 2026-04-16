use super::TrayUiError;
use super::controller::TrayController;
use super::state::TrayMenuModel;
use crate::app::{RairstreamError, SessionCoordinator, SessionState};
use crate::config::AppConfig;
use crate::discovery::DiscoveryService;
use inputbox::InputBox;
use std::collections::HashMap;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tracing::{error, info, warn};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayAction {
    RefreshDevices,
    SelectDevice(String),
    StopStreaming,
    Quit,
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
    let (initial_menu, mut action_map) = build_menu(&initial_model, &controller)?;
    let icon = build_icon()?;
    let mut event_loop_builder = EventLoopBuilder::<MenuEvent>::with_user_event();
    let event_loop = event_loop_builder.build();
    let proxy = event_loop.create_proxy();

    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(event);
    }));

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip(initial_model.status_label.clone())
        .with_menu(Box::new(initial_menu))
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
                &mut action_map,
                &menu_event.id,
                control_flow,
            );
        }
    })
}

fn handle_menu_event(
    controller: &mut TrayController<impl super::controller::TraySessionService>,
    tray_icon: &TrayIcon,
    action_map: &mut HashMap<MenuId, TrayAction>,
    menu_id: &MenuId,
    control_flow: &mut ControlFlow,
) {
    let model = match action_map.get(menu_id).cloned() {
        Some(TrayAction::RefreshDevices) => {
            info!("托盘菜单触发刷新设备");
            controller.refresh_devices()
        }
        Some(TrayAction::SelectDevice(device_id)) => handle_select_device(controller, &device_id),
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

    match apply_menu_model(tray_icon, &model, controller) {
        Ok(updated_action_map) => *action_map = updated_action_map,
        Err(error) => {
            error!(error = %error, "应用托盘菜单状态失败，准备退出事件循环");
            *control_flow = ControlFlow::ExitWithCode(1);
        }
    }
}

fn handle_select_device(
    controller: &mut TrayController<impl super::controller::TraySessionService>,
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
    controller: &mut TrayController<impl super::controller::TraySessionService>,
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

fn build_menu(
    model: &TrayMenuModel,
    controller: &TrayController<impl super::controller::TraySessionService>,
) -> Result<(Menu, HashMap<MenuId, TrayAction>), TrayUiError> {
    let menu = Menu::new();
    let mut action_map = HashMap::new();

    let status_item = MenuItem::new(model.status_label.as_str(), false, None);
    menu.append(&status_item)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    let status_separator = PredefinedMenuItem::separator();
    menu.append(&status_separator)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    let refresh_item = MenuItem::new("刷新设备", model.refresh_enabled, None);
    action_map.insert(refresh_item.id().clone(), TrayAction::RefreshDevices);
    menu.append(&refresh_item)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    if matches!(
        controller.state().app_state.active_session,
        SessionState::Streaming { .. }
    ) {
        let stop_item = MenuItem::new("停止串流", true, None);
        action_map.insert(stop_item.id().clone(), TrayAction::StopStreaming);
        menu.append(&stop_item)
            .map_err(|error| TrayUiError::CreateMenu {
                message: error.to_string(),
            })?;
    }

    let refresh_separator = PredefinedMenuItem::separator();
    menu.append(&refresh_separator)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    match model.empty_label.as_deref() {
        Some(empty_label) => {
            let empty_item = MenuItem::new(empty_label, false, None);
            menu.append(&empty_item)
                .map_err(|error| TrayUiError::CreateMenu {
                    message: error.to_string(),
                })?;
        }
        None => {
            for device_item in &model.device_items {
                let menu_item =
                    MenuItem::new(device_item.label.as_str(), device_item.enabled, None);
                action_map.insert(
                    menu_item.id().clone(),
                    TrayAction::SelectDevice(device_item.device_id.clone()),
                );
                menu.append(&menu_item)
                    .map_err(|error| TrayUiError::CreateMenu {
                        message: error.to_string(),
                    })?;
            }
        }
    }

    let quit_separator = PredefinedMenuItem::separator();
    menu.append(&quit_separator)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    let quit_item = MenuItem::new("退出", true, None);
    action_map.insert(quit_item.id().clone(), TrayAction::Quit);
    menu.append(&quit_item)
        .map_err(|error| TrayUiError::CreateMenu {
            message: error.to_string(),
        })?;

    Ok((menu, action_map))
}

fn apply_menu_model(
    tray_icon: &TrayIcon,
    model: &TrayMenuModel,
    controller: &TrayController<impl super::controller::TraySessionService>,
) -> Result<HashMap<MenuId, TrayAction>, TrayUiError> {
    let (menu, action_map) = build_menu(model, controller)?;

    tray_icon.set_menu(Some(Box::new(menu)));
    tray_icon
        .set_tooltip(Some(model.status_label.as_str()))
        .map_err(|error| TrayUiError::UpdateTooltip {
            message: error.to_string(),
        })?;

    Ok(action_map)
}

fn prompt_pairing_pin(
    controller: &TrayController<impl super::controller::TraySessionService>,
    device_id: &str,
) -> Result<Option<String>, TrayUiError> {
    let device_name = controller
        .state()
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
