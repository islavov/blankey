use crate::crypto;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("Vault is locked")]
    Locked,
    #[error("Wrong password or key")]
    WrongSecret,
    #[error("Vault is not initialized")]
    NotInitialized,
    #[error("Vault is already initialized")]
    AlreadyInitialized,
    #[error("Unlock Blankey once to enable requests from AI assistants")]
    NoRequestKey,
    #[error("{0}")]
    NotFound(String),
    #[error("Vault schema version {0} is newer than this app supports")]
    Schema(i64),
    #[error("Keychain: {0}")]
    Keychain(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, VaultError>;
