# andon

*Andon* (アンドン) — the lean-manufacturing signal board that surfaces what's happening on the factory floor. This is the andon board for Claude Code usage.

A local, single-binary desktop app that ingests Claude Code OpenTelemetry data, stores it in embedded SQLite, and renders a reporting dashboard. Everything runs on `127.0.0.1`. No cloud, no auth, no outbound network.

Works with **any Claude Code subscription** (Pro, Max, Team, Enterprise, API key) — telemetry is emitted client-side regardless of plan.

## What you get

- **Today's cost / sessions / accept-rate** at a glance
- **Cost by model** (last 30 days, stacked bar)
- **Token usage by type** (input / output / cache, line chart)
- **Accept rate by language**
- **Active time** split user vs CLI
- Per-session detail (cost, tokens, files touched, decision timeline)
- File edit heatmap colored by accept rate

## Requirements

- Windows 10/11 (x64). No admin rights required.
- That's it. SQLite, the OTLP receivers, and the web UI are all bundled in a single executable.

## Install

1. Download the latest `andon.exe` from the [releases page](https://github.com/satish-krishna/andon/releases) (once built).
2. Double-click to launch. A tray icon appears (yellow disc).
3. Left-click the tray icon → window opens.

## Wire up Claude Code

Add the following to `%USERPROFILE%\.claude\settings.json`:

```json
{
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "OTEL_METRICS_EXPORTER": "otlp",
    "OTEL_LOGS_EXPORTER": "otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
    "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:4317"
  }
}
```

Restart any open Claude Code sessions. Run a session — within ~10 seconds you'll see today's numbers populate on the Overview page.

## Data location

All data lives in `%USERPROFILE%\.andon\`:

| File           | Purpose                              |
|----------------|--------------------------------------|
| `data.db`      | SQLite database (WAL mode)           |
| `log.txt`      | Rotating daily log                   |

Open from the **Settings → Database** panel via the "Open data folder" button.

## Ports

| Port  | Bind        | Purpose                       |
|-------|-------------|-------------------------------|
| 4317  | 127.0.0.1   | OTLP gRPC ingest              |
| 4318  | 127.0.0.1   | OTLP HTTP/protobuf ingest     |
| 8765  | 127.0.0.1   | Internal API + SPA backend    |

All bind to loopback only. If any port is already in use (e.g., another OTLP collector), andon will fail to start with a clear error.

## Pause ingestion

Tray menu → **Pause Ingestion** drops incoming metrics on the floor without closing the listeners. Tray menu → **Resume Ingestion** re-enables. Also toggleable from the Settings page.

## Build from source

```powershell
# One-time setup
rustup default stable-x86_64-pc-windows-msvc
# Visual Studio Build Tools 2022 with "Desktop development with C++" workload required.
cargo install tauri-cli --version "^2.0" --locked

# Frontend
cd web
npm install
npm run build

# Run dev
cd ..
cargo tauri dev

# Production build
cargo tauri build
```

The release binary lands in `src-tauri/target/release/bundle/`.

## License

MIT.
