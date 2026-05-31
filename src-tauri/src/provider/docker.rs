//! Provider para containers Docker via socket local (bollard, sem shell).
//!
//! Prova a abstração `JobProvider` com um segundo mecanismo: a mesma interface
//! que serve o systemd serve o Docker. Inerte (não registrado) quando o daemon
//! Docker não está acessível.

use async_trait::async_trait;
use bollard::container::{
    ListContainersOptions, RestartContainerOptions, StartContainerOptions, StatsOptions,
    StopContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;

use crate::error::{Error, Result};
use crate::model::{Action, Health, Job, JobKind, JobState, Resources};
use crate::provider::JobProvider;

pub const PROVIDER_ID: &str = "docker";

fn other<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(format!("docker: {e}"))
}

pub struct DockerProvider {
    docker: Docker,
}

impl DockerProvider {
    /// Conecta ao daemon Docker (socket/named pipe local) e valida com um ping.
    pub async fn connect() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| Error::Unavailable(format!("docker: {e}")))?;
        docker
            .ping()
            .await
            .map_err(|e| Error::Unavailable(format!("docker ping: {e}")))?;
        Ok(Self { docker })
    }
}

fn map_state(state: &str) -> JobState {
    match state {
        "running" => JobState::Active,
        "restarting" => JobState::Activating,
        "removing" | "paused" => JobState::Deactivating,
        "created" | "exited" => JobState::Inactive,
        "dead" => JobState::Failed,
        _ => JobState::Unknown,
    }
}

fn map_health(state: JobState) -> Health {
    match state {
        JobState::Failed => Health::Failed,
        JobState::Activating | JobState::Deactivating => Health::Degraded,
        _ => Health::Ok,
    }
}

#[async_trait]
impl JobProvider for DockerProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    async fn available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    async fn list(&self) -> Result<Vec<Job>> {
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };
        let containers = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(other)?;

        let mut jobs = Vec::new();
        for c in containers {
            let id = c.id.clone().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let name = c
                .names
                .as_ref()
                .and_then(|n| n.first())
                .map(|s| s.trim_start_matches('/').to_string())
                .unwrap_or_else(|| id.chars().take(12).collect());
            let state = map_state(c.state.as_deref().unwrap_or(""));

            jobs.push(Job {
                id: Job::global_id(PROVIDER_ID, &id),
                provider: PROVIDER_ID.to_string(),
                local_id: id,
                kind: JobKind::Container,
                name,
                description: c.image.clone().unwrap_or_default(),
                command: c.command.clone(),
                state,
                enabled: false, // restart policy fica para um incremento futuro
                schedule: None,
                resources: Resources::default(),
                health: map_health(state),
            });
        }
        jobs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(jobs)
    }

    async fn control(&self, local_id: &str, action: Action) -> Result<()> {
        match action {
            Action::Start | Action::TriggerNow => {
                self.docker
                    .start_container(local_id, None::<StartContainerOptions<String>>)
                    .await
                    .map_err(other)?;
            }
            Action::Stop => {
                self.docker
                    .stop_container(local_id, None::<StopContainerOptions>)
                    .await
                    .map_err(other)?;
            }
            Action::Restart => {
                self.docker
                    .restart_container(local_id, None::<RestartContainerOptions>)
                    .await
                    .map_err(other)?;
            }
            Action::Enable | Action::Disable => {
                return Err(Error::Unsupported("autostart de container".into()));
            }
        }
        Ok(())
    }

    async fn metrics(&self, local_id: &str) -> Result<Resources> {
        let opts = StatsOptions {
            stream: false,
            one_shot: false,
        };
        let mut stream = self.docker.stats(local_id, Some(opts));
        let Some(stat) = stream.next().await else {
            return Ok(Resources::default());
        };
        let s = stat.map_err(other)?;

        let mut res = Resources::default();
        if let Some(usage) = s.memory_stats.usage {
            res.mem_bytes = Some(usage);
        }
        // Fórmula padrão de CPU% do Docker (delta de uso vs delta do sistema).
        let cpu_delta =
            s.cpu_stats.cpu_usage.total_usage as f64 - s.precpu_stats.cpu_usage.total_usage as f64;
        let sys_delta = s.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
            - s.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;
        let cpus = s.cpu_stats.online_cpus.unwrap_or(1).max(1) as f64;
        if sys_delta > 0.0 && cpu_delta >= 0.0 {
            res.cpu_pct = Some(((cpu_delta / sys_delta) * cpus * 100.0) as f32);
        }
        Ok(res)
    }

    async fn watch(&self) -> Option<crate::provider::ChangeStream> {
        let docker = self.docker.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(16);
        tokio::spawn(async move {
            let mut events = docker.events(None::<bollard::system::EventsOptions<String>>);
            while let Some(ev) = events.next().await {
                if ev.is_ok() && tx.send(()).await.is_err() {
                    break;
                }
            }
        });
        Some(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}
