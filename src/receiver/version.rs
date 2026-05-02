use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }

        let mut parts = value.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |part| part.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |part| part.parse().ok())?;

        Some(Self::new(major, minor, patch))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn parses_one_two_and_three_part_versions() {
        assert_eq!(Version::parse("366").unwrap(), Version::new(366, 0, 0));
        assert_eq!(Version::parse("366.1").unwrap(), Version::new(366, 1, 0));
        assert_eq!(Version::parse("366.1.2").unwrap(), Version::new(366, 1, 2));
    }
}
