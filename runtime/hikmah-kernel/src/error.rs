use thiserror::Error;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("integrity error at ledger sequence {seq}: {message}")]
    Integrity { seq: u64, message: String },
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("trace not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, KernelError>;
