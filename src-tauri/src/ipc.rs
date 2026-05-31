//! Camada IPC: a única superfície que o frontend pode chamar. Cada comando é
//! tipado e fino — só delega ao núcleo. Nenhum exec arbitrário é exposto.

use std::sync::Arc;

use tauri::State;

use crate::core::registry::Registry;
use crate::error::Result;
use crate::model::{Action, Job, JobDetail, Resources};

/// Lista todos os jobs de todos os providers disponíveis.
#[tauri::command]
pub async fn list_jobs(registry: State<'_, Arc<Registry>>) -> Result<Vec<Job>> {
    Ok(registry.list_all().await)
}

/// Executa uma ação de ciclo de vida sobre um job (id global).
#[tauri::command]
pub async fn control_job(
    registry: State<'_, Arc<Registry>>,
    id: String,
    action: Action,
) -> Result<()> {
    registry.control(&id, action).await
}

/// Lê o consumo de recursos atual de um job (id global).
#[tauri::command]
pub async fn job_metrics(registry: State<'_, Arc<Registry>>, id: String) -> Result<Resources> {
    registry.metrics(&id).await
}

/// Detalhe sob demanda de um job (comando, motivo de falha, etc.).
#[tauri::command]
pub async fn job_detail(registry: State<'_, Arc<Registry>>, id: String) -> Result<JobDetail> {
    registry.detail(&id).await
}

/// Últimas `lines` linhas de log de um job.
#[tauri::command]
pub async fn job_logs(
    registry: State<'_, Arc<Registry>>,
    id: String,
    lines: u32,
) -> Result<Vec<String>> {
    registry.logs(&id, lines).await
}
