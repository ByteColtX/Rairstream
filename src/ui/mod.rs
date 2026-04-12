//! 系统托盘与桌面交互层。

/// 首阶段先保留 UI 模块占位，后续接入托盘菜单与设备选择界面。
pub const UI_PLACEHOLDER: &str = "ui-placeholder";

#[cfg(test)]
mod tests {
    use super::UI_PLACEHOLDER;

    #[test]
    fn ui_placeholder_is_available() {
        assert_eq!(UI_PLACEHOLDER, "ui-placeholder");
    }
}
