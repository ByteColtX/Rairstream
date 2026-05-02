use rairstream::timing::{ntp::NtpClock, ptp::PtpClock};

#[test]
fn timing_modules_expose_clock_capabilities() {
    assert!(NtpClock::now() > 0);
    assert!(!PtpClock::available());
}
