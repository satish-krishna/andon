mod api;
mod autostart;
mod config;
mod db;
mod diagnostics;
mod integration;
mod otlp;

use std::sync::Mutex;

use std::sync::Arc;

use tauri::{
    Manager, WindowEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tracing_subscriber::{EnvFilter, prelude::*};

const MAIN_WINDOW: &str = "main";

struct AppState {
    control: otlp::IngestionControl,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = match config::Paths::resolve_and_prepare() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("failed to prepare andon data directory: {e:?}");
            std::process::exit(1);
        }
    };

    let file_appender = tracing_appender::rolling::daily(&paths.data_dir, "log.txt");
    let (file_writer, _file_guard) = tracing_appender::non_blocking(file_appender);
    // _file_guard must live as long as the process.
    Box::leak(Box::new(_file_guard));

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr_layer = tracing_subscriber::fmt::layer().with_target(false);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    tracing::info!(data_dir = %paths.data_dir.display(), "andon starting");

    let integration_status = integration::ensure_claude_settings();
    tracing::info!(?integration_status, "claude code integration check complete");

    match autostart::ensure_current() {
        Ok(outcome) => tracing::info!(?outcome, "autostart ensured"),
        Err(e) => tracing::warn!(error = ?e, "autostart setup failed (non-fatal)"),
    }

    let pool = match db::init(&paths.db_path) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!(error = ?e, "failed to initialise database");
            std::process::exit(1);
        }
    };

    let control = otlp::IngestionControl::new();
    let diagnostics = diagnostics::Diagnostics::new();
    diagnostics.seed_from_db(&pool);

    let api_state = api::ApiState {
        pool: pool.clone(),
        db_path: paths.db_path.clone(),
        control: control.clone(),
        integration: std::sync::Arc::new(Mutex::new(integration_status)),
        diagnostics: diagnostics.clone(),
    };

    let app_state = AppState {
        control: control.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .setup(move |app| {
            // API server
            let api_state_for_api = api_state.clone();
            let diag_for_api = diagnostics.clone();
            tauri::async_runtime::spawn(async move {
                match api::serve(api_state_for_api).await {
                    Ok(()) => diag_for_api.record_bind("api", true, None),
                    Err(e) => {
                        tracing::error!(error = ?e, "api server exited with error");
                        diag_for_api.record_bind("api", false, Some(format!("{e:#}")));
                    }
                }
            });

            // OTLP servers
            let pool = pool.clone();
            let control_for_otlp = control.clone();
            let diag_for_otlp = diagnostics.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = otlp::serve(pool, control_for_otlp, diag_for_otlp).await {
                    tracing::error!(error = ?e, "otlp server exited with error");
                }
            });

            // Tray icon + menu
            let open = MenuItem::with_id(app, "open", "Open Dashboard", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause Ingestion", true, None::<&str>)?;
            let resume = MenuItem::with_id(app, "resume", "Resume Ingestion", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit andon", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &pause, &resume, &sep, &quit])?;

            let tray_icon = app
                .default_window_icon()
                .cloned()
                .expect("default window icon must be configured in tauri.conf.json");
            let _tray = TrayIconBuilder::with_id("andon-tray")
                .icon(tray_icon)
                .tooltip("andon — Claude Code dashboard")
                .menu(&menu)
                .on_menu_event(handle_menu_event)
                .on_tray_icon_event(handle_tray_event)
                .build(app)?;

            // Start hidden — user opens via tray.
            if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
                let _ = w.hide();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Hide instead of quit
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn handle_menu_event(app: &tauri::AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        "open" => show_main_window(app),
        "pause" => {
            if let Some(state) = app.try_state::<AppState>() {
                state.control.set_paused(true);
                tracing::info!("ingestion paused via tray");
            }
        }
        "resume" => {
            if let Some(state) = app.try_state::<AppState>() {
                state.control.set_paused(false);
                tracing::info!("ingestion resumed via tray");
            }
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    }
}

fn handle_tray_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(tray.app_handle());
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.unminimize();
    }
}
