use thiserror::Error;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("midly parse error: {0}")]
    Parse(#[from] midly::Error),
}
