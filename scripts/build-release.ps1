<#
.SYNOPSIS
    Builds the complete set of andon release artifacts.

.DESCRIPTION
    Runs the frontend build, the Tauri production build, and stages the
    portable binary - producing the three downloadables for a GitHub release:

      - andon_X.Y.Z_x64-setup.exe   (NSIS installer)
      - andon_X.Y.Z_x64_en-US.msi   (MSI installer)
      - andon_X.Y.Z_x64_portable.exe (raw binary, renamed)

    This covers steps 3-4 of docs/releasing.md. It does NOT bump the version,
    tag, or publish a release - those stay deliberate manual steps.

.EXAMPLE
    pwsh scripts/build-release.ps1
#>

$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$SrcTauri   = Join-Path $RepoRoot 'src-tauri'
$Web        = Join-Path $RepoRoot 'web'
$ReleaseDir = Join-Path $SrcTauri 'target\release'

function Write-Step($msg) {
    Write-Host ''
    Write-Host "==> $msg" -ForegroundColor Cyan
}

function Assert-ExitCode($what) {
    if ($LASTEXITCODE -ne 0) {
        throw "$what failed (exit code $LASTEXITCODE)."
    }
}

# --- 1. Version consistency -------------------------------------------------

Write-Step 'Checking version consistency'

$cargoToml = Join-Path $SrcTauri 'Cargo.toml'
$cargoMatch = Select-String -Path $cargoToml -Pattern '^version = "(.+)"' | Select-Object -First 1
if (-not $cargoMatch) {
    throw "Could not find a package version in $cargoToml."
}
$cargoVersion = $cargoMatch.Matches[0].Groups[1].Value

$confPath = Join-Path $SrcTauri 'tauri.conf.json'
$confVersion = (Get-Content $confPath -Raw | ConvertFrom-Json).version

if ($cargoVersion -ne $confVersion) {
    throw "Version mismatch: Cargo.toml is '$cargoVersion' but tauri.conf.json is '$confVersion'. Reconcile them before building."
}
$Version = $cargoVersion
Write-Host "    version $Version" -ForegroundColor Green

# --- 2. Pre-flight ----------------------------------------------------------

Write-Step 'Pre-flight checks'

if (Get-Process -Name 'andon' -ErrorAction SilentlyContinue) {
    throw "An 'andon.exe' process is running. Quit it first - the link step fails with 'Access is denied' otherwise."
}

& cargo tauri --version *> $null
if ($LASTEXITCODE -ne 0) {
    throw "The 'cargo tauri' subcommand is not available. Install it with: cargo install tauri-cli --version `"^2.0`" --locked"
}
Write-Host '    no running andon.exe; cargo tauri present' -ForegroundColor Green

# --- 3. Frontend ------------------------------------------------------------

Write-Step 'Building the Angular frontend (npm ci + npm run build)'

Push-Location $Web
try {
    & npm ci
    Assert-ExitCode 'npm ci'
    & npm run build
    Assert-ExitCode 'npm run build'
}
finally {
    Pop-Location
}

# --- 4. Installers ----------------------------------------------------------

Write-Step 'Building the Tauri production bundle (cargo tauri build)'

Push-Location $SrcTauri
try {
    & cargo tauri build
    Assert-ExitCode 'cargo tauri build'
}
finally {
    Pop-Location
}

# --- 5. Stage the portable binary ------------------------------------------

Write-Step 'Staging the portable binary'

$rawExe      = Join-Path $ReleaseDir 'andon.exe'
$portableExe = Join-Path $ReleaseDir "andon_${Version}_x64_portable.exe"
if (-not (Test-Path $rawExe)) {
    throw "Expected raw binary not found at $rawExe."
}
Copy-Item $rawExe $portableExe -Force
Write-Host "    $portableExe" -ForegroundColor Green

# --- 6. Collect + report ----------------------------------------------------

Write-Step 'Release artifacts'

$nsis = Get-ChildItem -Path (Join-Path $ReleaseDir 'bundle\nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue
$msi  = Get-ChildItem -Path (Join-Path $ReleaseDir 'bundle\msi')  -Filter '*.msi'        -ErrorAction SilentlyContinue
if (-not $nsis) { throw 'No NSIS installer (*-setup.exe) found under target/release/bundle/nsis.' }
if (-not $msi)  { throw 'No MSI installer (*.msi) found under target/release/bundle/msi.' }

$artifacts = @($nsis) + @($msi) + @(Get-Item $portableExe)
$artifacts |
    Select-Object `
        @{ Name = 'Artifact'; Expression = { $_.Name } },
        @{ Name = 'Size';     Expression = { '{0:N1} MB' -f ($_.Length / 1MB) } },
        @{ Name = 'Path';     Expression = { $_.FullName } } |
    Format-Table -AutoSize

Write-Host "Release artifacts for v$Version are ready." -ForegroundColor Green
Write-Host 'Next: tag and publish per docs/releasing.md (steps 5-6).'
