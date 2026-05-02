use std::path::{Path, PathBuf};

use crate::config::{
    AppConfig, CachedReceiver, ConfigError, default_config_path, load_config, save_config,
};
use crate::discovery::{DiscoveryService, MdnsDiscoveryService};
use crate::error::RairstreamError;
use crate::pairing::ReceiverCredentials;
use crate::receiver::{Receiver, selector};
use crate::session::{
    PlaybackSession, pair_receiver_with_pin, play_capture, play_file,
    request_pairing_pin_display as session_request_pairing_pin_display,
};
use crate::storage::{paired_devices, receiver_cache};

use super::models::{AppState, InspectResult, PairedReceiverEntry, SessionState};

pub struct AppFacade<D> {
    discovery: D,
    config_path: PathBuf,
    config: AppConfig,
    state: AppState,
}

impl AppFacade<MdnsDiscoveryService> {
    pub fn new(discovery: MdnsDiscoveryService) -> Result<Self, ConfigError> {
        Self::with_config_path(discovery, default_config_path())
    }
}

impl<D> AppFacade<D>
where
    D: DiscoveryService,
{
    pub fn with_config_path(discovery: D, config_path: PathBuf) -> Result<Self, ConfigError> {
        let config = load_config(&config_path)?;
        Ok(Self {
            discovery,
            config_path,
            config,
            state: AppState::default(),
        })
    }

    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    #[must_use]
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn discover(&mut self) -> Result<Vec<Receiver>, RairstreamError> {
        self.state.session = SessionState::Discovering;
        let mut receivers = self.discovery.discover_devices();
        sort_receivers(&mut receivers);
        self.state.last_receivers.clone_from(&receivers);
        receiver_cache::cache_receivers(&mut self.config, &receivers);
        let persist_result = self.persist_config();
        self.state.session = SessionState::Idle;
        persist_result?;
        if receivers.is_empty() {
            return Err(RairstreamError::NoReceiversDiscovered);
        }
        Ok(receivers)
    }

    pub fn inspect(&mut self, selector_text: &str) -> Result<InspectResult, RairstreamError> {
        let receivers = self.ensure_receivers()?;
        let receiver = selector::resolve_receiver(&receivers, selector_text)?;
        Ok(InspectResult {
            has_stored_credentials: paired_devices::get_paired_receiver(&self.config, &receiver.id)
                .is_some(),
            receiver,
        })
    }

    pub fn pair(
        &mut self,
        selector_text: &str,
        pin: &str,
    ) -> Result<PairedReceiverEntry, RairstreamError> {
        let receivers = self.ensure_receivers()?;
        let receiver = selector::resolve_receiver(&receivers, selector_text)?;
        self.state.session = SessionState::Pairing {
            receiver_id: receiver.id.clone(),
        };
        let result = (|| {
            let credentials = pair_receiver_with_pin(
                &receiver,
                pin,
                self.config.sender_volume_percent,
                self.config.paired_receivers.get(&receiver.id).cloned(),
            )?;
            self.store_credentials(&receiver, credentials)?;
            Ok(self.build_paired_entry(&receiver.id))
        })();
        self.state.session = SessionState::Idle;
        result
    }

    pub fn request_pairing_pin_display(
        &mut self,
        selector_text: &str,
    ) -> Result<Receiver, RairstreamError> {
        let receivers = self.ensure_receivers()?;
        let receiver = selector::resolve_receiver(&receivers, selector_text)?;
        self.state.session = SessionState::Pairing {
            receiver_id: receiver.id.clone(),
        };
        let result = session_request_pairing_pin_display(
            &receiver,
            self.config.sender_volume_percent,
            self.config.paired_receivers.get(&receiver.id).cloned(),
        )
        .map(|()| receiver);
        self.state.session = SessionState::Idle;
        result
    }

    #[must_use]
    pub fn paired_list(&self) -> Vec<PairedReceiverEntry> {
        let mut entries: Vec<_> = self
            .config
            .paired_receivers
            .keys()
            .map(|receiver_id| self.build_paired_entry(receiver_id))
            .collect();
        sort_paired_entries(&mut entries);
        entries
    }

    pub fn paired_forget(
        &mut self,
        selector_text: &str,
    ) -> Result<PairedReceiverEntry, RairstreamError> {
        let receiver_id = self.resolve_cached_receiver_id(selector_text)?;
        if !self.config.paired_receivers.contains_key(&receiver_id) {
            return Err(RairstreamError::InvalidInput {
                message: format!("receiver `{selector_text}` has no saved pairing"),
            });
        }
        let entry = self.build_paired_entry(&receiver_id);
        self.config.remove_paired_receiver(&receiver_id);
        self.persist_config()?;
        Ok(entry)
    }

    pub fn play_file(&mut self, path: &Path, selectors: &[String]) -> Result<(), RairstreamError> {
        let receivers = self.ensure_receivers()?;
        let targets = selector::resolve_receivers(&receivers, selectors)?;
        self.state.session = SessionState::Streaming {
            receiver_ids: targets.iter().map(|receiver| receiver.id.clone()).collect(),
        };
        let result = play_file(
            path,
            &targets,
            &self.config.paired_receivers,
            self.config.sender_volume_percent,
        );
        self.state.session = SessionState::Idle;
        result
    }

    pub fn play_capture(
        &mut self,
        selectors: &[String],
    ) -> Result<PlaybackSession, RairstreamError> {
        let receivers = self.ensure_receivers()?;
        let targets = selector::resolve_receivers(&receivers, selectors)?;
        self.state.session = SessionState::Streaming {
            receiver_ids: targets.iter().map(|receiver| receiver.id.clone()).collect(),
        };
        match play_capture(
            &targets,
            &self.config.paired_receivers,
            self.config.sender_volume_percent,
        ) {
            Ok(session) => Ok(session),
            Err(error) => {
                self.state.session = SessionState::Idle;
                Err(error)
            }
        }
    }

    pub fn stop_capture(&mut self, session: PlaybackSession) -> Result<(), RairstreamError> {
        let result = session.stop();
        self.state.session = SessionState::Idle;
        result
    }

    fn ensure_receivers(&mut self) -> Result<Vec<Receiver>, RairstreamError> {
        if self.state.last_receivers.is_empty() {
            return self.discover();
        }

        Ok(self.state.last_receivers.clone())
    }

    fn resolve_cached_receiver_id(
        &mut self,
        selector_text: &str,
    ) -> Result<String, RairstreamError> {
        if self.config.paired_receivers.contains_key(selector_text) {
            return Ok(selector_text.to_string());
        }

        if let Some(receiver_id) = self.resolve_receiver_id_from_cache(selector_text)? {
            return Ok(receiver_id);
        }

        let receivers = self.ensure_receivers()?;
        let receiver = selector::resolve_receiver(&receivers, selector_text)?;
        Ok(receiver.id)
    }

    fn store_credentials(
        &mut self,
        receiver: &Receiver,
        credentials: ReceiverCredentials,
    ) -> Result<(), RairstreamError> {
        self.config
            .upsert_paired_receiver(receiver.id.clone(), credentials);
        self.persist_config()?;
        Ok(())
    }

    fn build_paired_entry(&self, receiver_id: &str) -> PairedReceiverEntry {
        let display_name = self
            .config
            .receiver_cache
            .get(receiver_id)
            .map(|receiver| receiver.name.clone());
        let auth_flow = self
            .config
            .paired_receivers
            .get(receiver_id)
            .map(|credentials| credentials.auth_flow.clone())
            .unwrap_or_default();

        PairedReceiverEntry {
            receiver_id: receiver_id.to_string(),
            display_name,
            auth_flow,
        }
    }

    fn resolve_receiver_id_from_cache(
        &self,
        selector_text: &str,
    ) -> Result<Option<String>, RairstreamError> {
        let matches: Vec<&CachedReceiver> = self
            .config
            .receiver_cache
            .values()
            .filter(|receiver| cached_receiver_matches(receiver, selector_text))
            .collect();

        match matches.as_slice() {
            [] => Ok(None),
            [receiver] => Ok(Some(receiver.id.clone())),
            _ => Err(RairstreamError::AmbiguousReceiver {
                selector: selector_text.to_string(),
                matches: matches
                    .iter()
                    .map(|receiver| receiver.name.clone())
                    .collect(),
            }),
        }
    }

    fn persist_config(&self) -> Result<(), RairstreamError> {
        save_config(&self.config_path, &self.config)?;
        Ok(())
    }
}

fn sort_receivers(receivers: &mut [Receiver]) {
    receivers.sort_by_cached_key(|receiver| {
        (
            receiver.name.to_ascii_lowercase(),
            receiver.id.to_ascii_lowercase(),
        )
    });
}

fn sort_paired_entries(entries: &mut [PairedReceiverEntry]) {
    entries.sort_by_cached_key(|entry| {
        (
            entry
                .display_name
                .as_deref()
                .unwrap_or(&entry.receiver_id)
                .to_ascii_lowercase(),
            entry.receiver_id.to_ascii_lowercase(),
        )
    });
}

fn cached_receiver_matches(receiver: &CachedReceiver, selector_text: &str) -> bool {
    let selector = selector_text.trim().to_ascii_lowercase();
    let id = receiver.id.to_ascii_lowercase();
    let name = receiver.name.to_ascii_lowercase();
    let host = receiver.host.to_ascii_lowercase();

    id == selector || host == selector || name == selector || name.contains(&selector)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::config::AppConfig;
    use crate::pairing::{ReceiverAuthFlow, ReceiverCredentials};
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };

    use super::{AppFacade, DiscoveryService, SessionState, save_config};

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
        std::env::temp_dir().join(format!("rairstream-cli-facade-{unique}.json"))
    }

    fn build_receiver(id: &str, name: &str, host: &str) -> Receiver {
        Receiver {
            id: id.to_string(),
            name: name.to_string(),
            host: host.to_string(),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            transport_profile: ReceiverKind::ClassicRaop,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields()
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

    #[test]
    fn discover_returns_sorted_receivers() {
        let path = temp_config_path();
        let mut facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: vec![
                    build_receiver("zed", "Zed", "192.168.1.40"),
                    build_receiver("alpha", "Alpha", "192.168.1.20"),
                ],
            },
            path.clone(),
        )
        .unwrap();

        let receivers = facade.discover().unwrap();

        assert_eq!(receivers[0].name, "Alpha");
        assert_eq!(receivers[1].name, "Zed");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn discover_reports_no_receivers_and_resets_state() {
        let path = temp_config_path();
        let mut facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: Vec::new(),
            },
            path.clone(),
        )
        .unwrap();

        let error = facade.discover().unwrap_err();

        assert!(matches!(
            error,
            crate::error::RairstreamError::NoReceiversDiscovered
        ));
        assert_eq!(facade.state().session, SessionState::Idle);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn paired_list_is_sorted_by_display_name() {
        let path = temp_config_path();
        let mut config = AppConfig::default();
        config.upsert_paired_receiver("zed", paired_credentials(ReceiverAuthFlow::Modern));
        config.upsert_paired_receiver("alpha", paired_credentials(ReceiverAuthFlow::LegacyPin));
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("zed"),
            name: String::from("Zed"),
            host: String::from("192.168.1.40"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("alpha"),
            name: String::from("Alpha"),
            host: String::from("192.168.1.20"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        save_config(&path, &config).unwrap();

        let facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: Vec::new(),
            },
            path.clone(),
        )
        .unwrap();

        let entries = facade.paired_list();

        assert_eq!(entries[0].receiver_id, "alpha");
        assert_eq!(entries[1].receiver_id, "zed");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn paired_forget_resolves_cached_name_without_discovery() {
        let path = temp_config_path();
        let mut config = AppConfig::default();
        config.upsert_paired_receiver("kitchen", paired_credentials(ReceiverAuthFlow::LegacyPin));
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("kitchen"),
            name: String::from("Kitchen"),
            host: String::from("192.168.1.30"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        save_config(&path, &config).unwrap();

        let mut facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: Vec::new(),
            },
            path.clone(),
        )
        .unwrap();

        let entry = facade.paired_forget("kitch").unwrap();

        assert_eq!(entry.receiver_id, "kitchen");
        assert!(facade.paired_list().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn paired_forget_rejects_unpaired_cached_receiver() {
        let path = temp_config_path();
        let mut config = AppConfig::default();
        config.upsert_receiver_cache(crate::config::CachedReceiver {
            id: String::from("office"),
            name: String::from("Office"),
            host: String::from("192.168.1.50"),
            port: 7000,
            transport_profile: ReceiverKind::ClassicRaop,
            receiver_kind: ReceiverKind::ClassicRaop,
        });
        save_config(&path, &config).unwrap();

        let mut facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: Vec::new(),
            },
            path.clone(),
        )
        .unwrap();

        let error = facade.paired_forget("Office").unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid input: receiver `Office` has no saved pairing"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn play_file_resets_state_after_failure() {
        let path = temp_config_path();
        let mut facade = AppFacade::with_config_path(
            FixedDiscoveryService {
                receivers: vec![build_receiver("living-room", "Living Room", "192.168.1.20")],
            },
            path.clone(),
        )
        .unwrap();
        let missing_file = std::env::temp_dir().join(format!(
            "rairstream-missing-{}.wav",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let result = facade.play_file(&missing_file, &[String::from("Living Room")]);

        assert!(result.is_err());
        assert_eq!(facade.state().session, SessionState::Idle);
        let _ = std::fs::remove_file(path);
    }
}
