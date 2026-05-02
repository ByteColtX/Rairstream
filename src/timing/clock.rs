use std::time::{SystemTime, UNIX_EPOCH};

#[must_use]
pub fn ntp_timestamp_now() -> u64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs().saturating_add(2_208_988_800);
    let fractional = ((u128::from(duration.subsec_nanos())) << 32) / 1_000_000_000_u128;

    (seconds << 32) | u64::try_from(fractional).unwrap_or(u64::MAX)
}
