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

> **Pre-releases skip the MSI.** A version with a pre-release identifier
> (e.g. `0.5.0-rc.2`) cannot be bundled as an MSI — the Windows Installer
> format rejects non-numeric pre-release segments. The script detects this
> from the version string and builds NSIS + portable only.

The script builds artifacts only — it does not bump the version, tag, or
publish. Those remain the manual steps below.

### 4. Tag and push

```powershell
git tag vX.Y.Z
git push origin vX.Y.Z
```

### 5. Capture release screenshots

For an illustrated release, capture the dashboard pages. Needs **andon
running** (so its API answers on `:8765`) and the frontend built (step 3):

```powershell
cd scripts; npm install            # one-time — pulls puppeteer-core
node capture-release-screenshots.js
```

It reads the version from `tauri.conf.json`, drives headless Chrome through
the running app, blurs every dollar cost, and writes one PNG per page to
`docs/images/release/v<version>/`. Repo paths, token counts and session IDs
are left intact — review the PNGs, re-blur or crop anything sensitive, then
commit them.

### 6. Create the GitHub release

```powershell
gh release create vX.Y.Z `
  --title "vX.Y.Z — <short title>" `
  --notes "..." `
  src-tauri/target/release/bundle/nsis/andon_X.Y.Z_x64-setup.exe `
  src-tauri/target/release/bundle/msi/andon_X.Y.Z_x64_en-US.msi `
  src-tauri/target/release/andon_X.Y.Z_x64_portable.exe
```

For a **pre-release**, add `--prerelease` and omit the MSI asset — the build
produced none:

```powershell
gh release create vX.Y.Z-rc.N --prerelease `
  --title "vX.Y.Z-rc.N — <short title>" `
  --notes "..." `
  src-tauri/target/release/bundle/nsis/andon_X.Y.Z-rc.N_x64-setup.exe `
  src-tauri/target/release/andon_X.Y.Z-rc.N_x64_portable.exe
```

### Release notes structure

See prior releases on GitHub for tone. Skeleton:

```markdown
## What's new

### <Section>
- …

### Downloads
- **andon_X.Y.Z_x64-setup.exe** — NSIS installer (recommended)
- **andon_X.Y.Z_x64_en-US.msi** — MSI installer (group-policy friendly; stable releases only)
- **andon_X.Y.Z_x64_portable.exe** — portable single binary, no install
```

Embed the step-5 screenshots with raw-GitHub URLs pinned to the commit that
added them, so the notes never rot as files move:

```markdown
![Overview](https://raw.githubusercontent.com/<owner>/andon/<commit>/docs/images/release/v<version>/01-overview.png)
```
