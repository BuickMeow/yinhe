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
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("postcard error: {0}")]
    Postcard(#[from] postcard::Error),
}

/// 构造 InvalidData 错误（损坏的段数据统一入口）。
pub(crate) fn invalid_data(msg: impl Into<String>) -> YinError {
    YinError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.into(),
    ))
}
