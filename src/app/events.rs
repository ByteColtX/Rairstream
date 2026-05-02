#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    ReceiversRefreshed,
    PairingSaved,
    PlaybackStarted,
    PlaybackStopped,
}
