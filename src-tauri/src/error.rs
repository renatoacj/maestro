//! Tipo de erro do núcleo. Serializa como string para atravessar o IPC do Tauri.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("provider indisponível: {0}")]
    Unavailable(String),

    #[error("job não encontrado: {0}")]
    NotFound(String),

    #[error("id de job inválido (esperado 'provider:local_id'): {0}")]
    InvalidId(String),

    #[error("operação não suportada: {0}")]
    Unsupported(String),

    #[error("erro de D-Bus: {0}")]
    Dbus(#[from] zbus::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// O frontend recebe o erro como string legível.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
