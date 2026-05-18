# Design — Single Instance, OTEL Forwarder, Session Reports

**Date:** 2026-05-17
**Status:** Approved for planning
**Target version:** v0.3.0

Three independent features added to Andon:

1. **Single-instance lock** — second launch focuses the existing window.
2. **Configurable OTEL forwarder** — optionally re-emits ingested metrics + logs to a user-configured HTTP/protobuf endpoint.
3. **Per-session HTML reports** — on Claude Code `SessionEnd`, render a self-contained HTML summary to `~/.andon/reports/<session_id>.html`.

All three are independent and can ship in any order; the spec groups them because they share supporting infrastructure (persistent app settings and an extended Claude Code hook footprint).

---

## Feature 1 — Single-instance lock

### Goal

Prevent two Andon processes from racing on the OTLP listener ports and the SQLite WAL. Make the second launch a no-op that surfaces the already-running window.

### Approach

Use `tauri-plugin-single-instance` (Tauri 2 official plugin).

- Add dependency `tauri-plugin-single-instance = "2"` to `src-tauri/Cargo.toml`.
- Register the plugin in `src-tauri/src/lib.rs` during the Tauri builder phase, **before** any OTLP listeners or DB are opened. If the lock is already held, the plugin's init returns early in the secondary process.
- Callback in the primary process receives `(app, argv, cwd)` and:
  - Resolves the main window via `app.get_webview_window("main")`.
  - Calls `.unminimize()`, `.show()`, `.set_focus()`.
  - Ignores argv (Andon has no CLI args today).
- Secondary process exits silently with code 0.
- Lock is per-user (plugin default).

### Why this approach

- Zero schema impact, no UI changes, ~15 lines of Rust.
- Avoids the failure mode where two instances both bind `:4317` and one crashes mid-startup, leaving the user with a confusing error and an orphaned process.

### Edge cases

- Main window hidden in tray when second launch occurs → callback shows + focuses it (this is the desired "bring it to front" behavior).
- Plugin lock file stale after a crash → Tauri plugin handles cleanup on next launch.

### Out of scope

- Cross-user single-instance (different OS users may run their own copy).
- Forwarding CLI args to the primary instance (no CLI args defined).

---

## Feature 2 — Configurable OTEL forwarder

### Goal

Let the user re-emit every OTLP `ExportMetricsServiceRequest` and `ExportLogsServiceRequest` Andon receives to a second, user-configured HTTP/protobuf endpoint, without affecting local persistence or Claude Code's view of success. Default: **disabled**.

### New infrastructure: persistent app settings

Andon currently has no persistent application settings. Introduce them now since the forwarder needs them and future features will reuse the store.

**File:** `~/.andon/settings.json`

**Initial schema (v1):**

```json
{
  "version": 1,
  "forwarder": {
    "enabled": false,
    "endpoint": "",
    "timeout_ms": 2000,
    "headers": {}
  }
}
```

- `version` — integer, for forward-compat migrations.
- `forwarder.enabled` — boolean toggle.
- `forwarder.endpoint` — base URL (e.g. `https://otel.example.com`). Andon appends `/v1/metrics` and `/v1/logs`.
- `forwarder.timeout_ms` — connect + read timeout for each forward attempt.
- `forwarder.headers` — flat map of arbitrary header name → value (e.g. `{"Authorization": "Bearer …"}`).

**Backend module:** `src-tauri/src/settings.rs`

- `AppSettings` struct (serde) matches the JSON schema.
- `SettingsStore` wraps `Arc<RwLock<AppSettings>>` plus the file path.
- On startup: load file if present; otherwise write defaults atomically (tmp file + rename).
- On save: validate, write atomically, swap in-memory copy under write lock.
- Atomic writes prevent partial files on crash.

### Forwarder module

**Location:** `src-tauri/src/otlp/forwarder.rs`

- Owns a single `reqwest::Client` built with the configured timeout, rebuilt when settings change.
- Public surface: `forward_metrics(req: &ExportMetricsServiceRequest)` and `forward_logs(req: &ExportLogsServiceRequest)`.
- Each function:
  1. Reads settings under read lock; if disabled or endpoint empty, returns immediately.
  2. Encodes the protobuf message via `prost::Message::encode_to_vec`.
  3. `tokio::spawn`s a fire-and-forget task that POSTs to `<endpoint>/v1/metrics` (or `/v1/logs`) with `Content-Type: application/x-protobuf`, applying configured headers.
  4. Logs failures at `tracing::warn!`. Never returns an error to the caller.
- The spawned task is detached; ingestion latency is unaffected by network conditions.

### Ingestor integration

The existing `Ingestor` in `src-tauri/src/otlp/ingestor.rs` currently receives decoded `ResourceMetrics` / `ResourceLogs`. For lossless forwarding (including fields Andon doesn't yet model), wire the forwarder one level up:

- In `grpc_server.rs` and `http_server.rs`, after the local ingest call succeeds, invoke `forwarder.forward_metrics(&req)` / `forward_logs(&req)` with the **already-decoded request object**. The forwarder re-encodes to canonical protobuf — this is lossless because prost round-trips the entire wire representation.
- Forwarding happens after local persistence to ensure local writes are never delayed by network I/O, and so a slow remote can't cause Claude Code to retry.
- gRPC inbound is normalized to HTTP/protobuf outbound (per the brainstorming decision); the OTLP HTTP receiver spec accepts this exact content type.

### API

| Method | Path                           | Body                                                | Response                                    |
|--------|--------------------------------|-----------------------------------------------------|---------------------------------------------|
| GET    | `/api/settings`                | —                                                   | Full `AppSettings`                          |
| PUT    | `/api/settings/forwarder`      | `{enabled, endpoint, timeout_ms, headers}`          | Updated `forwarder` block                   |
| POST   | `/api/settings/forwarder/test` | `{endpoint, timeout_ms, headers}` (need not be saved) | `{ok: bool, status?: number, error?: string}` |

- `PUT` validates: `endpoint` must parse as `http://` or `https://` URL when `enabled=true`; `timeout_ms` must be 100–30000.
- `test` POSTs an **empty** `ExportMetricsServiceRequest` (zero resource_metrics) to `<endpoint>/v1/metrics`. Most OTLP collectors accept this and return 200; the response surfaces the status code or connection error verbatim.

### Frontend

- New "Forwarder" card on `/settings`:
  - Enable toggle (SpartanNG switch).
  - Endpoint input.
  - Timeout input (numeric, ms).
  - Headers editor: dynamic list of `(key, value)` rows with add/remove buttons.
  - "Test connection" button → calls test endpoint, shows green/red toast with status.
  - "Save" button persists via `PUT /api/settings/forwarder`.
- Uses existing SpartanNG components and the same form style as the rest of `/settings`.
- Disabled inputs (greyed out) when the toggle is off.

### Edge cases

- Endpoint becomes unreachable mid-session → spawned forward tasks time out at `timeout_ms`, are logged as warnings, dropped. Local ingestion continues.
- Settings file corrupted on disk at startup → log a warning, back up the bad file to `settings.json.corrupt-<ts>`, write defaults, continue.
- User sets `enabled=true` with empty endpoint → API rejects with 400.
- Headers map contains an invalid header name → API rejects with 400 (validate via `http::HeaderName::from_str`).

### Out of scope

- gRPC outbound (HTTP/protobuf only).
- Retry / buffering of failed exports.
- Per-signal-type endpoints (single endpoint serves both metrics and logs).
- mTLS / client certs (use a reverse proxy if needed).

---

## Feature 3 — Per-session HTML reports

### Goal

When a Claude Code session ends, render a self-contained HTML summary of that session to `~/.andon/reports/<session_id>.html`. The user can open it from the session detail page; it works offline and is shareable as a single file.

### Trigger: Claude Code `SessionEnd` hook

Andon already installs a `PostToolUse` hook in `src-tauri/src/integration.rs`. Extend the same patcher to additionally install:

```jsonc
"SessionEnd": [
  {
    "hooks": [
      {
        "type": "command",
        "command": "curl -s -X POST http://127.0.0.1:8765/api/hooks/session-end -H \"Content-Type: application/json\" --data-binary @-"
      }
    ]
  }
]
```

- The detection marker for idempotency uses the URL substring `/api/hooks/session-end` (mirroring the existing pattern).
- The patcher's "already configured" state requires **both** hooks present.
- The unpatcher removes both.

### Endpoint

`POST /api/hooks/session-end`

Request body (Claude Code hook payload, partial):

```json
{
  "session_id": "…",
  "reason": "exit | clear | logout | prompt_input_exit | other"
}
```

Handler steps:

1. Validate `session_id` is non-empty; otherwise 400.
2. If `sessions.ended_at IS NULL`, set it to current unix ms.
3. `tokio::spawn` a background task that renders the report (see below).
4. Return 200 immediately.

The hook's curl is fire-and-forget; the user's terminal isn't blocked on rendering.

### Renderer

**Location:** `src-tauri/src/reports/`
- `mod.rs` — public `generate_report(session_id: &str)` and helpers.
- `model.rs` — `ReportData` aggregate built from existing query helpers.
- `render.rs` — `minijinja` environment + render function.
- `assets.rs` — `include_str!` constants for inlined CSS and Chart.js.
- `templates/session_report.html.j2` — single Jinja2 template under `src-tauri/templates/`.

**Crate choice:** `minijinja` (runtime templates, MIT-licensed, ~200KB compiled, no codegen step). Chosen over `askama` because runtime templates are easier to iterate on without recompiling.

**`ReportData` contents** (mirror the session detail page):

- Session metadata: id, started_at, ended_at, duration, service_version, host_arch, os_type, terminal_type.
- KPIs: total cost USD, total tokens (by type), accept rate, total active time (user + cli).
- Cost-by-model bar data (small JSON literal embedded in HTML).
- Token-by-type line data (small JSON literal embedded in HTML).
- File changes table: path, lines added, lines removed, accept rate per file.
- Tool decisions timeline: timestamp, tool_name, decision, language, file_path.

All data sourced via the existing query helpers in `src-tauri/src/db/queries.rs`; the renderer adds no new SQL beyond a handful of `WHERE session_id = ?` variants of existing queries.

**Self-contained output:**

- CSS: ~5KB hand-rolled (no Tailwind, no SpartanNG — those are SPA-only). Inlined in a `<style>` block.
- JS: `chart.umd.min.js` (~70KB, MIT-licensed) inlined in a `<script>` block.
- Data: inlined as `<script type="application/json" id="report-data">…</script>`, parsed by a small initialization script also in the template.
- No `<link>` or `<script src="">` references. File works without network.

**Output location:** `~/.andon/reports/<session_id>.html`

- Directory created on demand.
- Write is atomic (tmp file + rename) to prevent half-written reports if Andon crashes mid-render.
- Re-rendering an existing report overwrites it (idempotent).

### Access

| Method | Path                                | Purpose                                                  |
|--------|-------------------------------------|----------------------------------------------------------|
| GET    | `/api/sessions/:id/report`          | `{exists: bool, path: string, generated_at?: number}`   |
| POST   | `/api/sessions/:id/report`          | Regenerate on demand; returns same shape as GET         |
| POST   | `/api/sessions/:id/report/open`     | Open the file in the OS default browser; returns `{ok}` |

- The `open` action uses `tauri-plugin-opener` (`opener::open_path`). Path is `~/.andon/reports/<session_id>.html`; if not present, the API returns 404 and the UI surfaces a "Generate now" affordance.

### Frontend

- Session detail page gains an "Open report" button in the header.
  - If `exists=true` → enabled, opens via `POST …/report/open`.
  - If `exists=false` → button shows "Generate report"; on click calls `POST …/report`, then `…/report/open`.
- Sessions list gets a small icon column indicating which sessions have generated reports (cheap lookup: filesystem `exists()` per row, batched via a new `GET /api/sessions/reports/index` returning an array of session ids that have reports).

### Edge cases

- Hook fires for a session Andon never saw (e.g. session started before Andon was running, ended after) → no `sessions` row exists. The handler inserts a stub row with `started_at = ended_at` (zero-duration placeholder) and proceeds. The report will be sparse but valid; the UI may render "session metadata incomplete" if `started_at == ended_at`.
- Hook fires multiple times for the same session → first updates `ended_at`, later calls leave it; each call regenerates the report file.
- Render fails (template error, IO error) → logged at `error`, report file left in whatever state it was (atomic-write means no partial file). The "Generate report" button remains available for retry.
- Andon not running when the hook fires → `curl` fails silently inside Claude Code (it's a `-s` POST with no error propagation by design); user opens Andon later and clicks "Generate report" manually.
- Session id used as filename: sanitize defensively (allow only `[A-Za-z0-9_-]`), even though Claude Code session ids are already UUIDs.

### Out of scope

- Reports for non-`session_id`-keyed events.
- AI-generated narrative summaries (would require an LLM call; violates the no-outbound-network rule in `CLAUDE.md`).
- Report indexing UI (browse all reports as a list separate from sessions).
- Embedding reports inside the Tauri webview (out of scope per brainstorming decision; reports always open in the system browser).

---

## Cross-cutting

### New crates

| Crate                         | Version | Purpose                                  |
|-------------------------------|---------|------------------------------------------|
| `tauri-plugin-single-instance`| `2`     | Feature 1                                |
| `tauri-plugin-opener`         | `2`     | Feature 3 — open report in OS browser    |
| `reqwest`                     | `0.12`  | Feature 2 — outbound HTTP/protobuf POST. Use `default-features = false`, enable `rustls-tls`, `http2`. |
| `minijinja`                   | `2`     | Feature 3 — HTML template rendering      |

### Database

- **No schema migration required.** `sessions.ended_at` already exists. Reports are filesystem-backed, not DB-backed. Settings are file-backed.

### Filesystem layout (post-change)

```
~/.andon/
├── data.db
├── data.db-wal
├── data.db-shm
├── log.txt
├── settings.json          ← new (Feature 2)
└── reports/               ← new (Feature 3)
    └── <session_id>.html
```

### Privacy invariants preserved

- Single-instance lock: purely local IPC, no network.
- Forwarder: default disabled; when enabled, sends only OTLP data already received from Claude Code, to a user-chosen endpoint. No telemetry-of-telemetry.
- Reports: written to user-only-readable directory (existing `~/.andon` perms apply).

### Acceptance criteria

- [ ] Launching a second `andon.exe` while one is running shows + focuses the existing window and exits the second process with code 0.
- [ ] `/api/settings/forwarder/test` succeeds against a local OTel collector running on a separate port.
- [ ] With forwarder enabled, every metric Andon receives is also received by the configured collector within 2 seconds; with forwarder disabled, no outbound traffic occurs.
- [ ] Forwarder failures (collector down, slow, returning 5xx) do not block local ingestion and do not cause Claude Code to retry.
- [ ] Ending a Claude Code session (`/exit`) produces `~/.andon/reports/<session_id>.html` within 5 seconds.
- [ ] The generated HTML file renders correctly when opened from the filesystem with no network connection.
- [ ] Session detail page "Open report" button opens the file in the system default browser.
- [ ] All three features can be disabled / uninstalled cleanly (forwarder via toggle; SessionEnd hook removed by existing unpatch flow extended).
