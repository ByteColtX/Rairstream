#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspEvent {
    Connected,
    Recording,
    Teardown,
}
