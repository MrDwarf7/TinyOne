use thiserror::Error;

pub type Result<T> = std::result::Result<T, TinyOneError>;

#[derive(Debug, Error)]
pub enum TinyOneError {
    #[error("{0}")]
    Compile(String),
    #[error("{0}")]
    Runtime(String),
}

impl TinyOneError {
    pub(crate) fn compile(message: impl Into<String>) -> Self {
        Self::Compile(message.into())
    }

    pub(crate) fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
}
