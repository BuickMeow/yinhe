use thiserror::Error;

#[derive(Error, Debug)]
pub enum MidiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MIDI parse error: {0}")]
    MidiParse(#[from] midly::Error),

    #[error("Invalid MIDI data: {0}")]
    InvalidData(String),
}
