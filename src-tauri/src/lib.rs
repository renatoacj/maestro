//! Maestro — cockpit de jobs de background.
//!
//! Wiring do app: monta o [`Registry`] com os providers disponíveis, expõe os
//! comandos IPC e sobe a janela Tauri.

mod core;
mod error;
mod ipc;
mod model;
mod provider;

use crate::core::registry::Registry;
use crate::provider::systemd::SystemdUserProvider;
use crate::provider::JobProvider;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "maestro=info".into()),
        )
        .init();

    // Descobre os providers disponíveis nesta máquina antes de subir a UI.
    let registry = tauri::async_runtime::block_on(build_registry());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(registry)
        .invoke_handler(tauri::generate_handler![
            ipc::list_jobs,
            ipc::control_job,
            ipc::job_metrics,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao rodar a aplicação Tauri");
}

/// Monta o registry com cada provider que conseguir inicializar. Provider que
/// falha ao conectar é apenas registrado e omitido — o app sobe mesmo assim.
async fn build_registry() -> Registry {
    let mut providers: Vec<Box<dyn JobProvider>> = Vec::new();

    match SystemdUserProvider::connect().await {
        Ok(p) => {
            tracing::info!("provider systemd-user conectado");
            providers.push(Box::new(p));
        }
        Err(e) => tracing::warn!(error = %e, "systemd --user indisponível"),
    }

    // Próximos incrementos: DockerProvider, CronProvider, …

    Registry::new(providers)
}
