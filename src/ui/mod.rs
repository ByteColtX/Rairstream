//! 系统托盘与桌面交互层。

mod controller;
mod state;
mod tray;

use crate::app::SessionCoordinator;
use crate::config::AppConfig;
use crate::discovery::DiscoveryService;
use thiserror::Error;

pub use controller::{TrayController, TraySessionService};
pub use state::{TrayAppState, TrayDeviceMenuItem, TrayMenuModel, build_tray_menu_model};

#[derive(Debug, Error)]
pub enum TrayUiError {
    #[error("系统托盘仅在 Windows 目标平台可用")]
    UnsupportedPlatform,
    #[error("创建托盘图标失败: {message}")]
    CreateTrayIcon { message: String },
    #[error("创建托盘菜单失败: {message}")]
    CreateMenu { message: String },
    #[error("更新托盘提示失败: {message}")]
    UpdateTooltip { message: String },
    #[error("初始化托盘界面失败: {message}")]
    Initialize { message: String },
}

pub fn run_tray_app<D>(
    coordinator: SessionCoordinator<D>,
    config: AppConfig,
) -> Result<(), TrayUiError>
where
    D: DiscoveryService + 'static,
{
    tray::run_tray_app(coordinator, config)
}

#[cfg(test)]
mod tests {
    use super::TrayUiError;

    #[test]
    fn tray_ui_error_formats_message() {
        let error = TrayUiError::CreateMenu {
            message: String::from("menu failure"),
        };

        assert_eq!(error.to_string(), "创建托盘菜单失败: menu failure");
    }
}
