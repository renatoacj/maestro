//! Modelo de dados normalizado. Todo provider devolve `Job`s nesta forma,
//! independentemente do mecanismo (systemd, Docker, cron, …).

use serde::{Deserialize, Serialize};

/// O tipo de job. Define como a UI o apresenta e quais ações fazem sentido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Processo contínuo (ex: um daemon, um servidor).
    Service,
    /// Roda por agendamento (timer do systemd, cron).
    Scheduled,
    /// Container (Docker).
    Container,
}

/// Estado de execução, normalizado entre mecanismos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Active,
    Inactive,
    Failed,
    Activating,
    Deactivating,
    Unknown,
}

/// Leitura de saúde de alto nível, derivada do estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Degraded,
    Failed,
}

/// Agendamento — só para jobs `Scheduled`. Epoch em segundos (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub next_run: Option<i64>,
    pub last_run: Option<i64>,
}

/// Consumo de recursos de um job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    /// Uso de CPU em porcento (0–100 por core somado). `None` se desconhecido.
    pub cpu_pct: Option<f32>,
    /// Memória residente em bytes. `None` se desconhecido.
    pub mem_bytes: Option<u64>,
    /// PIDs associados ao job.
    pub pids: Vec<u32>,
}

/// Um job de background normalizado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// Id global qualificado: `"systemd-user:my-worker.service"`.
    pub id: String,
    /// Provider de origem: `"systemd-user"`, `"docker"`, …
    pub provider: String,
    /// Id local dentro do provider: `"my-worker.service"`.
    pub local_id: String,
    pub kind: JobKind,
    pub name: String,
    pub description: String,
    /// Comando executado, quando o provider sabe informar.
    pub command: Option<String>,
    pub state: JobState,
    /// Sobe no boot? (autostart habilitado)
    pub enabled: bool,
    pub schedule: Option<Schedule>,
    pub resources: Resources,
    pub health: Health,
}

impl Job {
    /// Monta o id global a partir do provider e do id local.
    pub fn global_id(provider: &str, local_id: &str) -> String {
        format!("{provider}:{local_id}")
    }

    /// Quebra um id global em `(provider, local_id)`.
    /// Divide no primeiro `:` — ids locais podem conter `:` (ex: nomes de container).
    pub fn split_id(global_id: &str) -> Option<(&str, &str)> {
        global_id.split_once(':')
    }
}

/// Informação rica de um job, buscada sob demanda ao abrir o painel de detalhe.
/// Complementa o [`Job`] que a UI já tem da listagem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JobDetail {
    /// Comando executado (ExecStart), quando aplicável.
    pub command: Option<String>,
    /// Caminho do arquivo da unit (FragmentPath).
    pub fragment_path: Option<String>,
    /// Código de saída do processo principal (ExecMainStatus), quando houve.
    pub exit_code: Option<i32>,
    /// Motivo do último término (Result): "success", "exit-code", "signal", …
    pub exit_reason: Option<String>,
    /// Desde quando está no estado atual (ActiveEnterTimestamp, epoch s).
    pub since: Option<i64>,
}

/// Ações de ciclo de vida que um provider pode executar sobre um job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    /// Habilita autostart no boot.
    Enable,
    /// Desabilita autostart no boot.
    Disable,
    /// Dispara uma execução agora (jobs `Scheduled`), sem mexer no agendamento.
    TriggerNow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_id_round_trips() {
        let id = Job::global_id("systemd-user", "my-worker.service");
        assert_eq!(id, "systemd-user:my-worker.service");
        assert_eq!(
            Job::split_id(&id),
            Some(("systemd-user", "my-worker.service"))
        );
    }

    #[test]
    fn split_id_divides_on_first_colon() {
        // ids locais com ':' continuam intactos
        assert_eq!(
            Job::split_id("docker:my:container"),
            Some(("docker", "my:container"))
        );
    }

    #[test]
    fn split_id_rejects_unqualified() {
        assert_eq!(Job::split_id("semprovedor"), None);
    }
}
