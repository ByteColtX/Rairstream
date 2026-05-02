#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtpClock;

impl PtpClock {
    #[must_use]
    pub const fn available() -> bool {
        false
    }
}
