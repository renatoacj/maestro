//! A abstração central do Maestro.
//!
//! Toda a aplicação conversa apenas com a trait [`JobProvider`]. Adicionar suporte
//! a um novo mecanismo (launchd, Task Scheduler, cron, …) é escrever **um** provider,
//! sem tocar no núcleo. É isso que torna o "universal cross-platform" um incremento
//! e não uma reescrita.

use std::pin::Pin;

use crate::error::{Error, Result};
use crate::model::{Action, Job, JobDetail, Resources};
use async_trait::async_trait;
use futures_util::Stream;

pub mod docker;
pub mod systemd;

/// Stream de "algo mudou" empurrado por um provider (sem payload — sinaliza que
/// vale relistar). `'static` para poder viver numa task de background.
pub type ChangeStream = Pin<Box<dyn Stream<Item = ()> + Send>>;

#[cfg(test)]
pub mod fake;

/// Fonte de jobs de background de um mecanismo específico.
///
/// Implementações devem ser `Send + Sync`: o núcleo as compartilha entre tasks.
/// Métodos que falham por indisponibilidade do mecanismo devem retornar
/// [`crate::error::Error::Unavailable`], nunca dar panic.
#[async_trait]
pub trait JobProvider: Send + Sync {
    /// Identificador estável do provider (ex: `"systemd-user"`). Vira o prefixo do id global.
    fn id(&self) -> &'static str;

    /// O mecanismo existe e está acessível nesta máquina?
    /// Providers indisponíveis são exibidos como tal, sem derrubar o app.
    async fn available(&self) -> bool;

    /// Lista todos os jobs conhecidos por este provider.
    async fn list(&self) -> Result<Vec<Job>>;

    /// Executa uma ação de ciclo de vida sobre um job (id local, sem o prefixo do provider).
    async fn control(&self, local_id: &str, action: Action) -> Result<()>;

    /// Lê o consumo de recursos atual de um job (id local).
    async fn metrics(&self, local_id: &str) -> Result<Resources>;

    /// Stream opcional de mudanças (push). `None` se o provider não suporta —
    /// nesse caso o núcleo cai para amostragem periódica.
    async fn watch(&self) -> Option<ChangeStream> {
        None
    }

    /// Detalhe sob demanda de um job (comando, motivo de falha, etc.).
    async fn detail(&self, _local_id: &str) -> Result<JobDetail> {
        Err(Error::Unsupported("detail".into()))
    }

    /// Últimas `lines` linhas de log do job.
    async fn logs(&self, _local_id: &str, _lines: u32) -> Result<Vec<String>> {
        Err(Error::Unsupported("logs".into()))
    }
}
