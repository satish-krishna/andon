# Building from source

Andon is a Tauri 2 app: Rust backend, Angular frontend, bundled together by the Tauri CLI.

## Prerequisites (Windows)

1. **Rust** — install via [rustup](https://rustup.rs/) and pick the MSVC toolchain:

   ```powershell
   rustup default stable-x86_64-pc-windows-msvc
   ```

2. **Visual Studio Build Tools 2022** with the **"Desktop development with C++"** workload. Required by the MSVC toolchain and by `rusqlite`'s bundled SQLite.

3. **Node.js 20+** for the Angular build.

4. **Tauri CLI**:

   ```powershell
   cargo install tauri-cli --version "^2.0" --locked
   ```

## Build the frontend

```powershell
cd web
npm install
npm run build
```

Output lands in `web/dist/web/browser/` — this is the path Tauri's `frontendDist` points at.

## Run in dev

From the repo root:

```powershell
cargo tauri dev
```

This launches the Rust backend with hot-reload of the embedded SPA. The tray icon appears; the main window opens automatically in dev mode.

## Production build

```powershell
cargo tauri build
```

Produces:

- `src-tauri/target/release/andon.exe` — the raw binary.
- `src-tauri/target/release/bundle/` — the platform installers (`.msi`, `.exe` NSIS).

## Repo layout

```
andon/
├── src-tauri/         # Rust backend, Tauri config, bundle assets
│   ├── src/
│   │   ├── otlp/      # gRPC + HTTP receivers, ingestor, forwarder
│   │   ├── api/       # axum routes + DTOs for the SPA
│   │   ├── db/        # rusqlite pool, migrations, queries
│   │   ├── reports/   # standalone HTML session reports
│   │   ├── settings.rs
│   │   ├── integration.rs    # patches ~/.claude/settings.json
│   │   ├── autostart.rs      # HKCU\Run registration
│   │   └── diagnostics.rs
│   ├── templates/     # Tera/Jinja2 report templates
│   └── tauri.conf.json
├── web/               # Angular 21 SPA (standalone components, signals)
│   └── src/app/features/
│       ├── overview/
│       ├── sessions/
│       ├── files/
│       ├── diagnostics/
│       └── settings/
├── docs/              # This documentation
└── scripts/           # Release / packaging helpers
```

## Notes

- WAL mode requires the DB file to live on a local filesystem. Network mounts will misbehave.
- The `opentelemetry-proto` crate version must match the protobuf schema Claude Code emits. If decoding suddenly fails, check the crate version against the latest stable release first.
- The frontend's API base URL is hardcoded to `http://127.0.0.1:8765`. To run the SPA standalone against a running backend during frontend development, just `python -m http.server` the built `dist/web/browser/` — CORS is `Any` on the API.
