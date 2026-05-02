#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepalivePolicy {
    pub interval_seconds: u64,
}

impl Default for KeepalivePolicy {
    fn default() -> Self {
        Self {
            interval_seconds: 15,
        }
    }
}
