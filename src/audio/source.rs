use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSourceKind {
    File(PathBuf),
    Capture,
}
