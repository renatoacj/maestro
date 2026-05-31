//! Agrega todos os providers numa visão unificada e roteia ações pelo id global.

use crate::error::{Error, Result};
use crate::model::{Action, Job, Resources};
use crate::provider::{ChangeStream, JobProvider};

pub struct Registry {
    providers: Vec<Box<dyn JobProvider>>,
}

impl Registry {
    pub fn new(providers: Vec<Box<dyn JobProvider>>) -> Self {
        Self { providers }
    }

    /// Lista os jobs de todos os providers. Um provider que falhar é registrado
    /// e ignorado — a falha de um mecanismo não derruba a visão dos outros.
    pub async fn list_all(&self) -> Vec<Job> {
        let mut all = Vec::new();
        for p in &self.providers {
            match p.list().await {
                Ok(mut jobs) => all.append(&mut jobs),
                Err(e) => {
                    tracing::warn!(provider = p.id(), error = %e, "falha ao listar provider");
                }
            }
        }
        all
    }

    fn route(&self, global_id: &str) -> Result<(&dyn JobProvider, String)> {
        let (provider_id, local_id) =
            Job::split_id(global_id).ok_or_else(|| Error::InvalidId(global_id.into()))?;
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == provider_id)
            .ok_or_else(|| Error::Unavailable(provider_id.into()))?;
        Ok((provider.as_ref(), local_id.to_string()))
    }

    /// Executa uma ação sobre um job identificado pelo id global.
    pub async fn control(&self, global_id: &str, action: Action) -> Result<()> {
        let (provider, local_id) = self.route(global_id)?;
        provider.control(&local_id, action).await
    }

    /// Lê recursos de um job pelo id global.
    pub async fn metrics(&self, global_id: &str) -> Result<Resources> {
        let (provider, local_id) = self.route(global_id)?;
        provider.metrics(&local_id).await
    }

    /// Detalhe sob demanda de um job pelo id global.
    pub async fn detail(&self, global_id: &str) -> Result<crate::model::JobDetail> {
        let (provider, local_id) = self.route(global_id)?;
        provider.detail(&local_id).await
    }

    /// Últimas `lines` linhas de log de um job pelo id global.
    pub async fn logs(&self, global_id: &str, lines: u32) -> Result<Vec<String>> {
        let (provider, local_id) = self.route(global_id)?;
        provider.logs(&local_id, lines).await
    }

    /// Streams de mudança de todos os providers que suportam push.
    pub async fn watch_streams(&self) -> Vec<ChangeStream> {
        let mut streams = Vec::new();
        for p in &self.providers {
            if let Some(s) = p.watch().await {
                streams.push(s);
            }
        }
        streams
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Action;
    use crate::provider::fake::FakeProvider;

    fn registry() -> Registry {
        Registry::new(vec![
            Box::new(FakeProvider::new("systemd-user", true, &["a.service", "b.timer"])),
            Box::new(FakeProvider::new("docker", false, &[])),
        ])
    }

    #[tokio::test]
    async fn list_all_skips_unavailable_provider() {
        let reg = registry();
        let jobs = reg.list_all().await;
        // docker está indisponível → só os 2 do systemd entram.
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.provider == "systemd-user"));
    }

    #[tokio::test]
    async fn control_routes_to_correct_provider() {
        let reg = registry();
        reg.control("systemd-user:a.service", Action::Restart)
            .await
            .unwrap();
        // roteou para o provider e o id local certos
        let jobs = reg.list_all().await;
        assert!(jobs.iter().any(|j| j.local_id == "a.service"));
    }

    #[tokio::test]
    async fn control_rejects_unqualified_id() {
        let reg = registry();
        let err = reg.control("a.service", Action::Start).await.unwrap_err();
        assert!(matches!(err, Error::InvalidId(_)));
    }

    #[tokio::test]
    async fn control_rejects_unknown_provider() {
        let reg = registry();
        let err = reg
            .control("cron:x", Action::Start)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unavailable(_)));
    }

    #[tokio::test]
    async fn metrics_routes_and_returns() {
        let reg = registry();
        let res = reg.metrics("systemd-user:a.service").await.unwrap();
        assert_eq!(res.mem_bytes, Some(1024));
    }
}
