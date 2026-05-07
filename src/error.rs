use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("input error: {0}")]
    Input(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("engine error: {0}")]
    Engine(String),

    #[error("model error: {0}")]
    Model(String),

    #[error("output error: {0}")]
    Output(String),

    #[error("queue error: {0}")]
    Queue(String),

    #[error("blob error: {0}")]
    Blob(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
