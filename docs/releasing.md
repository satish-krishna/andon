# Releasing

How to ship a new andon release.

Manual local-build pipeline. No GitHub Actions — `cargo tauri build` runs on the dev machine and assets are uploaded via `gh release create`. Single-developer project, Windows-only desktop binary; the cost of maintaining a cross-platform CI matrix for one user isn't worth it.

## Steps

### 1. Merge the feature PR (squash)

```powershell
gh pr merge <N> --squash --delete-branch
```

Squash matches the existing history (`feat: … (#4)` style).

### 2. Bump version on `main`

Separate commit, not part of the PR.

- `src-tauri/Cargo.toml` → `version = "X.Y.Z"`
- `src-tauri/tauri.conf.json` → `"version": "X.Y.Z"`

Versioning:

- Bug fixes → patch bump (`0.4.0` → `0.4.1`)
- Features → minor bump (`0.4.x` → `0.5.0`)

Commit message: `chore: bump version to X.Y.Z (<short reason>)`, then push.

### 3. Build installers

```powershell
cd src-tauri
cargo tauri build
```

Takes 5–10 minutes. Outputs:

| Artefact | Path |
|---|---|
| Raw binary | `src-tauri/target/release/andon.exe` |
| NSIS installer | `src-tauri/target/release/bundle/nsis/andon_X.Y.Z_x64-setup.exe` |
| MSI installer | `src-tauri/target/release/bundle/msi/andon_X.Y.Z_x64_en-US.msi` |

> Make sure no `andon.exe` instance is running before the build, or the link step fails with `Access is denied` on the `.exe` file.

### 4. Stage the portable binary

Just a rename of the raw binary so users can tell what version it is:

```powershell
cp src-tauri/target/release/andon.exe src-tauri/target/release/andon_X.Y.Z_x64_portable.exe
```

### 5. Tag and push

```powershell
git tag vX.Y.Z
git push origin vX.Y.Z
```

### 6. Create the GitHub release

```powershell
gh release create vX.Y.Z `
  --title "vX.Y.Z — <short title>" `
  --notes "..." `
  src-tauri/target/release/bundle/nsis/andon_X.Y.Z_x64-setup.exe `
  src-tauri/target/release/bundle/msi/andon_X.Y.Z_x64_en-US.msi `
  src-tauri/target/release/andon_X.Y.Z_x64_portable.exe
```

### Release notes structure

See prior releases on GitHub for tone. Skeleton:

```markdown
## What's new

### <Section>
- …

### Downloads
- **andon_X.Y.Z_x64-setup.exe** — NSIS installer (recommended)
- **andon_X.Y.Z_x64_en-US.msi** — MSI installer (group-policy friendly)
- **andon_X.Y.Z_x64_portable.exe** — portable single binary, no install
```
