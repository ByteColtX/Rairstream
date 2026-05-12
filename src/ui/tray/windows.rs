use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use native_dialog::{DialogBuilder, MessageLevel};
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use crate::app::AppFacade;
use crate::config::TrayLanguagePreference;
use crate::discovery::MdnsDiscoveryService;
use crate::platform;

use super::i18n::{TrayI18n, TrayText};
use super::{TrayCommand, TrayEvent, TrayPhase, TraySnapshot, TrayWorker, tray_receiver_label};

const APP_TITLE: &str = "Rairstream";
const POWERSHELL_CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(250);

const MENU_ID_REFRESH: &str = "refresh";
const MENU_ID_START_STREAMING: &str = "start-streaming";
const MENU_ID_STOP_STREAMING: &str = "stop-streaming";
const MENU_ID_START_AT_LOGIN: &str = "start-at-login";
const MENU_ID_LANGUAGE_SYSTEM: &str = "language:system";
const MENU_ID_LANGUAGE_EN_US: &str = "language:en-us";
const MENU_ID_LANGUAGE_ZH_CN: &str = "language:zh-cn";
const MENU_ID_QUIT: &str = "quit";
const MENU_ID_TARGET_PREFIX: &str = "target:";
const MENU_ID_PAIR_PREFIX: &str = "pair:";
const MENU_ID_FORGET_PREFIX: &str = "forget:";

#[derive(Debug, Clone)]
enum UserEvent {
    Menu(MenuEvent),
    Worker(TrayEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuAction {
    Refresh,
    StartStreaming,
    StopStreaming,
    ToggleStartAtLogin,
    SetLanguage(TrayLanguagePreference),
    Quit,
    ToggleReceiver { receiver_id: String },
    RequestPairing { receiver_id: String },
    ForgetPairing { receiver_id: String },
}

pub fn run() -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|error| format!("failed to create tray event loop: {error}"))?;

    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let mut app = TrayApp::new(event_loop.create_proxy());
    let run_result = event_loop.run_app(&mut app);
    match app.fatal_error {
        Some(error) => Err(error),
        None => run_result.map_err(|error| format!("failed to run tray event loop: {error}")),
    }
}

struct TrayApp {
    proxy: EventLoopProxy<UserEvent>,
    menu: Option<TrayMenu>,
    tray_icon: Option<TrayIcon>,
    worker_sender: Option<Sender<TrayCommand>>,
    initialized: bool,
    fatal_error: Option<String>,
}

impl TrayApp {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy,
            menu: None,
            tray_icon: None,
            worker_sender: None,
            initialized: false,
            fatal_error: None,
        }
    }

    fn initialize(&mut self) -> Result<(), String> {
        let menu = TrayMenu::build()?;
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip(APP_TITLE)
            .with_menu(Box::new(menu.root.clone()))
            .with_icon(load_tray_icon()?)
            .build()
            .map_err(|error| format!("failed to create tray icon: {error}"))?;
        let worker_sender = spawn_worker(self.proxy.clone())?;

        self.menu = Some(menu);
        self.tray_icon = Some(tray_icon);
        self.worker_sender = Some(worker_sender);
        self.initialized = true;
        Ok(())
    }

    fn handle_menu_event(&mut self, event_loop: &ActiveEventLoop, event: &MenuEvent) {
        let Some(menu) = self.menu.as_ref() else {
            return;
        };

        let Some(action) = parse_menu_action(event.id()) else {
            return;
        };

        let command = match action {
            MenuAction::Refresh => TrayCommand::RefreshDevices,
            MenuAction::StartStreaming => TrayCommand::StartStreaming,
            MenuAction::StopStreaming => TrayCommand::StopStreaming,
            MenuAction::ToggleStartAtLogin => {
                if let Some(menu) = self.menu.as_ref() {
                    let enabled = menu.start_at_login.is_checked();
                    if let Err(error) = platform::set_start_at_login_enabled(enabled) {
                        menu.start_at_login.set_checked(!enabled);
                        show_alert(MessageLevel::Error, &error);
                    }
                }
                return;
            }
            MenuAction::SetLanguage(language) => TrayCommand::SetLanguage { language },
            MenuAction::Quit => TrayCommand::Quit,
            MenuAction::ToggleReceiver { receiver_id } => {
                let Some(selected) = menu.checked_state(&receiver_id) else {
                    return;
                };
                TrayCommand::SetReceiverSelected {
                    receiver_id,
                    selected,
                }
            }
            MenuAction::RequestPairing { receiver_id } => {
                TrayCommand::RequestPairingPin { receiver_id }
            }
            MenuAction::ForgetPairing { receiver_id } => TrayCommand::ForgetPairing { receiver_id },
        };

        if let Err(error) = self.send_command(command) {
            self.fail_and_exit(event_loop, error);
        }
    }

    fn handle_worker_event(&mut self, event_loop: &ActiveEventLoop, event: TrayEvent) {
        match event {
            TrayEvent::SnapshotUpdated(snapshot) => {
                if let Some(menu) = self.menu.as_mut() {
                    if let Err(error) = menu.apply_snapshot(&snapshot) {
                        self.fail_and_exit(event_loop, error);
                    }
                }
            }
            TrayEvent::PromptForPin {
                receiver_id,
                display_name,
            } => {
                let i18n = self.menu.as_ref().map_or_else(
                    || TrayI18n::new(TrayLanguagePreference::System),
                    TrayMenu::i18n,
                );
                match prompt_for_pin(&display_name, i18n) {
                    Ok(Some(pin)) => {
                        if let Err(error) =
                            self.send_command(TrayCommand::SubmitPairingPin { receiver_id, pin })
                        {
                            self.fail_and_exit(event_loop, error);
                        }
                    }
                    Ok(None) => {
                        if let Err(error) = self.send_command(TrayCommand::CancelPairingPrompt) {
                            self.fail_and_exit(event_loop, error);
                        }
                    }
                    Err(error) => {
                        show_alert(MessageLevel::Error, &error);
                        if let Err(send_error) = self.send_command(TrayCommand::CancelPairingPrompt)
                        {
                            self.fail_and_exit(event_loop, send_error);
                        }
                    }
                }
            }
            TrayEvent::Info(message) => show_alert(MessageLevel::Info, &message),
            TrayEvent::Error(message) => show_alert(MessageLevel::Error, &message),
            TrayEvent::ExitRequested => event_loop.exit(),
        }
    }

    fn send_command(&self, command: TrayCommand) -> Result<(), String> {
        self.worker_sender
            .as_ref()
            .ok_or_else(|| String::from("tray worker is not available"))?
            .send(command)
            .map_err(|_| String::from("tray worker has stopped"))
    }

    fn fail_and_exit(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.initialized {
            return;
        }

        if let Err(error) = self.initialize() {
            self.fail_and_exit(event_loop, error);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Menu(event) => self.handle_menu_event(event_loop, &event),
            UserEvent::Worker(event) => self.handle_worker_event(event_loop, event),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

struct TrayMenu {
    root: Menu,
    status: MenuItem,
    refresh: MenuItem,
    playback_targets: Submenu,
    pair_device: Submenu,
    forget_pairing: Submenu,
    start_streaming: MenuItem,
    stop_streaming: MenuItem,
    start_at_login: CheckMenuItem,
    language: Submenu,
    language_system: CheckMenuItem,
    language_en_us: CheckMenuItem,
    language_zh_cn: CheckMenuItem,
    quit: MenuItem,
    target_items: HashMap<String, CheckMenuItem>,
    language_preference: TrayLanguagePreference,
}

struct TrayLanguageMenu {
    submenu: Submenu,
    system: CheckMenuItem,
    en_us: CheckMenuItem,
    zh_cn: CheckMenuItem,
}

impl TrayMenu {
    fn build() -> Result<Self, String> {
        let i18n = TrayI18n::new(TrayLanguagePreference::System);
        let root = Menu::new();
        let status = MenuItem::new(i18n.text(TrayText::StatusStarting), false, None);
        let refresh = MenuItem::with_id(
            MENU_ID_REFRESH,
            i18n.text(TrayText::RefreshDevices),
            true,
            None,
        );
        let playback_targets = Submenu::new(i18n.text(TrayText::PlaybackTargets), true);
        let pair_device = Submenu::new(i18n.text(TrayText::PairDevice), true);
        let forget_pairing = Submenu::new(i18n.text(TrayText::ForgetPairing), true);
        let start_streaming = MenuItem::with_id(
            MENU_ID_START_STREAMING,
            i18n.text(TrayText::StartStreaming),
            false,
            None,
        );
        let stop_streaming = MenuItem::with_id(
            MENU_ID_STOP_STREAMING,
            i18n.text(TrayText::StopStreaming),
            false,
            None,
        );
        let start_at_login = CheckMenuItem::with_id(
            MENU_ID_START_AT_LOGIN,
            i18n.text(TrayText::StartAtLogin),
            true,
            platform::is_start_at_login_enabled().unwrap_or(false),
            None,
        );
        let language_menu = build_language_menu(i18n)?;
        let quit = MenuItem::with_id(MENU_ID_QUIT, i18n.text(TrayText::Quit), true, None);

        let separator_a = PredefinedMenuItem::separator();
        let separator_b = PredefinedMenuItem::separator();
        let separator_c = PredefinedMenuItem::separator();
        let separator_d = PredefinedMenuItem::separator();
        root.append_items(&[
            &status,
            &separator_a,
            &refresh,
            &playback_targets,
            &pair_device,
            &forget_pairing,
            &separator_b,
            &start_streaming,
            &stop_streaming,
            &separator_c,
            &start_at_login,
            &language_menu.submenu,
            &separator_d,
            &quit,
        ])
        .map_err(|error| format!("failed to build tray menu: {error}"))?;

        let mut menu = Self {
            root,
            status,
            refresh,
            playback_targets,
            pair_device,
            forget_pairing,
            start_streaming,
            stop_streaming,
            start_at_login,
            language: language_menu.submenu,
            language_system: language_menu.system,
            language_en_us: language_menu.en_us,
            language_zh_cn: language_menu.zh_cn,
            quit,
            target_items: HashMap::new(),
            language_preference: TrayLanguagePreference::System,
        };
        menu.apply_snapshot(&TraySnapshot {
            phase: TrayPhase::Idle,
            receivers: Vec::new(),
            language: TrayLanguagePreference::System,
        })?;
        Ok(menu)
    }

    fn apply_snapshot(&mut self, snapshot: &TraySnapshot) -> Result<(), String> {
        self.language_preference = snapshot.language;
        self.apply_static_text();
        self.status.set_text(format_status(snapshot));
        self.sync_language_checks();
        self.rebuild_targets(snapshot)?;
        self.rebuild_pair_devices(snapshot)?;
        self.rebuild_forget_pairing(snapshot)?;

        let is_streaming = matches!(snapshot.phase, TrayPhase::Streaming { .. });
        let is_waiting_for_pin = matches!(snapshot.phase, TrayPhase::WaitingForPin { .. });
        let has_selected_targets = snapshot.receivers.iter().any(|entry| entry.is_selected);
        self.refresh.set_enabled(!is_waiting_for_pin);
        self.start_streaming
            .set_enabled(!is_streaming && !is_waiting_for_pin && has_selected_targets);
        self.stop_streaming.set_enabled(is_streaming);
        self.start_at_login.set_enabled(true);
        self.quit.set_enabled(true);
        Ok(())
    }

    fn i18n(&self) -> TrayI18n {
        TrayI18n::new(self.language_preference)
    }

    fn apply_static_text(&self) {
        let i18n = self.i18n();
        self.refresh.set_text(i18n.text(TrayText::RefreshDevices));
        self.playback_targets
            .set_text(i18n.text(TrayText::PlaybackTargets));
        self.pair_device.set_text(i18n.text(TrayText::PairDevice));
        self.forget_pairing
            .set_text(i18n.text(TrayText::ForgetPairing));
        self.start_streaming
            .set_text(i18n.text(TrayText::StartStreaming));
        self.stop_streaming
            .set_text(i18n.text(TrayText::StopStreaming));
        self.start_at_login
            .set_text(i18n.text(TrayText::StartAtLogin));
        self.language.set_text(i18n.text(TrayText::Language));
        self.language_system
            .set_text(i18n.text(TrayText::LanguageSystem));
        self.language_en_us
            .set_text(i18n.text(TrayText::LanguageEnglish));
        self.language_zh_cn
            .set_text(i18n.text(TrayText::LanguageChinese));
        self.quit.set_text(i18n.text(TrayText::Quit));
    }

    fn sync_language_checks(&self) {
        self.language_system
            .set_checked(self.language_preference == TrayLanguagePreference::System);
        self.language_en_us
            .set_checked(self.language_preference == TrayLanguagePreference::EnUs);
        self.language_zh_cn
            .set_checked(self.language_preference == TrayLanguagePreference::ZhCn);
    }

    fn checked_state(&self, receiver_id: &str) -> Option<bool> {
        self.target_items
            .get(receiver_id)
            .map(CheckMenuItem::is_checked)
    }

    fn rebuild_targets(&mut self, snapshot: &TraySnapshot) -> Result<(), String> {
        clear_submenu(&self.playback_targets);
        self.target_items.clear();

        if snapshot.receivers.is_empty() {
            append_placeholder(
                &self.playback_targets,
                self.i18n().text(TrayText::NoDevicesAvailable),
            )?;
            return Ok(());
        }

        for entry in &snapshot.receivers {
            let label = build_target_label(entry);
            let item = CheckMenuItem::with_id(
                format!("{MENU_ID_TARGET_PREFIX}{}", entry.receiver_id),
                label,
                true,
                entry.is_selected,
                None,
            );
            self.playback_targets
                .append(&item)
                .map_err(|error| format!("failed to update playback targets: {error}"))?;
            self.target_items.insert(entry.receiver_id.clone(), item);
        }

        Ok(())
    }

    fn rebuild_pair_devices(&mut self, snapshot: &TraySnapshot) -> Result<(), String> {
        clear_submenu(&self.pair_device);

        if snapshot.receivers.is_empty() {
            append_placeholder(
                &self.pair_device,
                self.i18n().text(TrayText::NoDevicesAvailable),
            )?;
            return Ok(());
        }

        for entry in &snapshot.receivers {
            let label = if entry.is_paired {
                format!(
                    "{} ({})",
                    tray_receiver_label(entry),
                    self.i18n().text(TrayText::PairedSuffix)
                )
            } else {
                tray_receiver_label(entry)
            };
            let item = MenuItem::with_id(
                format!("{MENU_ID_PAIR_PREFIX}{}", entry.receiver_id),
                label,
                true,
                None,
            );
            self.pair_device
                .append(&item)
                .map_err(|error| format!("failed to update pair menu: {error}"))?;
        }

        Ok(())
    }

    fn rebuild_forget_pairing(&mut self, snapshot: &TraySnapshot) -> Result<(), String> {
        clear_submenu(&self.forget_pairing);

        let paired_entries: Vec<_> = snapshot
            .receivers
            .iter()
            .filter(|entry| entry.is_paired)
            .collect();
        if paired_entries.is_empty() {
            append_placeholder(
                &self.forget_pairing,
                self.i18n().text(TrayText::NoSavedPairings),
            )?;
            return Ok(());
        }

        for entry in paired_entries {
            let item = MenuItem::with_id(
                format!("{MENU_ID_FORGET_PREFIX}{}", entry.receiver_id),
                tray_receiver_label(entry),
                true,
                None,
            );
            self.forget_pairing
                .append(&item)
                .map_err(|error| format!("failed to update forget menu: {error}"))?;
        }

        Ok(())
    }
}

fn spawn_worker(proxy: EventLoopProxy<UserEvent>) -> Result<Sender<TrayCommand>, String> {
    let (command_sender, command_receiver) = mpsc::channel();
    thread::Builder::new()
        .name(String::from("rairstream-tray-worker"))
        .spawn(move || {
            let facade = match AppFacade::new(MdnsDiscoveryService::default()) {
                Ok(facade) => facade,
                Err(error) => {
                    let _ =
                        proxy.send_event(UserEvent::Worker(TrayEvent::Error(error.to_string())));
                    let _ = proxy.send_event(UserEvent::Worker(TrayEvent::ExitRequested));
                    return;
                }
            };
            let mut worker = TrayWorker::new(facade);
            let _ = proxy.send_event(UserEvent::Worker(TrayEvent::SnapshotUpdated(
                worker.snapshot(),
            )));

            loop {
                match command_receiver.recv_timeout(PLAYBACK_POLL_INTERVAL) {
                    Ok(command) => {
                        let events = worker.handle_command(command);
                        let should_exit = events
                            .iter()
                            .any(|event| matches!(event, TrayEvent::ExitRequested));
                        for event in events {
                            let _ = proxy.send_event(UserEvent::Worker(event));
                        }

                        if should_exit {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        let events = worker.poll();
                        let should_exit = events
                            .iter()
                            .any(|event| matches!(event, TrayEvent::ExitRequested));
                        for event in events {
                            let _ = proxy.send_event(UserEvent::Worker(event));
                        }

                        if should_exit {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(|error| format!("failed to spawn tray worker thread: {error}"))?;

    Ok(command_sender)
}

fn load_tray_icon() -> Result<Icon, String> {
    Icon::from_resource(1, Some((32, 32)))
        .or_else(|_| Icon::from_resource(1, None))
        .map_err(|error| format!("failed to load embedded tray icon: {error}"))
}

fn clear_submenu(submenu: &Submenu) {
    while submenu.remove_at(0).is_some() {}
}

fn append_placeholder(submenu: &Submenu, text: &str) -> Result<(), String> {
    let item = MenuItem::new(text, false, None);
    submenu
        .append(&item)
        .map_err(|error| format!("failed to update placeholder menu item: {error}"))
}

fn build_language_menu(i18n: TrayI18n) -> Result<TrayLanguageMenu, String> {
    let submenu = Submenu::new(i18n.text(TrayText::Language), true);
    let system = CheckMenuItem::with_id(
        MENU_ID_LANGUAGE_SYSTEM,
        i18n.text(TrayText::LanguageSystem),
        true,
        true,
        None,
    );
    let en_us = CheckMenuItem::with_id(
        MENU_ID_LANGUAGE_EN_US,
        i18n.text(TrayText::LanguageEnglish),
        true,
        false,
        None,
    );
    let zh_cn = CheckMenuItem::with_id(
        MENU_ID_LANGUAGE_ZH_CN,
        i18n.text(TrayText::LanguageChinese),
        true,
        false,
        None,
    );
    submenu
        .append_items(&[&system, &en_us, &zh_cn])
        .map_err(|error| format!("failed to build language menu: {error}"))?;

    Ok(TrayLanguageMenu {
        submenu,
        system,
        en_us,
        zh_cn,
    })
}

fn build_target_label(entry: &crate::app::TrayReceiverEntry) -> String {
    if entry.host.is_empty() {
        tray_receiver_label(entry)
    } else {
        format!("{} ({})", tray_receiver_label(entry), entry.host)
    }
}

fn format_status(snapshot: &TraySnapshot) -> String {
    let i18n = TrayI18n::new(snapshot.language);
    match &snapshot.phase {
        TrayPhase::Idle => i18n.text(TrayText::StatusIdle).to_string(),
        TrayPhase::WaitingForPin { receiver_id } => {
            let label = snapshot
                .receivers
                .iter()
                .find(|entry| &entry.receiver_id == receiver_id)
                .map_or_else(|| receiver_id.clone(), tray_receiver_label);
            i18n.status_waiting_for_pin(&label)
        }
        TrayPhase::Streaming { receiver_ids } => {
            i18n.status_streaming_to_devices(receiver_ids.len())
        }
    }
}

fn show_alert(level: MessageLevel, message: &str) {
    let _ = DialogBuilder::message()
        .set_level(level)
        .set_title(APP_TITLE)
        .set_text(message)
        .alert()
        .show();
}

fn prompt_for_pin(display_name: &str, i18n: TrayI18n) -> Result<Option<String>, String> {
    let prompt = powershell_single_quoted(&i18n.enter_pin_shown_on(display_name));
    let title = powershell_single_quoted(APP_TITLE);
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Add-Type -AssemblyName Microsoft.VisualBasic; $pin = [Microsoft.VisualBasic.Interaction]::InputBox('{prompt}', '{title}', ''); [Console]::Out.Write($pin)"
    );

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-STA",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .creation_flags(POWERSHELL_CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to show PIN prompt: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "failed to show PIN prompt: {}",
            stderr.trim().trim_matches('\0')
        ));
    }

    let pin = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pin.is_empty() {
        Ok(None)
    } else {
        Ok(Some(pin))
    }
}

fn powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn parse_menu_action(menu_id: &MenuId) -> Option<MenuAction> {
    let menu_id = menu_id.as_ref();
    if menu_id == MENU_ID_REFRESH {
        return Some(MenuAction::Refresh);
    }
    if menu_id == MENU_ID_START_STREAMING {
        return Some(MenuAction::StartStreaming);
    }
    if menu_id == MENU_ID_STOP_STREAMING {
        return Some(MenuAction::StopStreaming);
    }
    if menu_id == MENU_ID_START_AT_LOGIN {
        return Some(MenuAction::ToggleStartAtLogin);
    }
    if menu_id == MENU_ID_LANGUAGE_SYSTEM {
        return Some(MenuAction::SetLanguage(TrayLanguagePreference::System));
    }
    if menu_id == MENU_ID_LANGUAGE_EN_US {
        return Some(MenuAction::SetLanguage(TrayLanguagePreference::EnUs));
    }
    if menu_id == MENU_ID_LANGUAGE_ZH_CN {
        return Some(MenuAction::SetLanguage(TrayLanguagePreference::ZhCn));
    }
    if menu_id == MENU_ID_QUIT {
        return Some(MenuAction::Quit);
    }
    if let Some(receiver_id) = menu_id.strip_prefix(MENU_ID_TARGET_PREFIX) {
        return Some(MenuAction::ToggleReceiver {
            receiver_id: receiver_id.to_string(),
        });
    }
    if let Some(receiver_id) = menu_id.strip_prefix(MENU_ID_PAIR_PREFIX) {
        return Some(MenuAction::RequestPairing {
            receiver_id: receiver_id.to_string(),
        });
    }
    if let Some(receiver_id) = menu_id.strip_prefix(MENU_ID_FORGET_PREFIX) {
        return Some(MenuAction::ForgetPairing {
            receiver_id: receiver_id.to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use tray_icon::menu::MenuId;

    use crate::config::TrayLanguagePreference;

    use super::{MenuAction, parse_menu_action};

    #[test]
    fn parse_menu_action_maps_static_items() {
        assert_eq!(
            parse_menu_action(&MenuId::new("refresh")),
            Some(MenuAction::Refresh)
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("start-streaming")),
            Some(MenuAction::StartStreaming)
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("quit")),
            Some(MenuAction::Quit)
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("start-at-login")),
            Some(MenuAction::ToggleStartAtLogin)
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("language:zh-cn")),
            Some(MenuAction::SetLanguage(TrayLanguagePreference::ZhCn))
        );
    }

    #[test]
    fn parse_menu_action_maps_dynamic_receiver_items() {
        assert_eq!(
            parse_menu_action(&MenuId::new("target:living-room")),
            Some(MenuAction::ToggleReceiver {
                receiver_id: String::from("living-room"),
            })
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("pair:kitchen")),
            Some(MenuAction::RequestPairing {
                receiver_id: String::from("kitchen"),
            })
        );
        assert_eq!(
            parse_menu_action(&MenuId::new("forget:office")),
            Some(MenuAction::ForgetPairing {
                receiver_id: String::from("office"),
            })
        );
    }
}
