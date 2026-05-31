//! Maestro — cockpit de jobs de background.
//!
//! Wiring do app: monta o [`Registry`] com os providers disponíveis, expõe os
//! comandos IPC e sobe a janela Tauri.

mod core;
mod error;
mod ipc;
mod model;
mod provider;

use std::sync::Arc;

use crate::core::registry::Registry;
use crate::provider::systemd::SystemdUserProvider;
use crate::provider::JobProvider;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "maestro=info".into()),
        )
        .init();

    // Descobre os providers disponíveis nesta máquina antes de subir a UI.
    let registry = Arc::new(tauri::async_runtime::block_on(build_registry()));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(registry.clone())
        .invoke_handler(tauri::generate_handler![
            ipc::list_jobs,
            ipc::control_job,
            ipc::job_metrics,
            ipc::job_detail,
            ipc::job_logs,
        ])
        .setup(move |app| {
            // Tray é não-fatal: se a libayatana-appindicator não estiver presente,
            // o app continua funcionando normalmente, apenas sem ícone na bandeja.
            if let Err(e) = setup_tray(app) {
                tracing::warn!(error = %e, "tray indisponível (falta libayatana-appindicator?)");
            }
            setup_close_to_tray(app);
            // Loops de estado/métricas (push para a UI).
            crate::core::events::spawn(registry.clone(), app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar a aplicação Tauri");
}

/// Traz a janela principal de volta à frente (mostra, desminimiza, foca).
fn reveal_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Ícone na bandeja do sistema, com menu (Mostrar / Sair) e clique para revelar.
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Mostrar Maestro", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("maestro-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Maestro")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Fechar (✕) esconde a janela para a bandeja em vez de encerrar; o app só sai
/// de fato pelo "Sair" no menu da bandeja. Minimizar mantém o comportamento
/// normal do sistema (vai para o dock/barra de tarefas).
fn setup_close_to_tray(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let w = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = w.hide();
            }
        });
    }
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

    match crate::provider::docker::DockerProvider::connect().await {
        Ok(p) => {
            tracing::info!("provider docker conectado");
            providers.push(Box::new(p));
        }
        Err(e) => tracing::debug!(error = %e, "docker indisponível (ok se não usa Docker)"),
    }

    // Próximos incrementos: CronProvider, LaunchdProvider, WindowsProvider…

    Registry::new(providers)
}
