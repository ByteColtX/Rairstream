mod capabilities;
mod features;
pub mod selector;
mod version;

use serde::{Deserialize, Serialize};

pub use capabilities::{CodecKind, PairingRequirement, ReceiverCapabilities};
pub use features::{AuthMethod, Features};
pub use version::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirPlayGeneration {
    AirPlay1,
    AirPlay2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProfile {
    #[serde(alias = "classic_raop")]
    Raop,
    #[serde(alias = "modern_airplay_auth")]
    ModernAuthRaop,
}

impl Default for TransportProfile {
    fn default() -> Self {
        Self::Raop
    }
}

impl TransportProfile {
    #[allow(non_upper_case_globals)]
    pub const ClassicRaop: Self = Self::Raop;
    #[allow(non_upper_case_globals)]
    pub const ModernAirPlayAuth: Self = Self::ModernAuthRaop;

    #[must_use]
    pub const fn is_modern(self) -> bool {
        matches!(self, Self::ModernAuthRaop)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportReason {
    FairPlayUnsupported,
    MfiAuthenticationRequired,
    PlatformCaptureUnsupported,
    RuntimePathDisabled,
}

impl SupportReason {
    #[allow(non_upper_case_globals)]
    pub const AuthenticationRequiredReceiver: Self = Self::RuntimePathDisabled;
    #[allow(non_upper_case_globals)]
    pub const ExperimentalAirPlay2: Self = Self::RuntimePathDisabled;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    #[default]
    Supported,
    Experimental {
        reason: SupportReason,
    },
    Unsupported {
        reason: SupportReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaopEncryptionType {
    None,
    Rsa,
    FairPlay,
    MfiSap,
    FairPlaySapV25,
    Unknown(u8),
}

impl RaopEncryptionType {
    #[must_use]
    pub const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::None,
            1 => Self::Rsa,
            3 => Self::FairPlay,
            4 => Self::MfiSap,
            5 => Self::FairPlaySapV25,
            unknown => Self::Unknown(unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RaopMetadata {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub source_version: Version,
    #[serde(default)]
    pub status_flags: u64,
    #[serde(default)]
    pub requires_password: bool,
    #[serde(default)]
    pub vodka_version: Option<String>,
    #[serde(default)]
    pub codecs: Vec<CodecKind>,
    #[serde(default)]
    pub encryption_types: Vec<RaopEncryptionType>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub metadata_types: Vec<u8>,
    #[serde(default)]
    pub digest_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receiver {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub generation: AirPlayGeneration,
    #[serde(default, alias = "receiver_kind")]
    pub transport_profile: TransportProfile,
    #[serde(default, alias = "support")]
    pub support_level: SupportLevel,
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub source_version: Version,
    #[serde(default)]
    pub firmware_version: Option<String>,
    #[serde(default)]
    pub os_version: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub features: Features,
    #[serde(default)]
    pub required_sender_features: Features,
    #[serde(default)]
    pub status_flags: u64,
    #[serde(default)]
    pub requires_password: bool,
    #[serde(default)]
    pub access_control: Option<u8>,
    #[serde(default, alias = "pairing_id")]
    pub pairing_identity: Option<String>,
    #[serde(default)]
    pub system_pairing_identity: Option<String>,
    #[serde(default)]
    pub receiver_public_key: Option<String>,
    #[serde(default)]
    pub bluetooth_address: Option<String>,
    #[serde(default)]
    pub homekit_home_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub is_group_leader: bool,
    #[serde(default)]
    pub group_public_name: Option<String>,
    #[serde(default)]
    pub group_contains_discoverable_leader: bool,
    #[serde(default)]
    pub home_group_id: Option<String>,
    #[serde(default)]
    pub household_id: Option<String>,
    #[serde(default)]
    pub parent_group_id: Option<String>,
    #[serde(default)]
    pub parent_group_contains_discoverable_leader: bool,
    #[serde(default)]
    pub tight_sync_id: Option<String>,
    #[serde(default)]
    pub capabilities: ReceiverCapabilities,
    #[serde(default)]
    pub raop: RaopMetadata,
    #[doc(hidden)]
    #[serde(default, skip_deserializing)]
    pub receiver_kind: TransportProfile,
    #[doc(hidden)]
    #[serde(default, skip_deserializing)]
    pub support: SupportLevel,
    #[doc(hidden)]
    #[serde(default, skip_deserializing)]
    pub pairing_id: Option<String>,
}

impl Receiver {
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    #[must_use]
    pub fn with_compat_fields(mut self) -> Self {
        self.receiver_kind = self.transport_profile;
        self.support = self.support_level.clone();
        self.pairing_id = self.pairing_identity.clone();
        self
    }
}

impl Default for Receiver {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: String::new(),
            port: 0,
            generation: AirPlayGeneration::AirPlay1,
            transport_profile: TransportProfile::default(),
            support_level: SupportLevel::default(),
            auth_method: AuthMethod::default(),
            model: None,
            manufacturer: None,
            serial_number: None,
            source_version: Version::default(),
            firmware_version: None,
            os_version: None,
            protocol_version: None,
            features: Features::default(),
            required_sender_features: Features::default(),
            status_flags: 0,
            requires_password: false,
            access_control: None,
            pairing_identity: None,
            system_pairing_identity: None,
            receiver_public_key: None,
            bluetooth_address: None,
            homekit_home_id: None,
            group_id: None,
            is_group_leader: false,
            group_public_name: None,
            group_contains_discoverable_leader: false,
            home_group_id: None,
            household_id: None,
            parent_group_id: None,
            parent_group_contains_discoverable_leader: false,
            tight_sync_id: None,
            capabilities: ReceiverCapabilities::default(),
            raop: RaopMetadata::default(),
            receiver_kind: TransportProfile::default(),
            support: SupportLevel::default(),
            pairing_id: None,
        }
    }
}

#[doc(hidden)]
pub type ReceiverKind = TransportProfile;
#[doc(hidden)]
pub type DeviceSupport = SupportLevel;
#[doc(hidden)]
pub type UnsupportedReason = SupportReason;
