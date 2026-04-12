//! 平台层：封装操作系统相关能力。

use super::RairstreamError;

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

#[cfg(test)]
mod tests {
    use super::current_platform;

    #[test]
    fn current_platform_has_name() {
        assert!(!current_platform().os.is_empty());
    }
}
