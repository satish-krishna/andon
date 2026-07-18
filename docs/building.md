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

This launches the Rust backend and opens the main window automatically in dev mode; the tray icon also appears. The Rust side recompiles and relaunches on change.

> **`cargo tauri dev` does not rebuild the Angular SPA.** `tauri.conf.json` sets no `beforeDevCommand` and no `devUrl`, so the webview loads the prebuilt bundle from `web/dist/web/browser/` exactly as it is on disk — there is no frontend hot-reload. A change to the SPA will not appear in the running app until you rebuild the bundle and restart:
>
> ```powershell
> cd web; npm run build   # rebuild web/dist/web/browser/
> # then restart `cargo tauri dev` so the webview loads the fresh bundle
> ```
>
> Symptom of a stale bundle: you edited a component, the app still behaves the old way, and the built `.js` under `web/dist/web/browser/` is older than your source edit.

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
│   │   ├── jsonl/     # transcript walker, parser, reducer, reconciler, pricing
│   │   ├── api/       # axum routes + DTOs + efficiency aggregator
│   │   ├── db/        # rusqlite pool, migrations, queries
│   │   ├── reports/   # standalone HTML session/diagnostic reports
│   │   ├── budget/    # month-end projection + tray monitor + alert state
│   │   ├── settings.rs
│   │   ├── integration.rs    # patches ~/.claude/settings.json
│   │   ├── autostart.rs      # HKCU\Run registration
│   │   ├── diagnostics.rs
│   │   ├── git_query.rs      # PostToolUse → git activity extraction
│   │   └── repo_inference.rs # cwd / remote / branch / repo_name detection
│   ├── templates/     # MiniJinja templates for HTML reports
│   └── tauri.conf.json
├── web/               # Angular 21 SPA (standalone components, signals)
│   └── src/app/features/
│       ├── overview/
│       ├── efficiency/
│       ├── sessions/
│       ├── files/
│       ├── behaviour/
│       ├── diagnostics/
│       └── settings/
├── docs/              # This documentation
└── scripts/           # Release build + screenshot capture (full + annotated)
```

## Run tests

```powershell
# Rust unit + integration tests (the test-support feature gates shared fixtures)
cd src-tauri; cargo test --features test-support

# Angular tests (Vitest, CI mode by default)
cd web; npm test
```

Both suites are green on every release branch — CI doesn't run them (it's a single-developer project), so local runs are the gate.

## Notes

- WAL mode requires the DB file to live on a local filesystem. Network mounts will misbehave.
- The `opentelemetry-proto` crate version must match the protobuf schema Claude Code emits. If decoding suddenly fails, check the crate version against the latest stable release first.
- The frontend's API base URL is hardcoded to `http://127.0.0.1:8765`. To run the SPA standalone against a running backend during frontend development, just `python -m http.server` the built `dist/web/browser/` — CORS is `Any` on the API.
