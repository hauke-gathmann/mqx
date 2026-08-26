use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("could not determine mqx application directories")]
    Directories,
    #[error("failed to parse {}: {source}", .path.display())]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize config")]
    TomlSer(#[from] toml::ser::Error),
    #[error("failed to parse {}: {source}", .path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("profile {0} not found")]
    ProfileNotFound(String),
    #[error("invalid profile id {0:?}")]
    InvalidProfileId(String),
    #[error("failed to read {}: {source}", .path.display())]
    CertFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TLS material: {0}")]
    Tls(String),
    #[error("{0}")]
    Mqtt(String),
    #[error("{0}")]
    Recording(String),
}

pub type Result<T> = std::result::Result<T, Error>;
