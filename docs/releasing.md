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

### 3. Build the release artifacts

```powershell
pwsh scripts/build-release.ps1
```

Takes 5–10 minutes. The script verifies the `Cargo.toml` / `tauri.conf.json`
versions match, checks no `andon.exe` is running (the link step fails with
`Access is denied` otherwise), builds the frontend and the Tauri bundle, stages
the portable binary, and prints a summary of the three artifacts:

| Artefact | Path |
|---|---|
| NSIS installer | `src-tauri/target/release/bundle/nsis/andon_X.Y.Z_x64-setup.exe` |
| MSI installer | `src-tauri/target/release/bundle/msi/andon_X.Y.Z_x64_en-US.msi` |
| Portable binary | `src-tauri/target/release/andon_X.Y.Z_x64_portable.exe` |

The script builds artifacts only — it does not bump the version, tag, or
publish. Those remain the manual steps below.

### 4. Tag and push

```powershell
git tag vX.Y.Z
git push origin vX.Y.Z
```

### 5. Create the GitHub release

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
