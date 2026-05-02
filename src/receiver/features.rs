use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Features(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    None,
    LegacyPin,
    HomeKitTransient,
    FairPlayRequired,
    MfiRequired,
}

impl Default for AuthMethod {
    fn default() -> Self {
        Self::None
    }
}

impl Features {
    pub const AUTHENTICATION_FAIRPLAY: u64 = 1 << 14;
    pub const AUTHENTICATION_MFI: u64 = 1 << 26;
    pub const SUPPORTS_LEGACY_PAIRING: u64 = 1 << 27;
    pub const SUPPORTS_BUFFERED_AUDIO: u64 = 1 << 40;
    pub const SUPPORTS_PTP: u64 = 1 << 41;
    pub const SUPPORTS_HOMEKIT_PAIRING: u64 = 1 << 46;
    pub const SUPPORTS_TRANSIENT_PAIRING: u64 = 1 << 48;
    pub const SUPPORTS_UNIFIED_PAIR_MFI: u64 = 1 << 51;
    pub const SUPPORTS_RFC2198_REDUNDANCY: u64 = 1 << 61;

    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn from_txt_value(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }

        fn parse_hex(part: &str) -> Option<u64> {
            let normalized = part
                .trim()
                .strip_prefix("0x")
                .or_else(|| part.trim().strip_prefix("0X"))
                .unwrap_or(part.trim());
            u64::from_str_radix(normalized, 16).ok()
        }

        let (lower, upper) = match value.split_once(',') {
            Some((lower, upper)) => (parse_hex(lower)?, parse_hex(upper)?),
            None => (parse_hex(value)?, 0),
        };

        Some(Self(lower | (upper << 32)))
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub fn to_txt_value(self) -> String {
        let lower = self.0 & 0xffff_ffff;
        let upper = self.0 >> 32;
        if upper == 0 {
            format!("0x{lower:X}")
        } else {
            format!("0x{lower:X},0x{upper:X}")
        }
    }

    #[must_use]
    pub const fn requires_fairplay(self) -> bool {
        self.0 & Self::AUTHENTICATION_FAIRPLAY != 0
    }

    #[must_use]
    pub const fn requires_mfi(self) -> bool {
        self.0 & Self::AUTHENTICATION_MFI != 0
    }

    #[must_use]
    pub const fn supports_legacy_pairing(self) -> bool {
        self.0 & Self::SUPPORTS_LEGACY_PAIRING != 0
    }

    #[must_use]
    pub const fn supports_buffered_audio(self) -> bool {
        self.0 & Self::SUPPORTS_BUFFERED_AUDIO != 0
    }

    #[must_use]
    pub const fn supports_ptp(self) -> bool {
        self.0 & Self::SUPPORTS_PTP != 0
    }

    #[must_use]
    pub const fn supports_homekit_pairing(self) -> bool {
        self.0 & Self::SUPPORTS_HOMEKIT_PAIRING != 0
    }

    #[must_use]
    pub const fn supports_transient_pairing(self) -> bool {
        self.0 & Self::SUPPORTS_TRANSIENT_PAIRING != 0
    }

    #[must_use]
    pub const fn supports_unified_pair_mfi(self) -> bool {
        self.0 & Self::SUPPORTS_UNIFIED_PAIR_MFI != 0
    }

    #[must_use]
    pub const fn supports_rfc2198_redundancy(self) -> bool {
        self.0 & Self::SUPPORTS_RFC2198_REDUNDANCY != 0
    }

    #[must_use]
    pub fn auth_method(self) -> AuthMethod {
        if self.requires_mfi() {
            return AuthMethod::MfiRequired;
        }

        if self.supports_unified_pair_mfi()
            || self.supports_transient_pairing()
            || self.supports_homekit_pairing()
        {
            return AuthMethod::HomeKitTransient;
        }

        if self.requires_fairplay() {
            return AuthMethod::FairPlayRequired;
        }

        if self.supports_legacy_pairing() {
            return AuthMethod::LegacyPin;
        }

        AuthMethod::None
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthMethod, Features};

    #[test]
    fn parses_single_value_features() {
        let features = Features::from_txt_value("0x200").unwrap();

        assert_eq!(features.raw(), 0x200);
        assert_eq!(features.to_txt_value(), "0x200");
    }

    #[test]
    fn parses_two_part_features() {
        let features = Features::from_txt_value("0x40000A00,0x80300").unwrap();

        assert!(features.supports_buffered_audio());
        assert!(features.supports_ptp());
        assert!(features.supports_unified_pair_mfi());
        assert_eq!(features.auth_method(), AuthMethod::HomeKitTransient);
    }

    #[test]
    fn mfi_authentication_takes_priority() {
        let features = Features::from_txt_value("0x445F8A00,0x1C340").unwrap();

        assert!(features.requires_mfi());
        assert_eq!(features.auth_method(), AuthMethod::MfiRequired);
    }
}
