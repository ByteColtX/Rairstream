use super::clock::ntp_timestamp_now;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtpClock;

impl NtpClock {
    #[must_use]
    pub fn now() -> u64 {
        ntp_timestamp_now()
    }
}
