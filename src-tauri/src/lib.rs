mod api;
mod config;
mod db;
mod otlp;

use std::sync::Arc;

use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let paths = match config::Paths::resolve_and_prepare() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = ?e, "failed to prepare andon data directory");
            std::process::exit(1);
        }
    };
    tracing::info!(data_dir = %paths.data_dir.display(), "andon starting");

    let pool = match db::init(&paths.db_path) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!(error = ?e, "failed to initialise database");
            std::process::exit(1);
        }
    };

    let control = otlp::IngestionControl::new();

    let api_state = api::ApiState {
        pool: pool.clone(),
        db_path: paths.db_path.clone(),
        control: control.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |_app| {
            let api_state = api_state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = api::serve(api_state).await {
                    tracing::error!(error = ?e, "api server exited with error");
                }
            });

            let pool = pool.clone();
            let control = control.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = otlp::serve(pool, control).await {
                    tracing::error!(error = ?e, "otlp server exited with error");
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
