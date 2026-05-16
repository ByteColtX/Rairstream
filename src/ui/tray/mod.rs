use crate::app::{AppFacade, TrayReceiverEntry};
use crate::config::TrayLanguagePreference;
use crate::discovery::DiscoveryService;
use crate::error::RairstreamError;
use crate::session::PlaybackSession;
use std::time::{Duration, Instant};

pub mod i18n;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
mod linux;

#[cfg(target_os = "windows")]
pub use windows::run;

#[cfg(not(target_os = "windows"))]
pub use linux::run;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayPhase {
    #[default]
    Idle,
    WaitingForPin {
        receiver_id: String,
    },
    Streaming {
        receiver_ids: Vec<String>,
    },
    Reconnecting {
        receiver_ids: Vec<String>,
        attempt: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraySnapshot {
    pub phase: TrayPhase,
    pub receivers: Vec<TrayReceiverEntry>,
    pub language: TrayLanguagePreference,
    pub auto_reconnect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    RefreshDevices,
    SetReceiverSelected { receiver_id: String, selected: bool },
    RequestPairingPin { receiver_id: String },
    SubmitPairingPin { receiver_id: String, pin: String },
    CancelPairingPrompt,
    ForgetPairing { receiver_id: String },
    StartStreaming,
    StopStreaming,
    SetAutoReconnect { enabled: bool },
    SetLanguage { language: TrayLanguagePreference },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    SnapshotUpdated(TraySnapshot),
    PromptForPin {
        receiver_id: String,
        display_name: String,
    },
    Info(String),
    Error(String),
    ExitRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAction {
    ExitNow,
    StopStreamingFirst,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrayRuntimeState {
    phase: TrayPhase,
    exit_after_stop: bool,
}

impl TrayRuntimeState {
    #[must_use]
    pub fn phase(&self) -> &TrayPhase {
        &self.phase
    }

    pub fn begin_waiting_for_pin(&mut self, receiver_id: impl Into<String>) {
        self.phase = TrayPhase::WaitingForPin {
            receiver_id: receiver_id.into(),
        };
    }

    pub fn cancel_waiting_for_pin(&mut self) {
        if matches!(self.phase, TrayPhase::WaitingForPin { .. }) {
            self.phase = TrayPhase::Idle;
        }
    }

    pub fn start_streaming(&mut self, receiver_ids: Vec<String>) {
        self.exit_after_stop = false;
        self.phase = TrayPhase::Streaming { receiver_ids };
    }

    pub fn begin_reconnecting(&mut self, receiver_ids: Vec<String>, attempt: u32) {
        self.exit_after_stop = false;
        self.phase = TrayPhase::Reconnecting {
            receiver_ids,
            attempt,
        };
    }

    #[must_use]
    pub fn stop_streaming(&mut self) -> bool {
        self.phase = TrayPhase::Idle;
        let should_exit = self.exit_after_stop;
        self.exit_after_stop = false;
        should_exit
    }

    #[must_use]
    pub fn request_quit(&mut self) -> QuitAction {
        if matches!(
            self.phase,
            TrayPhase::Streaming { .. } | TrayPhase::Reconnecting { .. }
        ) {
            self.exit_after_stop = true;
            QuitAction::StopStreamingFirst
        } else {
            QuitAction::ExitNow
        }
    }
}

pub struct TrayWorker<D> {
    facade: AppFacade<D>,
    runtime: TrayRuntimeState,
    active_session: Option<PlaybackSession>,
    reconnect_due: Option<Instant>,
}

impl<D> TrayWorker<D>
where
    D: DiscoveryService,
{
    pub fn new(facade: AppFacade<D>) -> Self {
        Self {
            facade,
            runtime: TrayRuntimeState::default(),
            active_session: None,
            reconnect_due: None,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> TraySnapshot {
        TraySnapshot {
            phase: self.runtime.phase().clone(),
            receivers: self.facade.tray_receivers(),
            language: self.facade.config().tray_language,
            auto_reconnect: self.facade.config().auto_reconnect,
        }
    }

    pub fn handle_command(&mut self, command: TrayCommand) -> Vec<TrayEvent> {
        match command {
            TrayCommand::RefreshDevices => self.refresh_devices(),
            TrayCommand::SetReceiverSelected {
                receiver_id,
                selected,
            } => self.set_receiver_selected(&receiver_id, selected),
            TrayCommand::RequestPairingPin { receiver_id } => {
                self.request_pairing_pin_display(&receiver_id)
            }
            TrayCommand::SubmitPairingPin { receiver_id, pin } => {
                self.submit_pairing_pin(&receiver_id, &pin)
            }
            TrayCommand::CancelPairingPrompt => {
                self.runtime.cancel_waiting_for_pin();
                vec![self.snapshot_event()]
            }
            TrayCommand::ForgetPairing { receiver_id } => self.forget_pairing(&receiver_id),
            TrayCommand::StartStreaming => self.start_streaming(),
            TrayCommand::StopStreaming => self.stop_streaming(),
            TrayCommand::SetAutoReconnect { enabled } => self.set_auto_reconnect(enabled),
            TrayCommand::SetLanguage { language } => self.set_language(language),
            TrayCommand::Quit => self.quit(),
        }
    }

    pub fn poll(&mut self) -> Vec<TrayEvent> {
        if let Some(error) = self
            .active_session
            .as_ref()
            .and_then(PlaybackSession::transport_error)
        {
            return self.handle_transport_error(error);
        }

        self.poll_reconnect(Instant::now())
    }

    fn refresh_devices(&mut self) -> Vec<TrayEvent> {
        let mut events = Vec::new();
        if let Err(error) = self.facade.discover() {
            events.push(TrayEvent::Error(error.to_string()));
        }
        events.push(self.snapshot_event());
        events
    }

    fn set_receiver_selected(&mut self, receiver_id: &str, selected: bool) -> Vec<TrayEvent> {
        let mut selected_ids: Vec<String> = self
            .facade
            .tray_receivers()
            .into_iter()
            .filter(|entry| entry.is_selected)
            .map(|entry| entry.receiver_id)
            .collect();

        if selected {
            if !selected_ids.iter().any(|id| id == receiver_id) {
                selected_ids.push(receiver_id.to_string());
            }
        } else {
            selected_ids.retain(|id| id != receiver_id);
        }

        let result = self.facade.set_tray_selected_receiver_ids(selected_ids);
        self.finish_config_write(result)
    }

    fn request_pairing_pin_display(&mut self, receiver_id: &str) -> Vec<TrayEvent> {
        match self.facade.request_pairing_pin_display(receiver_id) {
            Ok(receiver) => {
                self.runtime.begin_waiting_for_pin(receiver.id.clone());
                vec![
                    self.snapshot_event(),
                    TrayEvent::PromptForPin {
                        receiver_id: receiver.id,
                        display_name: receiver.name,
                    },
                ]
            }
            Err(error) => vec![TrayEvent::Error(error.to_string()), self.snapshot_event()],
        }
    }

    fn submit_pairing_pin(&mut self, receiver_id: &str, pin: &str) -> Vec<TrayEvent> {
        self.runtime.cancel_waiting_for_pin();
        match self.facade.pair(receiver_id, pin) {
            Ok(entry) => vec![
                TrayEvent::Info(
                    self.i18n()
                        .saved_pairing_for(&entry.display_name.unwrap_or(entry.receiver_id)),
                ),
                self.snapshot_event(),
            ],
            Err(error) => vec![TrayEvent::Error(error.to_string()), self.snapshot_event()],
        }
    }

    fn forget_pairing(&mut self, receiver_id: &str) -> Vec<TrayEvent> {
        match self.facade.paired_forget(receiver_id) {
            Ok(entry) => vec![
                TrayEvent::Info(
                    self.i18n()
                        .removed_pairing_for(&entry.display_name.unwrap_or(entry.receiver_id)),
                ),
                self.snapshot_event(),
            ],
            Err(error) => vec![TrayEvent::Error(error.to_string()), self.snapshot_event()],
        }
    }

    fn start_streaming(&mut self) -> Vec<TrayEvent> {
        let selected_ids = self.selected_receiver_ids();
        if selected_ids.is_empty() {
            return vec![
                TrayEvent::Error(
                    self.i18n()
                        .text(i18n::TrayText::SelectPlaybackTarget)
                        .to_string(),
                ),
                self.snapshot_event(),
            ];
        }

        match self.facade.play_capture(&selected_ids) {
            Ok(session) => {
                self.active_session = Some(session);
                self.reconnect_due = None;
                self.runtime.start_streaming(selected_ids);
                vec![self.snapshot_event()]
            }
            Err(error) => vec![TrayEvent::Error(error.to_string()), self.snapshot_event()],
        }
    }

    fn stop_streaming(&mut self) -> Vec<TrayEvent> {
        self.stop_active_session(false)
    }

    fn quit(&mut self) -> Vec<TrayEvent> {
        match self.runtime.request_quit() {
            QuitAction::ExitNow => vec![TrayEvent::ExitRequested],
            QuitAction::StopStreamingFirst => self.stop_active_session(true),
        }
    }

    fn stop_active_session(&mut self, exit_after_stop: bool) -> Vec<TrayEvent> {
        let mut events = Vec::new();
        self.reconnect_due = None;
        if let Some(session) = self.active_session.take() {
            if let Err(error) = self.facade.stop_capture(session) {
                events.push(TrayEvent::Error(error.to_string()));
            }
        }

        let should_exit = self.runtime.stop_streaming();
        events.push(self.snapshot_event());
        if exit_after_stop || should_exit {
            events.push(TrayEvent::ExitRequested);
        }
        events
    }

    fn handle_transport_error(&mut self, error: RairstreamError) -> Vec<TrayEvent> {
        let receiver_ids = match self.runtime.phase() {
            TrayPhase::Streaming { receiver_ids } => receiver_ids.clone(),
            _ => Vec::new(),
        };

        let error = if let Some(session) = self.active_session.take() {
            match self.facade.stop_capture(session) {
                Ok(()) => error,
                Err(stop_error) => RairstreamError::Playback {
                    message: format!("{error}; cleanup failed: {stop_error}"),
                },
            }
        } else {
            error
        };

        if self.facade.config().auto_reconnect && !receiver_ids.is_empty() {
            self.runtime.begin_reconnecting(receiver_ids, 1);
            self.reconnect_due = Some(Instant::now());
            return vec![TrayEvent::Error(error.to_string()), self.snapshot_event()];
        }

        let should_exit = self.runtime.stop_streaming();
        let mut events = vec![TrayEvent::Error(error.to_string()), self.snapshot_event()];
        if should_exit {
            events.push(TrayEvent::ExitRequested);
        }
        events
    }

    fn poll_reconnect(&mut self, now: Instant) -> Vec<TrayEvent> {
        let Some(due) = self.reconnect_due else {
            return Vec::new();
        };
        if now < due {
            return Vec::new();
        }

        let TrayPhase::Reconnecting {
            receiver_ids,
            attempt,
        } = self.runtime.phase()
        else {
            self.reconnect_due = None;
            return Vec::new();
        };
        let receiver_ids = receiver_ids.clone();
        let attempt = *attempt;

        let _ = self.facade.discover();
        if let Ok(session) = self.facade.play_capture(&receiver_ids) {
            self.active_session = Some(session);
            self.reconnect_due = None;
            self.runtime.start_streaming(receiver_ids);
            vec![self.snapshot_event()]
        } else {
            let next_attempt = attempt.saturating_add(1);
            self.runtime.begin_reconnecting(receiver_ids, next_attempt);
            self.reconnect_due = Some(now + reconnect_delay(next_attempt));
            vec![self.snapshot_event()]
        }
    }

    fn finish_config_write(&mut self, result: Result<(), RairstreamError>) -> Vec<TrayEvent> {
        match result {
            Ok(()) => vec![self.snapshot_event()],
            Err(error) => vec![TrayEvent::Error(error.to_string()), self.snapshot_event()],
        }
    }

    fn selected_receiver_ids(&self) -> Vec<String> {
        self.facade
            .tray_receivers()
            .into_iter()
            .filter(|entry| entry.is_selected)
            .map(|entry| entry.receiver_id)
            .collect()
    }

    fn snapshot_event(&self) -> TrayEvent {
        TrayEvent::SnapshotUpdated(self.snapshot())
    }

    fn set_language(&mut self, language: TrayLanguagePreference) -> Vec<TrayEvent> {
        let result = self.facade.set_tray_language(language);
        self.finish_config_write(result)
    }

    fn set_auto_reconnect(&mut self, enabled: bool) -> Vec<TrayEvent> {
        let result = self.facade.set_auto_reconnect(enabled);
        self.finish_config_write(result)
    }

    fn i18n(&self) -> i18n::TrayI18n {
        i18n::TrayI18n::new(self.facade.config().tray_language)
    }
}

#[must_use]
fn reconnect_delay(attempt: u32) -> Duration {
    match attempt {
        0 | 1 => Duration::ZERO,
        2 => Duration::from_secs(2),
        3 => Duration::from_secs(5),
        4 => Duration::from_secs(10),
        5 => Duration::from_secs(30),
        _ => Duration::from_secs(60),
    }
}

#[must_use]
pub fn tray_receiver_label(entry: &TrayReceiverEntry) -> String {
    entry
        .display_name
        .clone()
        .unwrap_or_else(|| entry.receiver_id.clone())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::app::AppFacade;
    use crate::config::{AppConfig, TrayLanguagePreference, save_config};
    use crate::pairing::{ReceiverAuthFlow, ReceiverCredentials};
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };

    use super::{
        QuitAction, TrayCommand, TrayEvent, TrayPhase, TrayRuntimeState, TrayWorker,
        reconnect_delay,
    };
    use crate::discovery::DiscoveryService;

    #[derive(Clone)]
    struct FixedDiscoveryService {
        receivers: Vec<Receiver>,
    }

    impl DiscoveryService for FixedDiscoveryService {
        fn discover_devices(&self) -> Vec<Receiver> {
            self.receivers.clone()
        }
    }

    fn temp_config_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("rairstream-tray-worker-{unique}.json"))
    }

    fn paired_credentials(auth_flow: ReceiverAuthFlow) -> ReceiverCredentials {
        ReceiverCredentials {
            auth_flow,
            controller_pairing_id: String::from("controller"),
            controller_ltpk_hex: String::from("aa"),
            controller_ltsk_hex: String::from("bb"),
            receiver_pairing_id: String::from("receiver"),
            receiver_ltpk_hex: String::from("cc"),
        }
    }

    fn build_worker(
        config: &AppConfig,
        receivers: Vec<Receiver>,
    ) -> (TrayWorker<FixedDiscoveryService>, PathBuf) {
        let path = temp_config_path();
        save_config(&path, config).unwrap();
        let facade =
            AppFacade::with_config_path(FixedDiscoveryService { receivers }, path.clone()).unwrap();
        (TrayWorker::new(facade), path)
    }

    fn spawn_pairing_probe_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let _ = read_rtsp_message(&mut reader);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let _ = read_rtsp_message(&mut reader);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });

        port
    }

    fn read_rtsp_message(reader: &mut BufReader<std::net::TcpStream>) -> String {
        let mut request = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        request
    }

    #[test]
    fn runtime_state_requests_stop_before_exit_while_streaming() {
        let mut state = TrayRuntimeState::default();
        state.start_streaming(vec![String::from("living-room")]);

        assert_eq!(state.request_quit(), QuitAction::StopStreamingFirst);
        assert!(state.stop_streaming());
        assert_eq!(state.phase(), &TrayPhase::Idle);
    }

    #[test]
    fn runtime_state_requests_stop_before_exit_while_reconnecting() {
        let mut state = TrayRuntimeState::default();
        state.begin_reconnecting(vec![String::from("living-room")], 2);

        assert_eq!(state.request_quit(), QuitAction::StopStreamingFirst);
        assert!(state.stop_streaming());
        assert_eq!(state.phase(), &TrayPhase::Idle);
    }

    #[test]
    fn runtime_state_tracks_reconnecting_attempt() {
        let mut state = TrayRuntimeState::default();
        state.begin_reconnecting(vec![String::from("living-room")], 3);

        assert_eq!(
            state.phase(),
            &TrayPhase::Reconnecting {
                receiver_ids: vec![String::from("living-room")],
                attempt: 3,
            }
        );
    }

    #[test]
    fn reconnect_delay_uses_capped_backoff() {
        assert_eq!(reconnect_delay(1), Duration::ZERO);
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(3), Duration::from_secs(5));
        assert_eq!(reconnect_delay(4), Duration::from_secs(10));
        assert_eq!(reconnect_delay(5), Duration::from_secs(30));
        assert_eq!(reconnect_delay(6), Duration::from_secs(60));
        assert_eq!(reconnect_delay(99), Duration::from_secs(60));
    }

    #[test]
    fn runtime_state_clears_waiting_pin_on_cancel() {
        let mut state = TrayRuntimeState::default();
        state.begin_waiting_for_pin("office");
        state.cancel_waiting_for_pin();

        assert_eq!(state.phase(), &TrayPhase::Idle);
    }

    #[test]
    fn worker_updates_selected_receivers_and_emits_snapshot() {
        let mut config = AppConfig::default();
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("living-room"),
            name: String::from("Living Room"),
            host: String::from("192.168.1.10"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        let (mut worker, path) = build_worker(&config, Vec::new());

        let events = worker.handle_command(TrayCommand::SetReceiverSelected {
            receiver_id: String::from("living-room"),
            selected: true,
        });

        assert!(matches!(
            &events[..],
            [TrayEvent::SnapshotUpdated(snapshot)]
                if snapshot.receivers[0].is_selected
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_refresh_preserves_cached_snapshot_after_discovery_failure() {
        let mut config = AppConfig::default();
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("kitchen"),
            name: String::from("Kitchen"),
            host: String::from("192.168.1.20"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        let (mut worker, path) = build_worker(&config, Vec::new());

        let events = worker.handle_command(TrayCommand::RefreshDevices);

        assert!(matches!(
            &events[..],
            [TrayEvent::Error(_), TrayEvent::SnapshotUpdated(snapshot)]
                if snapshot.receivers.len() == 1 && snapshot.receivers[0].receiver_id == "kitchen"
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_forget_pairing_emits_info_and_snapshot() {
        let mut config = AppConfig::default();
        config.upsert_paired_receiver("office", paired_credentials(ReceiverAuthFlow::Modern));
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("office"),
            name: String::from("Office"),
            host: String::from("192.168.1.30"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        let (mut worker, path) = build_worker(&config, Vec::new());

        let events = worker.handle_command(TrayCommand::ForgetPairing {
            receiver_id: String::from("office"),
        });

        assert!(matches!(
            &events[..],
            [TrayEvent::Info(_), TrayEvent::SnapshotUpdated(snapshot)]
                if !snapshot.receivers[0].is_paired
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_requests_pin_and_enters_waiting_state() {
        let port = spawn_pairing_probe_server();
        let receiver = Receiver {
            id: String::from("living-room"),
            name: String::from("Living Room"),
            host: String::from("127.0.0.1"),
            port,
            generation: AirPlayGeneration::AirPlay2,
            transport_profile: ReceiverKind::ModernAirPlayAuth,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields();
        let config = AppConfig::default();
        let (mut worker, path) = build_worker(&config, vec![receiver]);

        let events = worker.handle_command(TrayCommand::RequestPairingPin {
            receiver_id: String::from("living-room"),
        });

        assert!(matches!(
            &events[..],
            [
                TrayEvent::SnapshotUpdated(snapshot),
                TrayEvent::PromptForPin { receiver_id, display_name }
            ] if matches!(
                snapshot.phase,
                TrayPhase::WaitingForPin { ref receiver_id } if receiver_id == "living-room"
            ) && receiver_id == "living-room" && display_name == "Living Room"
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_persists_language_selection() {
        let config = AppConfig::default();
        let (mut worker, path) = build_worker(&config, Vec::new());

        let events = worker.handle_command(TrayCommand::SetLanguage {
            language: TrayLanguagePreference::ZhCn,
        });

        assert!(matches!(
            &events[..],
            [TrayEvent::SnapshotUpdated(snapshot)]
                if snapshot.language == TrayLanguagePreference::ZhCn
        ));
        let reloaded = crate::config::load_config(&path).unwrap();
        assert_eq!(reloaded.tray_language, TrayLanguagePreference::ZhCn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_snapshot_includes_auto_reconnect_state() {
        let mut config = AppConfig::default();
        config.set_auto_reconnect(true);
        let (worker, path) = build_worker(&config, Vec::new());

        assert!(worker.snapshot().auto_reconnect);
        let _ = std::fs::remove_file(path);
    }
}
