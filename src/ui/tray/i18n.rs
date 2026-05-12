use crate::config::TrayLanguagePreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayLocale {
    EnUs,
    ZhCn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayText {
    StatusStarting,
    StatusIdle,
    StatusWaitingForPin,
    StatusStreamingToDevices,
    RefreshDevices,
    PlaybackTargets,
    PairDevice,
    ForgetPairing,
    StartStreaming,
    StopStreaming,
    StartAtLogin,
    Language,
    LanguageSystem,
    LanguageEnglish,
    LanguageChinese,
    Quit,
    NoDevicesAvailable,
    NoSavedPairings,
    PairedSuffix,
    SavedPairingFor,
    RemovedPairingFor,
    SelectPlaybackTarget,
    EnterPinShownOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayI18n {
    locale: TrayLocale,
}

impl TrayI18n {
    #[must_use]
    pub fn new(preference: TrayLanguagePreference) -> Self {
        Self {
            locale: resolve_locale(preference, detected_system_locale().as_deref()),
        }
    }

    #[must_use]
    pub fn with_system_locale(
        preference: TrayLanguagePreference,
        system_locale: Option<&str>,
    ) -> Self {
        Self {
            locale: resolve_locale(preference, system_locale),
        }
    }

    #[must_use]
    pub fn text(&self, key: TrayText) -> &'static str {
        text_for(self.locale, key)
    }

    #[must_use]
    pub fn status_waiting_for_pin(&self, label: &str) -> String {
        format!("{} - {label}", self.text(TrayText::StatusWaitingForPin))
    }

    #[must_use]
    pub fn status_streaming_to_devices(&self, count: usize) -> String {
        format!(
            "{} {count} device(s)",
            self.text(TrayText::StatusStreamingToDevices)
        )
    }

    #[must_use]
    pub fn saved_pairing_for(&self, label: &str) -> String {
        format!("{} {label}", self.text(TrayText::SavedPairingFor))
    }

    #[must_use]
    pub fn removed_pairing_for(&self, label: &str) -> String {
        format!("{} {label}", self.text(TrayText::RemovedPairingFor))
    }

    #[must_use]
    pub fn enter_pin_shown_on(&self, display_name: &str) -> String {
        format!("{} {display_name}:", self.text(TrayText::EnterPinShownOn))
    }
}

#[must_use]
pub fn resolve_locale(
    preference: TrayLanguagePreference,
    system_locale: Option<&str>,
) -> TrayLocale {
    match preference {
        TrayLanguagePreference::EnUs => TrayLocale::EnUs,
        TrayLanguagePreference::ZhCn => TrayLocale::ZhCn,
        TrayLanguagePreference::System => system_locale
            .and_then(locale_from_tag)
            .unwrap_or(TrayLocale::EnUs),
    }
}

fn detected_system_locale() -> Option<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}

fn locale_from_tag(tag: &str) -> Option<TrayLocale> {
    let normalized = tag
        .split('.')
        .next()
        .unwrap_or(tag)
        .replace('_', "-")
        .to_ascii_lowercase();

    if normalized == "zh-cn" || normalized.starts_with("zh-hans") {
        return Some(TrayLocale::ZhCn);
    }
    if normalized == "en-us" || normalized.starts_with("en") {
        return Some(TrayLocale::EnUs);
    }

    None
}

fn text_for(locale: TrayLocale, key: TrayText) -> &'static str {
    match (locale, key) {
        (TrayLocale::ZhCn, TrayText::StatusStarting) => "状态：正在启动...",
        (TrayLocale::ZhCn, TrayText::StatusIdle) => "状态：空闲",
        (TrayLocale::ZhCn, TrayText::StatusWaitingForPin) => "状态：等待 PIN",
        (TrayLocale::ZhCn, TrayText::StatusStreamingToDevices) => "状态：正在串流到",
        (TrayLocale::ZhCn, TrayText::RefreshDevices) => "刷新设备",
        (TrayLocale::ZhCn, TrayText::PlaybackTargets) => "播放目标",
        (TrayLocale::ZhCn, TrayText::PairDevice) => "配对设备",
        (TrayLocale::ZhCn, TrayText::ForgetPairing) => "忘记配对",
        (TrayLocale::ZhCn, TrayText::StartStreaming) => "开始串流",
        (TrayLocale::ZhCn, TrayText::StopStreaming) => "停止串流",
        (TrayLocale::ZhCn, TrayText::StartAtLogin) => "登录时启动",
        (TrayLocale::ZhCn, TrayText::Language) => "语言",
        (TrayLocale::ZhCn, TrayText::LanguageSystem) => "跟随系统",
        (TrayLocale::ZhCn, TrayText::LanguageEnglish) => "英语",
        (TrayLocale::ZhCn, TrayText::LanguageChinese) => "简体中文",
        (TrayLocale::ZhCn, TrayText::Quit) => "退出",
        (TrayLocale::ZhCn, TrayText::NoDevicesAvailable) => "没有可用设备",
        (TrayLocale::ZhCn, TrayText::NoSavedPairings) => "没有已保存的配对",
        (TrayLocale::ZhCn, TrayText::PairedSuffix) => "已配对",
        (TrayLocale::ZhCn, TrayText::SavedPairingFor) => "已保存配对：",
        (TrayLocale::ZhCn, TrayText::RemovedPairingFor) => "已移除配对：",
        (TrayLocale::ZhCn, TrayText::SelectPlaybackTarget) => "请至少选择一个播放目标",
        (TrayLocale::ZhCn, TrayText::EnterPinShownOn) => "输入此设备上显示的 PIN：",
        (_, TrayText::StatusStarting) => "Status: Starting...",
        (_, TrayText::StatusIdle) => "Status: Idle",
        (_, TrayText::StatusWaitingForPin) => "Status: Waiting for PIN",
        (_, TrayText::StatusStreamingToDevices) => "Status: Streaming to",
        (_, TrayText::RefreshDevices) => "Refresh Devices",
        (_, TrayText::PlaybackTargets) => "Playback Targets",
        (_, TrayText::PairDevice) => "Pair Device",
        (_, TrayText::ForgetPairing) => "Forget Pairing",
        (_, TrayText::StartStreaming) => "Start Streaming",
        (_, TrayText::StopStreaming) => "Stop Streaming",
        (_, TrayText::StartAtLogin) => "Start at login",
        (_, TrayText::Language) => "Language",
        (_, TrayText::LanguageSystem) => "Follow System",
        (_, TrayText::LanguageEnglish) => "English",
        (_, TrayText::LanguageChinese) => "Chinese (Simplified)",
        (_, TrayText::Quit) => "Quit",
        (_, TrayText::NoDevicesAvailable) => "No devices available",
        (_, TrayText::NoSavedPairings) => "No saved pairings",
        (_, TrayText::PairedSuffix) => "paired",
        (_, TrayText::SavedPairingFor) => "saved pairing for",
        (_, TrayText::RemovedPairingFor) => "removed pairing for",
        (_, TrayText::SelectPlaybackTarget) => "select at least one playback target",
        (_, TrayText::EnterPinShownOn) => "Enter the PIN shown on",
    }
}

#[cfg(test)]
mod tests {
    use crate::config::TrayLanguagePreference;

    use super::{TrayI18n, TrayLocale, TrayText, resolve_locale};

    #[test]
    fn resolves_system_locale_with_english_fallback() {
        assert_eq!(
            resolve_locale(TrayLanguagePreference::System, Some("zh_CN.UTF-8")),
            TrayLocale::ZhCn
        );
        assert_eq!(
            resolve_locale(TrayLanguagePreference::System, Some("fr-FR")),
            TrayLocale::EnUs
        );
        assert_eq!(
            resolve_locale(TrayLanguagePreference::EnUs, Some("zh-CN")),
            TrayLocale::EnUs
        );
    }

    #[test]
    fn maps_key_tray_texts_for_supported_languages() {
        let english = TrayI18n::with_system_locale(TrayLanguagePreference::EnUs, None);
        let chinese = TrayI18n::with_system_locale(TrayLanguagePreference::ZhCn, None);

        assert_eq!(english.text(TrayText::RefreshDevices), "Refresh Devices");
        assert_eq!(chinese.text(TrayText::RefreshDevices), "刷新设备");
        assert_eq!(
            chinese.status_waiting_for_pin("Living Room"),
            "状态：等待 PIN - Living Room"
        );
    }
}
