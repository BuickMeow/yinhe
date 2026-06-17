use thiserror::Error;

#[derive(Debug, Error)]
pub enum YinError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid magic bytes (expected b\"YINH\")")]
    BadMagic,
    #[error("unsupported version: got {0}, expected {expected}", expected = crate::VERSION)]
    BadVersion(u16),
    #[error("truncated file: needed {needed} bytes, only {available} remain")]
    Truncated { needed: usize, available: usize },
    #[error("invalid utf-8 in JSON section: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
}
