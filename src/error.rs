use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("password required for this share link")]
    PasswordRequired,

    #[error("acw challenge: {0}")]
    Acw(String),

    #[error("parse: {0}")]
    Parse(String),

    #[error("http: {0}")]
    Http(String),

    #[error("cdn risk: {0}")]
    Cdn(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("request: {0}")]
    Request(#[from] reqwest::Error),

    #[error("url: {0}")]
    Url(#[from] url::ParseError),
}
