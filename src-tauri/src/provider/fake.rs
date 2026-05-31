//! Provider falso, usado só em testes — exercita a lógica do núcleo (Registry,
//! roteamento de ações) sem depender de systemd/D-Bus.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::{Error, Result};
use crate::model::{Action, Health, Job, JobKind, JobState, Resources};
use crate::provider::JobProvider;

pub struct FakeProvider {
    id: &'static str,
    available: bool,
    jobs: Vec<Job>,
    /// Registra as ações recebidas, para asserções nos testes.
    pub calls: Mutex<Vec<(String, Action)>>,
}

impl FakeProvider {
    pub fn new(id: &'static str, available: bool, local_ids: &[&str]) -> Self {
        let jobs = local_ids
            .iter()
            .map(|lid| Job {
                id: Job::global_id(id, lid),
                provider: id.to_string(),
                local_id: lid.to_string(),
                kind: JobKind::Service,
                name: lid.to_string(),
                description: format!("fake {lid}"),
                command: None,
                state: JobState::Active,
                enabled: true,
                schedule: None,
                resources: Resources::default(),
                health: Health::Ok,
            })
            .collect();
        Self {
            id,
            available,
            jobs,
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl JobProvider for FakeProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    async fn available(&self) -> bool {
        self.available
    }

    async fn list(&self) -> Result<Vec<Job>> {
        if !self.available {
            return Err(Error::Unavailable(self.id.into()));
        }
        Ok(self.jobs.clone())
    }

    async fn control(&self, local_id: &str, action: Action) -> Result<()> {
        if !self.jobs.iter().any(|j| j.local_id == local_id) {
            return Err(Error::NotFound(local_id.into()));
        }
        self.calls
            .lock()
            .unwrap()
            .push((local_id.to_string(), action));
        Ok(())
    }

    async fn metrics(&self, local_id: &str) -> Result<Resources> {
        if !self.jobs.iter().any(|j| j.local_id == local_id) {
            return Err(Error::NotFound(local_id.into()));
        }
        Ok(Resources {
            cpu_pct: Some(1.5),
            mem_bytes: Some(1024),
            pids: vec![42],
        })
    }
}
