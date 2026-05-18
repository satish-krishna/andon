# v0.3.0 — Single-Instance, OTEL Forwarder, Session Reports — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the three v0.3.0 features described in `docs/superpowers/specs/2026-05-17-singleton-forwarder-reports-design.md`: single-instance lock, configurable OTEL forwarder, and per-session HTML reports.

**Architecture:** Three independent features ship as three phases against the existing Tauri 2 + axum + rusqlite stack. Phase A is a ~15-line Tauri plugin registration. Phase B introduces a `~/.andon/settings.json` store, a fire-and-forget `reqwest` forwarder hooked into the existing OTLP HTTP/gRPC entry points, and a settings UI card. Phase C extends the existing Claude Code settings patcher with a `SessionEnd` hook, adds a `/api/hooks/session-end` endpoint, and renders a self-contained HTML report via `minijinja` with inlined CSS + Chart.js.

**Tech Stack:** Rust + Tauri 2, axum 0.7, rusqlite, reqwest 0.12 (rustls), minijinja 2, tauri-plugin-single-instance 2, tauri-plugin-opener 2, Angular 17 standalone + SpartanNG.

**Scope note:** The three phases share no code and can land in any order. Each phase ends with green tests and a working binary; you can stop after any phase and ship.

---

## File Structure

### Phase A — Single instance
- Modify: `src-tauri/Cargo.toml` (add `tauri-plugin-single-instance`)
- Modify: `src-tauri/src/lib.rs` (register plugin, focus-callback)

### Phase B — OTEL forwarder
- Create: `src-tauri/src/settings.rs` — `AppSettings`, `SettingsStore`, load/save
- Create: `src-tauri/src/otlp/forwarder.rs` — `Forwarder` with `forward_metrics` / `forward_logs`
- Modify: `src-tauri/Cargo.toml` (add `reqwest`)
- Modify: `src-tauri/src/lib.rs` (build `SettingsStore` + `Forwarder`, thread into `ApiState` and OTLP `serve`)
- Modify: `src-tauri/src/otlp/mod.rs` — accept `Arc<Forwarder>`, pass to servers
- Modify: `src-tauri/src/otlp/grpc_server.rs` — call `forwarder.forward_metrics(&req)` / `forward_logs(&req)` after local ingest
- Modify: `src-tauri/src/otlp/http_server.rs` — same
- Modify: `src-tauri/src/api/mod.rs` — add `settings: Arc<SettingsStore>`, `forwarder: Arc<Forwarder>` to `ApiState`
- Modify: `src-tauri/src/api/routes.rs` — add 3 routes (GET settings, PUT forwarder, POST test)
- Create: `web/src/app/features/settings/forwarder-card.component.ts` — standalone component
- Modify: `web/src/app/features/settings/settings.component.html` — embed `<app-forwarder-card />`
- Modify: `web/src/app/core/api.service.ts` — `getSettings`, `saveForwarder`, `testForwarder`

### Phase C — Session reports
- Modify: `src-tauri/Cargo.toml` (add `minijinja`, `tauri-plugin-opener`)
- Modify: `src-tauri/src/integration.rs` — install/uninstall `SessionEnd` hook alongside `PostToolUse`
- Create: `src-tauri/src/reports/mod.rs` — `generate_report(pool, session_id)`
- Create: `src-tauri/src/reports/model.rs` — `ReportData` aggregate
- Create: `src-tauri/src/reports/render.rs` — minijinja env + render
- Create: `src-tauri/src/reports/assets.rs` — `include_str!` for `chart.umd.min.js` and inline CSS
- Create: `src-tauri/templates/session_report.html.j2`
- Create: `src-tauri/assets/chart.umd.min.js` (downloaded vendor file)
- Create: `src-tauri/assets/report.css`
- Modify: `src-tauri/src/lib.rs` — register modules
- Modify: `src-tauri/src/api/routes.rs` — add 4 routes (hook receiver, GET/POST report, POST open)
- Modify: `web/src/app/features/sessions/session-detail.component.ts` — add "Open report" / "Generate report" button
- Modify: `web/src/app/core/api.service.ts` — `getReport`, `generateReport`, `openReport`

---

# Phase A — Single-instance lock

### Task A1: Add `tauri-plugin-single-instance` dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add dependency**

In `src-tauri/Cargo.toml`, in the `[dependencies]` section, after `tauri-plugin-shell = "2"`, add:

```toml
tauri-plugin-single-instance = "2"
```

- [ ] **Step 2: Verify the crate resolves**

Run: `cargo check -p andon`
Expected: compile succeeds (no warnings about unused dep yet — we wire it up next).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "deps: add tauri-plugin-single-instance"
```

---

### Task A2: Register the plugin and focus existing window on second launch

**Files:**
- Modify: `src-tauri/src/lib.rs:88` (the `tauri::Builder::default()` chain)

- [ ] **Step 1: Add the plugin registration**

In `src-tauri/src/lib.rs`, find:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
```

Replace with:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
```

The single-instance plugin **must be registered first** so that secondary processes exit before any other setup (DB, OTLP listeners) runs.

- [ ] **Step 2: Move OTLP listener spawn so it does not race the lock**

The OTLP listeners are inside `.setup(...)`, which only runs in the primary process — that is correct. No change needed here, but verify by reading `src-tauri/src/lib.rs:91` and confirming the OTLP / API server spawn is inside `.setup(...)`.

- [ ] **Step 3: Build and smoke-test**

Run: `cargo build --release -p andon`
Expected: compiles clean.

- [ ] **Step 4: Manual verification**

Launch `andon.exe` once, then launch it a second time. Expected:
1. Second process exits with code 0 (no error window).
2. Primary window is shown and focused (un-minimized if minimized, raised if hidden in tray).
3. `~/.andon/log.txt` shows only one "andon starting" line per primary launch.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(v0.3.0): single-instance lock — focus existing window on second launch"
```

---

# Phase B — Configurable OTEL forwarder

### Task B1: Add `reqwest` dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add reqwest with rustls + http2**

In `src-tauri/Cargo.toml` `[dependencies]`, after the `axum` line, add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "http2"] }
```

`default-features = false` drops native-tls so we don't pull in OpenSSL on Windows.

- [ ] **Step 2: Verify**

Run: `cargo check -p andon`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "deps: add reqwest (rustls) for outbound OTEL forwarding"
```

---

### Task B2: Settings module — failing test

**Files:**
- Create: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod settings;` later — defer to Task B4)

- [ ] **Step 1: Write the module with a failing inline test**

Create `src-tauri/src/settings.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub version: u32,
    pub forwarder: ForwarderSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwarderSettings {
    pub enabled: bool,
    pub endpoint: String,
    pub timeout_ms: u64,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            forwarder: ForwarderSettings {
                enabled: false,
                endpoint: String::new(),
                timeout_ms: 2000,
                headers: Default::default(),
            },
        }
    }
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
    inner: Arc<RwLock<AppSettings>>,
}

impl SettingsStore {
    /// Load settings from `path`. If the file is missing, write defaults atomically.
    /// If the file is unreadable / unparseable, back it up and write defaults.
    pub fn load(path: PathBuf) -> Result<Self> {
        let settings = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(raw) => match serde_json::from_str::<AppSettings>(&raw) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = ?e, path = %path.display(),
                            "settings.json unparseable — backing up + writing defaults");
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let bak = path.with_extension(format!("json.corrupt-{ts}"));
                        let _ = std::fs::copy(&path, &bak);
                        let defaults = AppSettings::default();
                        write_atomic(&path, &serde_json::to_string_pretty(&defaults)?)?;
                        defaults
                    }
                },
                Err(e) => {
                    tracing::warn!(error = ?e, "settings.json unreadable; using defaults");
                    AppSettings::default()
                }
            }
        } else {
            let defaults = AppSettings::default();
            write_atomic(&path, &serde_json::to_string_pretty(&defaults)?)?;
            defaults
        };

        Ok(Self {
            path,
            inner: Arc::new(RwLock::new(settings)),
        })
    }

    pub fn snapshot(&self) -> AppSettings {
        self.inner.read().expect("settings lock").clone()
    }

    pub fn forwarder(&self) -> ForwarderSettings {
        self.inner.read().expect("settings lock").forwarder.clone()
    }

    /// Replace the forwarder block, persist atomically, return the new value.
    pub fn save_forwarder(&self, new: ForwarderSettings) -> Result<ForwarderSettings> {
        let mut w = self.inner.write().expect("settings lock");
        w.forwarder = new.clone();
        let serialized = serde_json::to_string_pretty(&*w)?;
        write_atomic(&self.path, &serialized)?;
        Ok(new)
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_defaults_when_missing() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p.clone()).unwrap();
        assert_eq!(store.snapshot(), AppSettings::default());
        assert!(p.exists());
    }

    #[test]
    fn save_forwarder_persists() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let store = SettingsStore::load(p.clone()).unwrap();

        let new_fwd = ForwarderSettings {
            enabled: true,
            endpoint: "https://otel.example.com".into(),
            timeout_ms: 1500,
            headers: [("Authorization".to_string(), "Bearer x".to_string())]
                .into_iter()
                .collect(),
        };
        store.save_forwarder(new_fwd.clone()).unwrap();

        // Re-load from disk and verify.
        let reloaded = SettingsStore::load(p).unwrap();
        assert_eq!(reloaded.forwarder(), new_fwd);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ this is not json").unwrap();
        let store = SettingsStore::load(p.clone()).unwrap();
        assert_eq!(store.snapshot(), AppSettings::default());
        // backup file exists
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
    }
}
```

- [ ] **Step 2: Add `tempfile` as a dev-dependency**

In `src-tauri/Cargo.toml`, add a `[dev-dependencies]` section if missing:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Register the module**

In `src-tauri/src/lib.rs:1`, add:

```rust
mod settings;
```

(Place it alphabetically among the existing `mod` lines.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p andon --lib settings::`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/settings.rs src-tauri/src/lib.rs
git commit -m "feat(settings): persistent AppSettings store at ~/.andon/settings.json"
```

---

### Task B3: Forwarder module

**Files:**
- Create: `src-tauri/src/otlp/forwarder.rs`
- Modify: `src-tauri/src/otlp/mod.rs`

- [ ] **Step 1: Write the forwarder**

Create `src-tauri/src/otlp/forwarder.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest,
    metrics::v1::ExportMetricsServiceRequest,
};
use prost::Message;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};

use crate::settings::{ForwarderSettings, SettingsStore};

pub struct Forwarder {
    settings: Arc<SettingsStore>,
    client: reqwest::Client,
}

impl Forwarder {
    pub fn new(settings: Arc<SettingsStore>) -> Self {
        let timeout_ms = settings.forwarder().timeout_ms;
        let client = build_client(timeout_ms);
        Self { settings, client }
    }

    pub fn forward_metrics(&self, req: &ExportMetricsServiceRequest) {
        let fwd = self.settings.forwarder();
        if !fwd.enabled || fwd.endpoint.is_empty() {
            return;
        }
        let body = req.encode_to_vec();
        let url = join_url(&fwd.endpoint, "/v1/metrics");
        self.spawn_post(url, body, fwd);
    }

    pub fn forward_logs(&self, req: &ExportLogsServiceRequest) {
        let fwd = self.settings.forwarder();
        if !fwd.enabled || fwd.endpoint.is_empty() {
            return;
        }
        let body = req.encode_to_vec();
        let url = join_url(&fwd.endpoint, "/v1/logs");
        self.spawn_post(url, body, fwd);
    }

    fn spawn_post(&self, url: String, body: Vec<u8>, fwd: ForwarderSettings) {
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/x-protobuf"));
            for (k, v) in &fwd.headers {
                if let (Ok(name), Ok(val)) = (
                    HeaderName::try_from(k.as_str()),
                    HeaderValue::from_str(v),
                ) {
                    headers.insert(name, val);
                }
            }
            match client.post(&url).headers(headers).body(body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(%url, status = resp.status().as_u16(), "forwarded ok");
                }
                Ok(resp) => {
                    tracing::warn!(%url, status = resp.status().as_u16(), "forwarder remote returned non-2xx");
                }
                Err(e) => {
                    tracing::warn!(%url, error = ?e, "forwarder request failed");
                }
            }
        });
    }
}

pub fn build_client(timeout_ms: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.max(100)))
        .connect_timeout(Duration::from_millis(timeout_ms.max(100)))
        .build()
        .expect("reqwest client build")
}

pub fn join_url(base: &str, suffix: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_strips_trailing_slash() {
        assert_eq!(join_url("https://x/", "/v1/metrics"), "https://x/v1/metrics");
        assert_eq!(join_url("https://x",  "/v1/metrics"), "https://x/v1/metrics");
    }
}
```

- [ ] **Step 2: Expose the module**

In `src-tauri/src/otlp/mod.rs:1`, after `pub mod ingestor;`, add:

```rust
pub mod forwarder;
```

- [ ] **Step 3: Build and run tests**

Run: `cargo test -p andon --lib otlp::forwarder::`
Expected: 1 test passes.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/otlp/forwarder.rs src-tauri/src/otlp/mod.rs
git commit -m "feat(otlp): forwarder module — fire-and-forget HTTP/protobuf re-emit"
```

---

### Task B4: Wire settings + forwarder into startup and OTLP servers

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/otlp/mod.rs`
- Modify: `src-tauri/src/otlp/grpc_server.rs`
- Modify: `src-tauri/src/otlp/http_server.rs`
- Modify: `src-tauri/src/api/mod.rs`
- Modify: `src-tauri/src/config.rs` (only if it doesn't already expose a settings path)

- [ ] **Step 1: Expose a settings path from `config::Paths`**

Read `src-tauri/src/config.rs`. If `Paths` already has `data_dir`, you can derive `settings_path = data_dir.join("settings.json")` at call site — do that, don't change `config.rs`.

- [ ] **Step 2: Build the store and forwarder in `run()`**

In `src-tauri/src/lib.rs`, after the `let pool = …` block and before `let control = …`, add:

```rust
    let settings_path = paths.data_dir.join("settings.json");
    let settings_store = match settings::SettingsStore::load(settings_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = ?e, "failed to load settings.json");
            std::process::exit(1);
        }
    };
    let forwarder = Arc::new(otlp::forwarder::Forwarder::new(settings_store.clone()));
```

- [ ] **Step 3: Add `settings` + `forwarder` to `ApiState`**

In `src-tauri/src/api/mod.rs`, change `ApiState`:

```rust
use crate::otlp::forwarder::Forwarder;
use crate::settings::SettingsStore;

#[derive(Clone)]
pub struct ApiState {
    pub pool: Arc<DbPool>,
    pub db_path: PathBuf,
    pub control: IngestionControl,
    pub integration: Arc<Mutex<IntegrationStatus>>,
    pub diagnostics: Diagnostics,
    pub settings: Arc<SettingsStore>,
    pub forwarder: Arc<Forwarder>,
}
```

- [ ] **Step 4: Populate the new fields in `lib.rs`**

In `src-tauri/src/lib.rs`, in the `api_state = api::ApiState { … }` block, add:

```rust
        settings: settings_store.clone(),
        forwarder: forwarder.clone(),
```

- [ ] **Step 5: Thread `forwarder` into `otlp::serve`**

In `src-tauri/src/otlp/mod.rs`, change the `serve` signature:

```rust
pub async fn serve(
    pool: Arc<DbPool>,
    control: IngestionControl,
    diagnostics: Diagnostics,
    forwarder: Arc<forwarder::Forwarder>,
) -> Result<()> {
    let ingestor = Arc::new(Ingestor::new(pool, control, diagnostics.clone()));

    let grpc = tokio::spawn(grpc_server::serve(
        ingestor.clone(),
        diagnostics.clone(),
        forwarder.clone(),
    ));
    let http = tokio::spawn(http_server::serve(
        ingestor.clone(),
        diagnostics.clone(),
        forwarder,
    ));

    let (g, h) = tokio::join!(grpc, http);
    if let Err(e) = g { tracing::error!(error = ?e, "grpc server task panicked"); }
    if let Err(e) = h { tracing::error!(error = ?e, "http server task panicked"); }
    Ok(())
}
```

And in `src-tauri/src/lib.rs`, update the spawn call:

```rust
            let forwarder_for_otlp = forwarder.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = otlp::serve(pool, control_for_otlp, diag_for_otlp, forwarder_for_otlp).await {
                    tracing::error!(error = ?e, "otlp server exited with error");
                }
            });
```

- [ ] **Step 6: Forward from the HTTP receiver**

In `src-tauri/src/otlp/http_server.rs`:

Change `HttpState`:

```rust
use std::sync::Arc;
use super::forwarder::Forwarder;

#[derive(Clone)]
struct HttpState {
    ingestor: Arc<Ingestor>,
    forwarder: Arc<Forwarder>,
}
```

Change the `serve` signature:

```rust
pub async fn serve(ingestor: Arc<Ingestor>, diagnostics: Diagnostics, forwarder: Arc<Forwarder>) {
    let addr: SocketAddr = HTTP_ADDR.parse().expect("hardcoded otlp http bind valid");
    let state = HttpState { ingestor, forwarder };
    // …unchanged below…
```

In the `metrics` handler, after the successful decode, **before** `spawn_blocking`, forward:

```rust
async fn metrics(State(state): State<HttpState>, body: Bytes) -> impl IntoResponse {
    match ExportMetricsServiceRequest::decode(body) {
        Ok(req) => {
            state.forwarder.forward_metrics(&req);
            let ingestor = state.ingestor.clone();
            let resource_metrics = req.resource_metrics;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = ingestor.ingest_metrics_v2(resource_metrics, "http") {
                    tracing::warn!(error = ?e, "metrics ingestion error (http)");
                }
            })
            .await
            .ok();
            (StatusCode::OK, "")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "failed to decode OTLP metrics protobuf");
            (StatusCode::OK, "")
        }
    }
}
```

Apply the same change to the `logs` handler with `state.forwarder.forward_logs(&req)`.

- [ ] **Step 7: Forward from the gRPC receiver**

Read `src-tauri/src/otlp/grpc_server.rs`. In each of the two `Service::export` implementations (metrics and logs), after a successful decode and before (or in parallel with) handing off to the ingestor, call `self.forwarder.forward_metrics(&request.get_ref())` or the logs equivalent. Add `forwarder: Arc<Forwarder>` to whatever struct backs the tonic services and to the `serve` function signature, mirroring Step 6.

- [ ] **Step 8: Build**

Run: `cargo build -p andon`
Expected: compiles. If lifetimes complain because the gRPC service holds `&request`, clone the request before spawn or call `forward_metrics(&request.get_ref())` before consuming it.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/otlp/mod.rs src-tauri/src/otlp/http_server.rs src-tauri/src/otlp/grpc_server.rs src-tauri/src/api/mod.rs
git commit -m "feat(otlp): wire forwarder into HTTP + gRPC ingestion paths"
```

---

### Task B5: API routes — GET /api/settings, PUT /api/settings/forwarder, POST /api/settings/forwarder/test

**Files:**
- Modify: `src-tauri/src/api/routes.rs`

- [ ] **Step 1: Add the three routes**

In `src-tauri/src/api/routes.rs`, in the `router(state)` function, after `.route("/api/health", get(health))`, add:

```rust
        .route("/api/settings", get(get_settings))
        .route("/api/settings/forwarder", axum::routing::put(put_forwarder))
        .route("/api/settings/forwarder/test", post(test_forwarder))
```

- [ ] **Step 2: Implement the handlers**

Append to `src-tauri/src/api/routes.rs` (above the `// ---------- error ----------` block):

```rust
// ---------- settings (forwarder) ----------

async fn get_settings(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.settings.snapshot()).unwrap_or_else(|_| json!({})))
}

#[derive(Deserialize)]
struct ForwarderPayload {
    enabled: bool,
    endpoint: String,
    timeout_ms: u64,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
}

fn validate_forwarder(p: &ForwarderPayload) -> Result<(), String> {
    if p.timeout_ms < 100 || p.timeout_ms > 30_000 {
        return Err("timeout_ms must be between 100 and 30000".into());
    }
    if p.enabled {
        if p.endpoint.is_empty() {
            return Err("endpoint is required when enabled=true".into());
        }
        if !(p.endpoint.starts_with("http://") || p.endpoint.starts_with("https://")) {
            return Err("endpoint must start with http:// or https://".into());
        }
    }
    for k in p.headers.keys() {
        if axum::http::HeaderName::from_bytes(k.as_bytes()).is_err() {
            return Err(format!("invalid header name: {k}"));
        }
    }
    Ok(())
}

async fn put_forwarder(
    State(state): State<ApiState>,
    Json(p): Json<ForwarderPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Err(msg) = validate_forwarder(&p) {
        return Err(ApiError { status: StatusCode::BAD_REQUEST, message: msg });
    }
    let new = crate::settings::ForwarderSettings {
        enabled: p.enabled,
        endpoint: p.endpoint,
        timeout_ms: p.timeout_ms,
        headers: p.headers,
    };
    let saved = state
        .settings
        .save_forwarder(new)
        .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("{e:#}") })?;
    Ok(Json(serde_json::to_value(saved).unwrap_or_else(|_| json!({}))))
}

async fn test_forwarder(
    State(_state): State<ApiState>,
    Json(p): Json<ForwarderPayload>,
) -> Json<serde_json::Value> {
    if let Err(msg) = validate_forwarder(&p) {
        return Json(json!({ "ok": false, "error": msg }));
    }
    let client = crate::otlp::forwarder::build_client(p.timeout_ms);
    let url = crate::otlp::forwarder::join_url(&p.endpoint, "/v1/metrics");
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use prost::Message;
    let body = ExportMetricsServiceRequest::default().encode_to_vec();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/x-protobuf"),
    );
    let mut req_headers = reqwest::header::HeaderMap::new();
    req_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/x-protobuf"),
    );
    for (k, v) in &p.headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::try_from(k.as_str()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            req_headers.insert(name, val);
        }
    }
    match client.post(&url).headers(req_headers).body(body).send().await {
        Ok(resp) => Json(json!({ "ok": resp.status().is_success(), "status": resp.status().as_u16() })),
        Err(e) => Json(json!({ "ok": false, "error": format!("{e}") })),
    }
}
```

Also make `ApiError.status` and `ApiError.message` accessible from these handlers — they are in the same file but `pub(super)`; if the fields are private, expose them via constructors. Add to `impl ApiError`:

```rust
impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: msg.into() }
    }
}
```

Then replace `ApiError { status: …, message: … }` literal constructions in the handlers above with `ApiError::bad_request(msg)` and a similar `ApiError::internal(format!("{e:#}"))` constructor.

- [ ] **Step 3: Build**

Run: `cargo build -p andon`
Expected: compiles.

- [ ] **Step 4: Manual smoke test**

1. Run `cargo tauri dev`.
2. `curl http://127.0.0.1:8765/api/settings` — should return `{"version":1,"forwarder":{...}}`.
3. `curl -X PUT http://127.0.0.1:8765/api/settings/forwarder -H 'content-type: application/json' -d '{"enabled":true,"endpoint":"http://127.0.0.1:9999","timeout_ms":1000,"headers":{}}'` — should return the saved block.
4. `curl http://127.0.0.1:8765/api/settings` — confirm persisted.
5. Confirm `~/.andon/settings.json` now contains the new forwarder block.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api/routes.rs
git commit -m "feat(api): /api/settings + /api/settings/forwarder GET|PUT|test"
```

---

### Task B6: Frontend Forwarder card

**Files:**
- Create: `web/src/app/features/settings/forwarder-card.component.ts`
- Modify: `web/src/app/features/settings/settings.component.html`
- Modify: `web/src/app/features/settings/settings.component.ts`
- Modify: `web/src/app/core/api.service.ts`

- [ ] **Step 1: Extend ApiService**

In `web/src/app/core/api.service.ts`, add interfaces and methods to the `ApiService` class:

```typescript
export interface ForwarderSettings {
  enabled: boolean;
  endpoint: string;
  timeout_ms: number;
  headers: Record<string, string>;
}

export interface AppSettings {
  version: number;
  forwarder: ForwarderSettings;
}
```

And inside the class:

```typescript
  getSettings(): Observable<AppSettings> {
    return this.http.get<AppSettings>(`${BASE}/api/settings`);
  }
  saveForwarder(f: ForwarderSettings): Observable<ForwarderSettings> {
    return this.http.put<ForwarderSettings>(`${BASE}/api/settings/forwarder`, f);
  }
  testForwarder(f: ForwarderSettings): Observable<{ ok: boolean; status?: number; error?: string }> {
    return this.http.post<any>(`${BASE}/api/settings/forwarder/test`, f);
  }
```

- [ ] **Step 2: Create the standalone component**

Create `web/src/app/features/settings/forwarder-card.component.ts`:

```typescript
import { CommonModule } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ApiService, ForwarderSettings } from '../../core/api.service';

interface HeaderRow { key: string; value: string; }

@Component({
  selector: 'app-forwarder-card',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
  <section class="rounded-lg border border-gray-200 bg-white p-4">
    <header class="mb-3 flex items-center justify-between">
      <h3 class="text-base font-semibold">OTEL forwarder</h3>
      <label class="inline-flex items-center gap-2 text-sm">
        <input type="checkbox" [(ngModel)]="enabled" (ngModelChange)="dirty.set(true)" />
        <span>{{ enabled() ? 'Enabled' : 'Disabled' }}</span>
      </label>
    </header>
    <p class="mb-3 text-sm text-gray-600">
      Re-emit every metric &amp; log Andon receives to a second OTLP endpoint (HTTP/protobuf).
      Defaults: disabled.
    </p>

    <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
      <label class="text-sm">
        <span class="block text-gray-700">Endpoint base URL</span>
        <input class="mt-1 w-full rounded border px-2 py-1 disabled:bg-gray-100"
               [(ngModel)]="endpoint" [disabled]="!enabled()"
               (ngModelChange)="dirty.set(true)"
               placeholder="https://otel.example.com" />
      </label>
      <label class="text-sm">
        <span class="block text-gray-700">Timeout (ms)</span>
        <input class="mt-1 w-full rounded border px-2 py-1 disabled:bg-gray-100"
               type="number" min="100" max="30000"
               [(ngModel)]="timeoutMs" [disabled]="!enabled()"
               (ngModelChange)="dirty.set(true)" />
      </label>
    </div>

    <div class="mt-4">
      <div class="mb-1 text-sm font-medium text-gray-700">Headers</div>
      <div class="space-y-2">
        <div *ngFor="let row of headers(); let i = index" class="flex gap-2">
          <input class="flex-1 rounded border px-2 py-1 disabled:bg-gray-100"
                 placeholder="Header name"  [disabled]="!enabled()"
                 [(ngModel)]="row.key"   (ngModelChange)="dirty.set(true)" />
          <input class="flex-1 rounded border px-2 py-1 disabled:bg-gray-100"
                 placeholder="Header value" [disabled]="!enabled()"
                 [(ngModel)]="row.value" (ngModelChange)="dirty.set(true)" />
          <button class="rounded border px-2 py-1 text-sm"
                  [disabled]="!enabled()" (click)="removeRow(i)">Remove</button>
        </div>
        <button class="rounded border px-2 py-1 text-sm"
                [disabled]="!enabled()" (click)="addRow()">+ Add header</button>
      </div>
    </div>

    <div class="mt-4 flex items-center gap-2">
      <button class="rounded bg-blue-600 px-3 py-1.5 text-sm text-white disabled:opacity-50"
              [disabled]="!dirty()" (click)="save()">Save</button>
      <button class="rounded border px-3 py-1.5 text-sm"
              [disabled]="!enabled()" (click)="test()">Test connection</button>
      <span class="text-sm" [class.text-green-600]="ok()" [class.text-red-600]="err()">{{ msg() }}</span>
    </div>
  </section>
  `,
})
export class ForwarderCardComponent implements OnInit {
  private api = inject(ApiService);

  enabled  = signal(false);
  endpoint = signal('');
  timeoutMs = signal(2000);
  headers  = signal<HeaderRow[]>([]);
  dirty    = signal(false);
  msg      = signal('');
  ok       = signal(false);
  err      = signal(false);

  ngOnInit() {
    this.api.getSettings().subscribe((s) => {
      this.enabled.set(s.forwarder.enabled);
      this.endpoint.set(s.forwarder.endpoint);
      this.timeoutMs.set(s.forwarder.timeout_ms);
      this.headers.set(Object.entries(s.forwarder.headers).map(([key, value]) => ({ key, value })));
      this.dirty.set(false);
    });
  }

  addRow() { this.headers.set([...this.headers(), { key: '', value: '' }]); this.dirty.set(true); }
  removeRow(i: number) { const next = [...this.headers()]; next.splice(i, 1); this.headers.set(next); this.dirty.set(true); }

  private toPayload(): ForwarderSettings {
    const headers: Record<string, string> = {};
    for (const r of this.headers()) if (r.key.trim()) headers[r.key.trim()] = r.value;
    return {
      enabled: this.enabled(),
      endpoint: this.endpoint().trim(),
      timeout_ms: Number(this.timeoutMs()),
      headers,
    };
  }

  save() {
    this.api.saveForwarder(this.toPayload()).subscribe({
      next: () => { this.flash('Saved', true); this.dirty.set(false); },
      error: (e) => this.flash(`Error: ${e?.error?.error ?? e.message}`, false),
    });
  }

  test() {
    this.api.testForwarder(this.toPayload()).subscribe((r) => {
      if (r.ok) this.flash(`OK (HTTP ${r.status})`, true);
      else this.flash(`Failed: ${r.error ?? 'status ' + r.status}`, false);
    });
  }

  private flash(text: string, ok: boolean) {
    this.msg.set(text); this.ok.set(ok); this.err.set(!ok);
    setTimeout(() => { this.msg.set(''); this.ok.set(false); this.err.set(false); }, 4000);
  }
}
```

- [ ] **Step 3: Embed in settings page**

In `web/src/app/features/settings/settings.component.ts`, add the import:

```typescript
import { ForwarderCardComponent } from './forwarder-card.component';
```

And add `ForwarderCardComponent` to the component's `imports: [...]` array.

In `web/src/app/features/settings/settings.component.html`, insert `<app-forwarder-card class="block mt-6" />` at a sensible location — directly under the existing "Integration" or "Autostart" card.

- [ ] **Step 4: Build the SPA**

Run: `cd web && npm run build`
Expected: build succeeds. If TypeScript complains about missing `FormsModule`, confirm `@angular/forms` is in `web/package.json` (it is, in standard Angular CLI projects).

- [ ] **Step 5: Manual UI smoke test**

`cargo tauri dev` → open Settings tab → confirm Forwarder card renders, toggle works, save persists across reload (re-open window → values restored from `~/.andon/settings.json`).

- [ ] **Step 6: Commit**

```bash
git add web/src/app/features/settings/forwarder-card.component.ts web/src/app/features/settings/settings.component.ts web/src/app/features/settings/settings.component.html web/src/app/core/api.service.ts
git commit -m "feat(ui): forwarder card on Settings page"
```

---

### Task B7: End-to-end forwarder verification

- [ ] **Step 1: Start a local OTel collector on an alternate port**

The simplest verifier: run `nc -l 4327` or a tiny Python `http.server` and observe a POST. For real validation use the OTel Collector docker image bound to `:4327/v1/metrics`. If neither is convenient, point the forwarder at `http://127.0.0.1:9999` and confirm with `tcpdump -i lo` (Linux) / `Wireshark` (Windows) that a POST is attempted.

- [ ] **Step 2: Enable forwarder, run Claude Code session**

Use the Settings UI: enable, endpoint = `http://127.0.0.1:4327`, save. Run any Claude Code session. Confirm metrics arrive at the second collector within 2 seconds.

- [ ] **Step 3: Disable forwarder, repeat**

Confirm no traffic to the second collector. Local ingestion (rows in SQLite) still works.

- [ ] **Step 4: Failure mode**

Set endpoint to a bogus port. Confirm Claude Code sessions remain fast (no client-side retries / hangs), and `log.txt` contains `forwarder request failed` warnings.

- [ ] **Step 5: Tag the phase**

```bash
git tag -a phase-b-forwarder -m "v0.3.0 Phase B — forwarder complete"
```

---

# Phase C — Per-session HTML reports

### Task C1: Add `minijinja` + `tauri-plugin-opener` deps; download Chart.js asset

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/assets/chart.umd.min.js`
- Create: `src-tauri/assets/report.css`

- [ ] **Step 1: Cargo deps**

In `src-tauri/Cargo.toml` `[dependencies]`:

```toml
minijinja = "2"
tauri-plugin-opener = "2"
```

- [ ] **Step 2: Vendor Chart.js (MIT, ~70KB minified)**

Download from `https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js` (latest 4.x). Place at `src-tauri/assets/chart.umd.min.js`. Add a brief LICENSE note next to it (`src-tauri/assets/CHARTJS_LICENSE.txt` — copy the MIT license header from Chart.js's repo).

- [ ] **Step 3: Create report.css**

Create `src-tauri/assets/report.css` with hand-rolled CSS for the report (no Tailwind). Approx 100 lines, simple layout. Minimum acceptable:

```css
:root { color-scheme: light dark; --fg: #1f2937; --muted: #6b7280; --bg: #fff; --line: #e5e7eb; --accent: #2563eb; }
html, body { margin: 0; padding: 0; background: var(--bg); color: var(--fg); font: 14px/1.4 -apple-system, "Segoe UI", system-ui, sans-serif; }
.wrap { max-width: 1100px; margin: 0 auto; padding: 24px; }
h1 { font-size: 22px; margin: 0 0 4px; }
h2 { font-size: 16px; margin: 24px 0 8px; }
.muted { color: var(--muted); }
.kpis { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; margin: 16px 0; }
.kpi { border: 1px solid var(--line); border-radius: 8px; padding: 12px; }
.kpi .label { color: var(--muted); font-size: 12px; }
.kpi .value { font-size: 20px; font-weight: 600; margin-top: 4px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--line); }
th { font-weight: 600; color: var(--muted); }
canvas { max-width: 100%; }
.chart-row { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
@media (max-width: 720px) { .kpis { grid-template-columns: repeat(2, 1fr); } .chart-row { grid-template-columns: 1fr; } }
```

- [ ] **Step 4: Build**

Run: `cargo check -p andon`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/assets/
git commit -m "deps: minijinja + tauri-plugin-opener; vendor Chart.js + report.css"
```

---

### Task C2: Reports module skeleton + template

**Files:**
- Create: `src-tauri/src/reports/mod.rs`
- Create: `src-tauri/src/reports/model.rs`
- Create: `src-tauri/src/reports/render.rs`
- Create: `src-tauri/src/reports/assets.rs`
- Create: `src-tauri/templates/session_report.html.j2`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Module wiring**

Add to `src-tauri/src/lib.rs:1`:

```rust
mod reports;
```

Create `src-tauri/src/reports/mod.rs`:

```rust
pub mod assets;
pub mod model;
pub mod render;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::DbPool;

/// Sanitize a session id for use as a filename. Defensive — Claude Code uses UUIDs.
pub fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

pub fn report_path(reports_dir: &std::path::Path, session_id: &str) -> PathBuf {
    reports_dir.join(format!("{}.html", sanitize_id(session_id)))
}

pub fn generate_report(
    pool: Arc<DbPool>,
    reports_dir: &std::path::Path,
    session_id: &str,
) -> Result<PathBuf> {
    let data = model::ReportData::load(&pool, session_id)
        .context("load report data")?;
    let html = render::render(&data).context("render template")?;
    std::fs::create_dir_all(reports_dir)
        .with_context(|| format!("mkdir {}", reports_dir.display()))?;
    let path = report_path(reports_dir, session_id);
    let tmp = path.with_extension("html.tmp");
    std::fs::write(&tmp, html).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize_id("abc-123_xyz"), "abc-123_xyz");
        assert_eq!(sanitize_id("../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_id("a/b\\c"), "abc");
    }
}
```

- [ ] **Step 2: Assets module**

Create `src-tauri/src/reports/assets.rs`:

```rust
pub const CHART_JS: &str = include_str!("../../assets/chart.umd.min.js");
pub const REPORT_CSS: &str = include_str!("../../assets/report.css");
```

- [ ] **Step 3: ReportData model**

Create `src-tauri/src/reports/model.rs`:

```rust
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

use crate::db::DbPool;

#[derive(Serialize)]
pub struct ReportData {
    pub session_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: f64,
    pub service_version: Option<String>,
    pub host_arch: Option<String>,
    pub os_type: Option<String>,
    pub terminal_type: Option<String>,

    pub cost_usd: f64,
    pub tokens: Vec<KV>,
    pub accept_rate: f64,
    pub active_user_seconds: f64,
    pub active_cli_seconds: f64,

    pub cost_by_model: Vec<KVFloat>,
    pub tokens_by_type: Vec<KVFloat>,

    pub files: Vec<FileRow>,
    pub decisions: Vec<DecisionRow>,
}

#[derive(Serialize)]
pub struct KV { pub key: String, pub value: i64 }
#[derive(Serialize)]
pub struct KVFloat { pub key: String, pub value: f64 }

#[derive(Serialize)]
pub struct FileRow {
    pub file_path: String,
    pub added: i64,
    pub removed: i64,
    pub accept_rate: f64,
}

#[derive(Serialize)]
pub struct DecisionRow {
    pub timestamp: i64,
    pub tool_name: String,
    pub decision: String,
    pub language: Option<String>,
    pub file_path: Option<String>,
}

fn rate(a: i64, r: i64, x: i64) -> f64 {
    let d = a + r + x;
    if d == 0 { 0.0 } else { ((a as f64 / d as f64) * 10000.0).round() / 10000.0 }
}

impl ReportData {
    pub fn load(pool: &DbPool, sid: &str) -> Result<Self> {
        let conn = pool.get()?;

        // Session row — may not exist; insert stub if missing.
        let (started_at, ended_at, sv, ha, ot, tt) = conn
            .query_row(
                "SELECT started_at, ended_at, service_version, host_arch, os_type, terminal_type
                 FROM sessions WHERE session_id = ?1",
                params![sid],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                )),
            )
            .unwrap_or((0, None, None, None, None, None));

        let cost_usd: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_entries WHERE session_id = ?1",
            params![sid], |r| r.get(0)).unwrap_or(0.0);

        let mut tokens = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT token_type, COALESCE(SUM(count), 0) FROM token_usage
             WHERE session_id = ?1 GROUP BY token_type")?;
        for row in stmt.query_map(params![sid], |r| Ok(KV { key: r.get(0)?, value: r.get(1)? }))?.flatten() {
            tokens.push(row);
        }

        let (a, r, x): (i64, i64, i64) = conn.query_row(
            "SELECT
                SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision='reject' THEN 1 ELSE 0 END),
                SUM(CASE WHEN decision='abort'  THEN 1 ELSE 0 END)
             FROM tool_decisions WHERE session_id = ?1",
            params![sid],
            |r| Ok((r.get::<_,i64>(0).unwrap_or(0), r.get::<_,i64>(1).unwrap_or(0), r.get::<_,i64>(2).unwrap_or(0))),
        ).unwrap_or((0,0,0));
        let accept_rate = rate(a, r, x);

        let active_user: f64 = conn.query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE session_id = ?1 AND kind='user'",
            params![sid], |r| r.get(0)).unwrap_or(0.0);
        let active_cli: f64 = conn.query_row(
            "SELECT COALESCE(SUM(seconds), 0) FROM active_time WHERE session_id = ?1 AND kind='cli'",
            params![sid], |r| r.get(0)).unwrap_or(0.0);

        let mut cost_by_model = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT model, SUM(cost_usd) FROM cost_entries
             WHERE session_id = ?1 GROUP BY model ORDER BY 2 DESC")?;
        for row in stmt.query_map(params![sid], |r| Ok(KVFloat { key: r.get(0)?, value: r.get(1).unwrap_or(0.0) }))?.flatten() {
            cost_by_model.push(row);
        }

        let tokens_by_type = tokens.iter().map(|kv| KVFloat { key: kv.key.clone(), value: kv.value as f64 }).collect();

        let mut files = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(file_path, '?'), SUM(lines_added), SUM(lines_removed),
                    COALESCE((
                        SELECT
                            CAST(SUM(CASE WHEN decision='accept' THEN 1 ELSE 0 END) AS REAL)
                            / NULLIF(COUNT(*), 0)
                        FROM tool_decisions td
                        WHERE td.session_id = fc.session_id AND td.file_path = fc.file_path
                    ), 0)
             FROM file_changes fc WHERE session_id = ?1 GROUP BY file_path ORDER BY 2+3 DESC")?;
        for row in stmt.query_map(params![sid], |r| Ok(FileRow {
            file_path: r.get(0)?,
            added: r.get(1).unwrap_or(0),
            removed: r.get(2).unwrap_or(0),
            accept_rate: ((r.get::<_, f64>(3).unwrap_or(0.0) * 10000.0).round()) / 10000.0,
        }))?.flatten() {
            files.push(row);
        }

        let mut decisions = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT timestamp, tool_name, decision, language, file_path
             FROM tool_decisions WHERE session_id = ?1 ORDER BY timestamp ASC")?;
        for row in stmt.query_map(params![sid], |r| Ok(DecisionRow {
            timestamp: r.get(0)?,
            tool_name: r.get(1)?,
            decision: r.get(2)?,
            language: r.get(3).ok(),
            file_path: r.get(4).ok(),
        }))?.flatten() {
            decisions.push(row);
        }

        let duration_seconds = match ended_at {
            Some(e) if e > started_at => ((e - started_at) as f64) / 1000.0,
            _ => active_user + active_cli,
        };

        Ok(ReportData {
            session_id: sid.to_string(),
            started_at, ended_at,
            duration_seconds,
            service_version: sv, host_arch: ha, os_type: ot, terminal_type: tt,
            cost_usd: ((cost_usd * 10000.0).round()) / 10000.0,
            tokens, accept_rate,
            active_user_seconds: active_user,
            active_cli_seconds: active_cli,
            cost_by_model, tokens_by_type,
            files, decisions,
        })
    }
}
```

- [ ] **Step 4: Render**

Create `src-tauri/src/reports/render.rs`:

```rust
use anyhow::Result;
use minijinja::{Environment, context};

use super::assets;
use super::model::ReportData;

const TEMPLATE_SRC: &str = include_str!("../../templates/session_report.html.j2");

pub fn render(data: &ReportData) -> Result<String> {
    let mut env = Environment::new();
    env.add_template("report", TEMPLATE_SRC)?;
    let tpl = env.get_template("report")?;
    let report_data_json = serde_json::to_string(data)?;
    let html = tpl.render(context! {
        data            => data,
        report_data_json => report_data_json,
        chart_js        => assets::CHART_JS,
        css             => assets::REPORT_CSS,
    })?;
    Ok(html)
}
```

- [ ] **Step 5: Template**

Create directory: `src-tauri/templates/`. Then create `src-tauri/templates/session_report.html.j2`:

```jinja
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Andon session report — {{ data.session_id }}</title>
<style>{{ css|safe }}</style>
</head>
<body>
<div class="wrap">
  <h1>Session {{ data.session_id }}</h1>
  <div class="muted">
    {{ data.service_version or "unknown version" }} ·
    {{ data.os_type or "?" }}/{{ data.host_arch or "?" }} ·
    {{ data.terminal_type or "?" }}
  </div>

  <div class="kpis">
    <div class="kpi"><div class="label">Total cost</div><div class="value">${{ "%.4f"|format(data.cost_usd) }}</div></div>
    <div class="kpi"><div class="label">Accept rate</div><div class="value">{{ "%.1f"|format(data.accept_rate * 100) }}%</div></div>
    <div class="kpi"><div class="label">Duration</div><div class="value">{{ "%.0f"|format(data.duration_seconds) }}s</div></div>
    <div class="kpi"><div class="label">Active (user+cli)</div><div class="value">{{ "%.0f"|format(data.active_user_seconds + data.active_cli_seconds) }}s</div></div>
  </div>

  <h2>Charts</h2>
  <div class="chart-row">
    <div><canvas id="costChart" height="220"></canvas></div>
    <div><canvas id="tokenChart" height="220"></canvas></div>
  </div>

  <h2>Files</h2>
  <table>
    <thead><tr><th>Path</th><th>+lines</th><th>-lines</th><th>Accept rate</th></tr></thead>
    <tbody>
    {% for f in data.files %}
      <tr><td>{{ f.file_path }}</td><td>{{ f.added }}</td><td>{{ f.removed }}</td><td>{{ "%.1f"|format(f.accept_rate * 100) }}%</td></tr>
    {% endfor %}
    </tbody>
  </table>

  <h2>Tool decisions</h2>
  <table>
    <thead><tr><th>Time (ms)</th><th>Tool</th><th>Decision</th><th>Lang</th><th>File</th></tr></thead>
    <tbody>
    {% for d in data.decisions %}
      <tr>
        <td>{{ d.timestamp }}</td>
        <td>{{ d.tool_name }}</td>
        <td>{{ d.decision }}</td>
        <td>{{ d.language or "" }}</td>
        <td>{{ d.file_path or "" }}</td>
      </tr>
    {% endfor %}
    </tbody>
  </table>
</div>

<script type="application/json" id="report-data">{{ report_data_json|safe }}</script>
<script>{{ chart_js|safe }}</script>
<script>
(function () {
  const data = JSON.parse(document.getElementById('report-data').textContent);

  const cost = data.cost_by_model || [];
  new Chart(document.getElementById('costChart'), {
    type: 'bar',
    data: { labels: cost.map(c => c.key), datasets: [{ label: 'USD', data: cost.map(c => c.value), backgroundColor: '#2563eb' }] },
    options: { plugins: { title: { display: true, text: 'Cost by model (USD)' } } }
  });

  const tok = data.tokens_by_type || [];
  new Chart(document.getElementById('tokenChart'), {
    type: 'bar',
    data: { labels: tok.map(t => t.key), datasets: [{ label: 'tokens', data: tok.map(t => t.value), backgroundColor: '#16a34a' }] },
    options: { plugins: { title: { display: true, text: 'Tokens by type' } } }
  });
})();
</script>
</body>
</html>
```

- [ ] **Step 6: Build**

Run: `cargo build -p andon`
Expected: compiles. The `include_str!` paths must resolve — confirm `src-tauri/templates/session_report.html.j2`, `src-tauri/assets/chart.umd.min.js`, and `src-tauri/assets/report.css` all exist.

- [ ] **Step 7: Run module test**

Run: `cargo test -p andon --lib reports::tests::sanitize_strips_unsafe_chars`
Expected: pass.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/reports/ src-tauri/templates/ src-tauri/src/lib.rs
git commit -m "feat(reports): minijinja-based HTML session report renderer"
```

---

### Task C3: Extend integration patcher with SessionEnd hook

**Files:**
- Modify: `src-tauri/src/integration.rs`

- [ ] **Step 1: Add constants and helpers**

In `src-tauri/src/integration.rs`, add near the existing `HOOK_*` constants:

```rust
const SESSION_END_COMMAND: &str =
    "curl -s -X POST http://127.0.0.1:8765/api/hooks/session-end -H \"Content-Type: application/json\" --data-binary @-";
const SESSION_END_MARKER: &str = "/api/hooks/session-end";
```

- [ ] **Step 2: Generalize detection + install + remove**

Find `has_our_hook`, `install_hook`, `remove_hook`. Add a parallel pair scoped to `SessionEnd`:

```rust
fn has_session_end_hook(value: &Value) -> bool {
    let Some(arr) = value
        .get("hooks").and_then(|h| h.get("SessionEnd")).and_then(|a| a.as_array())
    else { return false };
    for entry in arr {
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            for h in hooks {
                if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                    if cmd.contains(SESSION_END_MARKER) { return true; }
                }
            }
        }
    }
    false
}

fn install_session_end_hook(value: &mut Value) {
    if has_session_end_hook(value) { return; }
    let Some(obj) = value.as_object_mut() else { return };
    let hooks = obj.entry("hooks".to_string()).or_insert_with(|| json!({}));
    if !hooks.is_object() { *hooks = json!({}); }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let arr = hooks_obj
        .entry("SessionEnd".to_string())
        .or_insert_with(|| json!([]));
    if !arr.is_array() { *arr = json!([]); }
    arr.as_array_mut().unwrap().push(json!({
        "hooks": [{ "type": "command", "command": SESSION_END_COMMAND }]
    }));
}

fn remove_session_end_hook(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else { return false };
    let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) else { return false };
    let Some(arr) = hooks.get_mut("SessionEnd").and_then(|a| a.as_array_mut()) else { return false };
    let before = arr.len();
    arr.retain(|entry| {
        let has = entry.get("hooks").and_then(|h| h.as_array()).map(|inner| {
            inner.iter().any(|h| h.get("command").and_then(|c| c.as_str())
                .map(|s| s.contains(SESSION_END_MARKER)).unwrap_or(false))
        }).unwrap_or(false);
        !has
    });
    let removed = arr.len() < before;
    if arr.is_empty() { hooks.remove("SessionEnd"); }
    if hooks.is_empty() { obj.remove("hooks"); }
    removed
}
```

- [ ] **Step 3: Wire into the patch + unpatch flow**

In `try_ensure`, change the `hook_installed` check from a single value to:

```rust
    let hook_installed = has_our_hook(&existing) && has_session_end_hook(&existing);
```

And in the patch section, after `install_hook(&mut merged_val);`, add:

```rust
    install_session_end_hook(&mut merged_val);
```

In `unpatch_claude_settings`, after the existing `if remove_hook(&mut value) { removed_any = true; }`, add:

```rust
    if remove_session_end_hook(&mut value) { removed_any = true; }
```

- [ ] **Step 4: Build**

Run: `cargo build -p andon`
Expected: compiles.

- [ ] **Step 5: Smoke test patcher**

1. Back up `~/.claude/settings.json` manually.
2. Launch andon; verify `settings.json` now contains a `SessionEnd` entry pointing at `/api/hooks/session-end`.
3. Re-launch — confirm no duplicate is added (idempotency).
4. Hit `POST /api/integration/unpatch`; confirm both `PostToolUse` *and* `SessionEnd` entries are removed.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/integration.rs
git commit -m "feat(integration): install/uninstall SessionEnd hook alongside PostToolUse"
```

---

### Task C4: API routes — hook receiver, GET/POST report, POST open

**Files:**
- Modify: `src-tauri/src/api/routes.rs`
- Modify: `src-tauri/src/api/mod.rs` (add `reports_dir` to `ApiState`)
- Modify: `src-tauri/src/lib.rs` (populate `reports_dir`; register `tauri-plugin-opener`)

- [ ] **Step 1: Add `reports_dir` to ApiState**

In `src-tauri/src/api/mod.rs`, add to `ApiState`:

```rust
    pub reports_dir: PathBuf,
```

- [ ] **Step 2: Populate it in lib.rs**

In `src-tauri/src/lib.rs`, before constructing `ApiState`, add:

```rust
    let reports_dir = paths.data_dir.join("reports");
```

And add `reports_dir: reports_dir.clone(),` to the `ApiState { … }` literal.

Also register the opener plugin in the Tauri builder, adjacent to the other `.plugin(...)` calls:

```rust
        .plugin(tauri_plugin_opener::init())
```

- [ ] **Step 3: Add the four routes**

In `src-tauri/src/api/routes.rs`, in the `router(state)` function, after the existing `.route("/api/hooks/tool-use", post(hook_tool_use))`, add:

```rust
        .route("/api/hooks/session-end", post(hook_session_end))
        .route("/api/sessions/:id/report", get(get_report).post(generate_report_handler))
        .route("/api/sessions/:id/report/open", post(open_report))
        .route("/api/sessions/reports/index", get(reports_index))
```

- [ ] **Step 4: Implement the handlers**

Append to `src-tauri/src/api/routes.rs` (above the existing `// ============================================================================` v2 marker, so it stays grouped):

```rust
// ---------- session-end hook + reports ----------

#[derive(Deserialize)]
struct SessionEndPayload {
    session_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

async fn hook_session_end(
    State(state): State<ApiState>,
    Json(p): Json<SessionEndPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sid = p.session_id.unwrap_or_default();
    if sid.is_empty() {
        return Err(ApiError { status: StatusCode::BAD_REQUEST, message: "session_id required".into() });
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // Update ended_at if null; insert stub session row if missing.
    {
        let conn = state.pool.get().map_err(ApiError::pool)?;
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id = ?1",
            params![sid], |r| r.get(0)).unwrap_or(0);
        if exists == 0 {
            let _ = conn.execute(
                "INSERT INTO sessions (session_id, started_at, ended_at) VALUES (?1, ?2, ?2)",
                params![sid, now]);
        } else {
            let _ = conn.execute(
                "UPDATE sessions SET ended_at = ?2 WHERE session_id = ?1 AND ended_at IS NULL",
                params![sid, now]);
        }
    }

    let pool = state.pool.clone();
    let reports_dir = state.reports_dir.clone();
    let sid_for_task = sid.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            crate::reports::generate_report(pool, &reports_dir, &sid_for_task)
        }).await;
        match result {
            Ok(Ok(path)) => tracing::info!(path = %path.display(), "report generated"),
            Ok(Err(e))   => tracing::error!(error = ?e, "report render failed"),
            Err(e)       => tracing::error!(error = ?e, "report task panicked"),
        }
    });

    Ok(Json(json!({ "ok": true, "reason": p.reason })))
}

async fn get_report(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let path = crate::reports::report_path(&state.reports_dir, &id);
    let exists = path.exists();
    let generated_at = if exists {
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
    } else { None };
    Json(json!({
        "exists": exists,
        "path": path.display().to_string(),
        "generated_at": generated_at,
    }))
}

async fn generate_report_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state.pool.clone();
    let dir = state.reports_dir.clone();
    let sid = id.clone();
    let path = tokio::task::spawn_blocking(move || crate::reports::generate_report(pool, &dir, &sid))
        .await
        .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("join: {e}") })?
        .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("{e:#}") })?;
    let generated_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(Json(json!({
        "exists": true,
        "path": path.display().to_string(),
        "generated_at": generated_at,
    })))
}

async fn open_report(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let path = crate::reports::report_path(&state.reports_dir, &id);
    if !path.exists() {
        return Json(json!({ "ok": false, "error": "report not found" }));
    }
    // Use std::process::Command for cross-platform open. On Windows: explorer "<path>".
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.display().to_string()])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&path).spawn();
    match result {
        Ok(_) => Json(json!({ "ok": true, "path": path.display().to_string() })),
        Err(e) => Json(json!({ "ok": false, "error": format!("{e}") })),
    }
}

async fn reports_index(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let mut ids: Vec<String> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(&state.reports_dir) {
        for entry in dir.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stem) = name.strip_suffix(".html") {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    Json(json!({ "session_ids": ids }))
}
```

(`tauri-plugin-opener` is available too, but using `std::process::Command` avoids needing to thread an `AppHandle` into this axum handler — the spec's "system default browser" requirement is met either way. Keep `tauri-plugin-opener` registered for the future "Open data folder" rewrite; nothing currently calls into it.)

- [ ] **Step 5: Build**

Run: `cargo build -p andon`
Expected: compiles. If `Path` is ambiguous between `axum::extract::Path` and `std::path::Path`, fully-qualify the std one: `std::path::Path`.

- [ ] **Step 6: Smoke test**

1. `cargo tauri dev`.
2. From another shell:
   ```bash
   curl -X POST http://127.0.0.1:8765/api/hooks/session-end \
     -H 'content-type: application/json' \
     -d '{"session_id":"test-uuid-1","reason":"exit"}'
   ```
3. Within 5 seconds, `~/.andon/reports/test-uuid-1.html` should exist.
4. Open it in a browser with no network — charts render, layout intact.
5. `curl http://127.0.0.1:8765/api/sessions/test-uuid-1/report` returns `{"exists":true,…}`.
6. `curl -X POST http://127.0.0.1:8765/api/sessions/test-uuid-1/report/open` opens the file.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/api/routes.rs src-tauri/src/api/mod.rs src-tauri/src/lib.rs
git commit -m "feat(api): session-end hook + report generate/open endpoints"
```

---

### Task C5: Frontend — Open report button on session detail

**Files:**
- Modify: `web/src/app/core/api.service.ts`
- Modify: `web/src/app/features/sessions/session-detail.component.ts`

- [ ] **Step 1: Add report methods to ApiService**

In `web/src/app/core/api.service.ts`, add to the class:

```typescript
  getReport(id: string): Observable<{ exists: boolean; path: string; generated_at: number | null }> {
    return this.http.get<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report`);
  }
  generateReport(id: string): Observable<{ exists: boolean; path: string; generated_at: number | null }> {
    return this.http.post<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report`, {});
  }
  openReport(id: string): Observable<{ ok: boolean; path?: string; error?: string }> {
    return this.http.post<any>(`${BASE}/api/sessions/${encodeURIComponent(id)}/report/open`, {});
  }
  reportsIndex(): Observable<{ session_ids: string[] }> {
    return this.http.get<any>(`${BASE}/api/sessions/reports/index`);
  }
```

- [ ] **Step 2: Wire button into session-detail**

Read `web/src/app/features/sessions/session-detail.component.ts` first to find the header markup. Add a state signal:

```typescript
  reportExists = signal(false);
  reportBusy   = signal(false);
```

In the existing init/load block (where `api.session(id)` is called), also call:

```typescript
    this.api.getReport(id).subscribe((r) => this.reportExists.set(r.exists));
```

Add a method:

```typescript
  openOrGenerateReport(id: string) {
    this.reportBusy.set(true);
    const action$ = this.reportExists()
      ? this.api.openReport(id)
      : this.api.generateReport(id).pipe(switchMap(() => { this.reportExists.set(true); return this.api.openReport(id); }));
    action$.subscribe({
      next: () => this.reportBusy.set(false),
      error: () => this.reportBusy.set(false),
    });
  }
```

Add the necessary imports: `import { switchMap } from 'rxjs';`

In the template (inline string or `templateUrl`), in the page header next to the session id, add:

```html
<button class="rounded border px-3 py-1.5 text-sm"
        [disabled]="reportBusy()"
        (click)="openOrGenerateReport(session()!.session_id)">
  {{ reportExists() ? 'Open report' : 'Generate report' }}
</button>
```

(The exact accessor for the current session may be `session()` or `data().session.session_id` — match what the existing component already uses.)

- [ ] **Step 3: Build the SPA**

Run: `cd web && npm run build`
Expected: build succeeds.

- [ ] **Step 4: Manual UI smoke test**

`cargo tauri dev` → open any session → click "Generate report" → file opens in the system browser → button now reads "Open report".

- [ ] **Step 5: Commit**

```bash
git add web/src/app/core/api.service.ts web/src/app/features/sessions/session-detail.component.ts
git commit -m "feat(ui): open-report button on session detail"
```

---

### Task C6: End-to-end report verification

- [ ] **Step 1: Run an actual Claude Code session and let it exit**

Start `cargo tauri dev`. In a separate terminal, run `claude` (or any short Claude Code session) to completion (use `/exit`). Within 5 seconds, confirm `~/.andon/reports/<session_id>.html` exists.

- [ ] **Step 2: Verify offline rendering**

Disconnect from the network (or use airplane mode), double-click the HTML file. Confirm charts render, tables populate, no broken-image icons, no console errors when opened with `--inspector` browsers.

- [ ] **Step 3: Verify idempotency**

Trigger `POST /api/hooks/session-end` twice for the same session. Confirm `ended_at` is only set the first time, but the report file is re-rendered both times.

- [ ] **Step 4: Verify orphan session**

Trigger `POST /api/hooks/session-end` with a `session_id` that has no rows in any table. Confirm a stub `sessions` row is inserted and a sparse-but-valid report renders.

- [ ] **Step 5: Tag the phase**

```bash
git tag -a phase-c-reports -m "v0.3.0 Phase C — reports complete"
```

---

## Final acceptance pass (after all three phases)

- [ ] Run `cargo test -p andon` — all unit tests pass.
- [ ] Run `cargo build --release -p andon` — release binary builds.
- [ ] Launch the release binary twice — second exits 0, first window focuses.
- [ ] Run a Claude Code session with forwarder disabled — no outbound traffic, local data populates.
- [ ] Enable forwarder, repeat — second collector receives matching metrics within 2 seconds.
- [ ] End that session via `/exit` — report appears in `~/.andon/reports/` within 5 seconds; opens cleanly offline; "Open report" button works in the UI.
- [ ] Hit `POST /api/integration/unpatch` — both hooks removed from `~/.claude/settings.json`.
- [ ] Bump `src-tauri/Cargo.toml` version from `0.2.5` to `0.3.0`.
- [ ] Commit version bump, tag `v0.3.0`.

---

## Notes

- **No DB migration** — settings live in JSON, reports live as files, `sessions.ended_at` already exists.
- **Privacy invariants:** forwarder default disabled; reports written into existing user-only `~/.andon/`; single-instance is local IPC only.
- **The `~/.andon` directory** is already created on startup by `config::Paths::resolve_and_prepare()`. `reports/` is created on demand by the renderer.
- **`tauri-plugin-opener`** is registered for completeness (and for future "open data folder" rework), but the current `open_report` handler uses `std::process::Command` to avoid threading an `AppHandle` into axum.

---

Plan complete and saved to `docs/superpowers/plans/2026-05-17-singleton-forwarder-reports.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
