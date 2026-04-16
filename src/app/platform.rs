//! 平台层：封装操作系统相关能力。

use super::RairstreamError;

#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

#[cfg(windows)]
const STARTUP_VALUE_NAME: &str = "Rairstream";

/// 平台运行时摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub supports_system_audio_capture: bool,
}

/// 当前目标平台的最小能力描述。
#[must_use]
pub fn current_platform() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS,
        supports_system_audio_capture: cfg!(target_os = "windows"),
    }
}

/// 首版本为 Windows 设计，其他平台暂时只返回占位错误。
pub fn ensure_supported_runtime() -> Result<(), RairstreamError> {
    if cfg!(target_os = "windows") {
        Ok(())
    } else {
        Err(RairstreamError::NotImplemented {
            feature: "non-Windows runtime support",
        })
    }
}

pub fn is_launch_at_startup_enabled() -> Result<bool, RairstreamError> {
    #[cfg(windows)]
    {
        let run_key = open_run_key(KEY_READ)?;
        match run_key.get_value::<String, _>(STARTUP_VALUE_NAME) {
            Ok(command) => Ok(command == startup_command()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(RairstreamError::InvalidConfiguration {
                message: format!("读取开机启动状态失败: {error}"),
            }),
        }
    }

    #[cfg(not(windows))]
    {
        Err(RairstreamError::NotImplemented {
            feature: "launch at startup support",
        })
    }
}

pub fn set_launch_at_startup_enabled(enabled: bool) -> Result<(), RairstreamError> {
    #[cfg(windows)]
    {
        let run_key = open_run_key(KEY_WRITE)?;
        if enabled {
            run_key
                .set_value(STARTUP_VALUE_NAME, &startup_command()?)
                .map_err(|error| RairstreamError::InvalidConfiguration {
                    message: format!("写入开机启动配置失败: {error}"),
                })?;
        } else {
            match run_key.delete_value(STARTUP_VALUE_NAME) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RairstreamError::InvalidConfiguration {
                        message: format!("移除开机启动配置失败: {error}"),
                    });
                }
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err(RairstreamError::NotImplemented {
            feature: "launch at startup support",
        })
    }
}

#[cfg(windows)]
fn startup_command() -> Result<String, RairstreamError> {
    let exe_path =
        std::env::current_exe().map_err(|error| RairstreamError::InvalidConfiguration {
            message: format!("读取当前可执行文件路径失败: {error}"),
        })?;
    Ok(format!("\"{}\"", exe_path.display()))
}

#[cfg(windows)]
fn open_run_key(access: u32) -> Result<RegKey, RairstreamError> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    current_user
        .open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run", access)
        .map_err(|error| RairstreamError::InvalidConfiguration {
            message: format!("打开开机启动注册表失败: {error}"),
        })
}

#[cfg(test)]
mod tests {
    use super::current_platform;

    #[test]
    fn current_platform_has_name() {
        assert!(!current_platform().os.is_empty());
    }
}
