//! Loops de background que empurram estado para a UI via eventos Tauri.
//!
//! - **watcher**: reage a sinais do systemd (push) e emite `jobs` ao mudar de estado,
//!   com coalescência de rajadas e reconexão com backoff exponencial.
//! - **sampler**: amostra CPU/memória dos jobs ativos a cada 2s e emite `metrics`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{stream::select_all, StreamExt};
use tauri::{AppHandle, Emitter, Manager};

use crate::core::registry::Registry;
use crate::model::JobState;

/// Evento com a lista completa de jobs (Vec<Job>).
pub const JOBS_EVENT: &str = "jobs";
/// Evento com o mapa id→Resources dos jobs ativos.
pub const METRICS_EVENT: &str = "metrics";

const DEBOUNCE: Duration = Duration::from_millis(200);
const SAMPLE_EVERY: Duration = Duration::from_secs(2);
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Sobe os dois loops. Não bloqueia.
pub fn spawn(registry: Arc<Registry>, app: AppHandle) {
    spawn_watcher(registry.clone(), app.clone());
    spawn_sampler(registry, app);
}

async fn emit_jobs(registry: &Registry, app: &AppHandle) {
    let _ = app.emit(JOBS_EVENT, registry.list_all().await);
}

/// Reage a mudanças de estado do systemd e re-emite a lista. Sem polling:
/// só relista quando o systemd avisa que algo mudou.
fn spawn_watcher(registry: Arc<Registry>, app: AppHandle) {
    // `tauri::async_runtime::spawn` garante o contexto do runtime Tokio
    // (a closure `setup` não roda dentro dele).
    tauri::async_runtime::spawn(async move {
        emit_jobs(&registry, &app).await; // estado inicial
        let mut backoff = BACKOFF_MIN;

        loop {
            let streams = registry.watch_streams().await;
            if streams.is_empty() {
                // Nenhum provider com push disponível: heartbeat lento como fallback.
                tokio::time::sleep(Duration::from_secs(5)).await;
                emit_jobs(&registry, &app).await;
                continue;
            }

            let mut merged = select_all(streams);
            let mut lived = false;
            while merged.next().await.is_some() {
                lived = true;
                // Coalesce rajadas (um `daemon-reload` dispara muitos sinais).
                tokio::time::sleep(DEBOUNCE).await;
                tracing::debug!("sinal do systemd → re-emitindo jobs");
                emit_jobs(&registry, &app).await;
            }

            // Streams encerraram → provavelmente a conexão caiu. Conexão que durou
            // zera o backoff; quedas em sequência crescem o intervalo.
            if lived {
                backoff = BACKOFF_MIN;
            }
            tracing::warn!(?backoff, "watch encerrou; reconectando");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
}

/// Amostra recursos dos jobs ativos periodicamente (CPU/mem mudam continuamente,
/// então isso é amostragem legítima — não polling de estado).
fn spawn_sampler(registry: Arc<Registry>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(SAMPLE_EVERY);
        loop {
            interval.tick().await;
            // Não amostra quando a janela está escondida (bandeja): economiza
            // CPU e tráfego D-Bus enquanto ninguém está olhando.
            let visible = app
                .get_webview_window("main")
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(true);
            if !visible {
                continue;
            }
            let jobs = registry.list_all().await;
            let mut map: HashMap<String, crate::model::Resources> = HashMap::new();
            for j in jobs.iter().filter(|j| matches!(j.state, JobState::Active)) {
                if let Ok(r) = registry.metrics(&j.id).await {
                    map.insert(j.id.clone(), r);
                }
            }
            if !map.is_empty() {
                let _ = app.emit(METRICS_EVENT, map);
            }
        }
    });
}
