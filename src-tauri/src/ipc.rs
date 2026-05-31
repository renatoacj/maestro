//! Camada IPC: a única superfície que o frontend pode chamar. Cada comando é
//! tipado e fino — só delega ao núcleo. Nenhum exec arbitrário é exposto.

use tauri::State;

use crate::core::registry::Registry;
use crate::error::Result;
use crate::model::{Action, Job, Resources};

/// Lista todos os jobs de todos os providers disponíveis.
#[tauri::command]
pub async fn list_jobs(registry: State<'_, Registry>) -> Result<Vec<Job>> {
    Ok(registry.list_all().await)
}

/// Executa uma ação de ciclo de vida sobre um job (id global).
#[tauri::command]
pub async fn control_job(
    registry: State<'_, Registry>,
    id: String,
    action: Action,
) -> Result<()> {
    registry.control(&id, action).await
}

/// Lê o consumo de recursos atual de um job (id global).
#[tauri::command]
pub async fn job_metrics(registry: State<'_, Registry>, id: String) -> Result<Resources> {
    registry.metrics(&id).await
}
