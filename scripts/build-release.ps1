<#
.SYNOPSIS
    Builds the complete set of andon release artifacts, with full logging.

.DESCRIPTION
    Runs the frontend build, the Tauri production build, and stages the
    portable binary - producing the release downloadables:

      - andon_X.Y.Z_x64-setup.exe    (NSIS installer)
      - andon_X.Y.Z_x64_en-US.msi    (MSI installer; stable releases only)
      - andon_X.Y.Z_x64_portable.exe (raw binary, renamed)

    Pre-release versions (e.g. 0.5.0-rc.2) skip the MSI: the Windows
    Installer format rejects non-numeric pre-release identifiers. The
    script detects this from the version string and builds NSIS only.

    Every run writes a complete, timestamped transcript to
    scripts/build-release.log. On any failure the script prints which step
    failed and points at that log - it never exits silently.

    This is step 3 of docs/releasing.md. It does NOT bump the version, tag,
    or publish a release - those stay deliberate manual steps.

.EXAMPLE
    pwsh scripts/build-release.ps1
#>

$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$SrcTauri   = Join-Path $RepoRoot 'src-tauri'
$Web        = Join-Path $RepoRoot 'web'
$ReleaseDir = Join-Path $SrcTauri 'target\release'
$LogFile    = Join-Path $PSScriptRoot 'build-release.log'

# Tracks the current phase so the failure handler can name it.
$script:CurrentStep = 'startup'

function Log {
    param([string]$Message, [string]$Color = 'Gray')
    $stamp = (Get-Date).ToString('HH:mm:ss')
    Write-Host "[$stamp] $Message" -ForegroundColor $Color
}

function Step {
    param([string]$Name)
    $script:CurrentStep = $Name
    Write-Host ''
    Log "==> $Name" 'Cyan'
}

function Assert-ExitCode {
    param([string]$What)
    if ($LASTEXITCODE -ne 0) {
        throw "$What failed (exit code $LASTEXITCODE)."
    }
}

# Start the transcript before anything else so the whole run - including a
# crash or an early abort - is captured to disk. If a transcript is somehow
# already running in this session, carry on without one rather than failing.
$transcriptOn = $false
try {
    Start-Transcript -Path $LogFile -Force | Out-Null
    $transcriptOn = $true
}
catch {
    Write-Host "WARNING: could not start the transcript log ($($_.Exception.Message))." -ForegroundColor Yellow
    Write-Host 'Continuing without a log file.' -ForegroundColor Yellow
}

try {
    Log "andon release build" 'White'
    Log "log file:  $LogFile"
    Log "repo root: $RepoRoot"

    # --- 1. Version consistency --------------------------------------------
    Step 'Checking version consistency'

    $cargoToml = Join-Path $SrcTauri 'Cargo.toml'
    if (-not (Test-Path $cargoToml)) {
        throw "Cargo.toml not found at $cargoToml - run this script from inside the andon repo."
    }
    $cargoMatch = Select-String -Path $cargoToml -Pattern '^version = "(.+)"' | Select-Object -First 1
    if (-not $cargoMatch) {
        throw "Could not find a package version in $cargoToml."
    }
    $cargoVersion = $cargoMatch.Matches[0].Groups[1].Value

    $confPath = Join-Path $SrcTauri 'tauri.conf.json'
    if (-not (Test-Path $confPath)) {
        throw "tauri.conf.json not found at $confPath."
    }
    $confVersion = (Get-Content $confPath -Raw | ConvertFrom-Json).version

    if ($cargoVersion -ne $confVersion) {
        throw "Version mismatch: Cargo.toml is '$cargoVersion' but tauri.conf.json is '$confVersion'. Reconcile them before building."
    }
    $Version = $cargoVersion
    Log "version $Version (Cargo.toml and tauri.conf.json agree)" 'Green'

    # A SemVer pre-release version carries an identifier after '-' (e.g.
    # 0.5.0-rc.2). The MSI target cannot bundle one - the Windows Installer
    # format requires a numeric-only pre-release segment - so pre-release
    # builds ship the NSIS installer + portable binary only.
    $IsPrerelease = $Version -match '-'
    if ($IsPrerelease) {
        Log 'pre-release version - the MSI target will be skipped' 'Yellow'
    }

    # --- 2. Pre-flight ------------------------------------------------------
    Step 'Pre-flight checks'

    if (Get-Process -Name 'andon' -ErrorAction SilentlyContinue) {
        throw "An 'andon.exe' process is running. Quit it first - the link step fails with 'Access is denied' otherwise."
    }
    Log 'no running andon.exe' 'Green'

    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        throw "'npm' is not on PATH. Install Node.js 20+ and reopen the shell."
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "'cargo' is not on PATH. Open a shell where Rust is available (PATH must include %USERPROFILE%\.cargo\bin)."
    }
    Log 'npm and cargo found on PATH' 'Green'

    # No stream redirection here: if 'cargo tauri' is missing, cargo's own
    # error text must reach the log instead of being swallowed.
    & cargo tauri --version
    if ($LASTEXITCODE -ne 0) {
        throw "The 'cargo tauri' subcommand is not available. Install it with: cargo install tauri-cli --version `"^2.0`" --locked"
    }
    Log 'cargo tauri subcommand present' 'Green'

    # --- 3. Frontend --------------------------------------------------------
    Step 'Building the Angular frontend (npm ci + npm run build)'

    Push-Location $Web
    try {
        Log 'running: npm ci'
        & npm ci
        Assert-ExitCode 'npm ci'
        Log 'running: npm run build'
        & npm run build
        Assert-ExitCode 'npm run build'
    }
    finally {
        Pop-Location
    }
    Log 'frontend build complete' 'Green'

    # --- 4. Installers ------------------------------------------------------
    Step 'Building the Tauri production bundle (cargo tauri build)'

    Push-Location $SrcTauri
    try {
        if ($IsPrerelease) {
            Log 'running: cargo tauri build --bundles nsis (this takes several minutes)'
            & cargo tauri build --bundles nsis
        }
        else {
            Log 'running: cargo tauri build (this takes several minutes)'
            & cargo tauri build
        }
        Assert-ExitCode 'cargo tauri build'
    }
    finally {
        Pop-Location
    }
    Log 'tauri bundle build complete' 'Green'

    # --- 5. Stage the portable binary --------------------------------------
    Step 'Staging the portable binary'

    $rawExe      = Join-Path $ReleaseDir 'andon.exe'
    $portableExe = Join-Path $ReleaseDir "andon_${Version}_x64_portable.exe"
    if (-not (Test-Path $rawExe)) {
        throw "Expected raw binary not found at $rawExe - the build did not produce it."
    }
    Copy-Item $rawExe $portableExe -Force
    Log "staged $portableExe" 'Green'

    # --- 6. Collect + report -----------------------------------------------
    Step 'Collecting release artifacts'

    $nsis = Get-ChildItem -Path (Join-Path $ReleaseDir 'bundle\nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue
    if (-not $nsis) { throw 'No NSIS installer (*-setup.exe) found under target/release/bundle/nsis.' }

    # Stable releases must also carry an MSI; pre-releases never build one.
    $artifacts = @($nsis)
    if (-not $IsPrerelease) {
        $msi = Get-ChildItem -Path (Join-Path $ReleaseDir 'bundle\msi') -Filter '*.msi' -ErrorAction SilentlyContinue
        if (-not $msi) { throw 'No MSI installer (*.msi) found under target/release/bundle/msi.' }
        $artifacts += @($msi)
    }

    # The Modified column makes it obvious whether the artifacts are fresh.
    $artifacts += @(Get-Item $portableExe)
    $table = $artifacts |
        Select-Object `
            @{ Name = 'Artifact'; Expression = { $_.Name } },
            @{ Name = 'Size';     Expression = { '{0:N1} MB' -f ($_.Length / 1MB) } },
            @{ Name = 'Modified'; Expression = { $_.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss') } },
            @{ Name = 'Path';     Expression = { $_.FullName } } |
        Format-Table -AutoSize | Out-String
    Write-Host $table

    Write-Host ''
    Log "SUCCESS - release artifacts for v$Version are ready." 'Green'
    Log "Full log: $LogFile"
    Log 'Next: tag and publish per docs/releasing.md (steps 4-5).'
    $script:CurrentStep = 'done'
}
catch {
    Write-Host ''
    Write-Host '================  BUILD FAILED  ================' -ForegroundColor Red
    Write-Host "Step:  $script:CurrentStep" -ForegroundColor Red
    Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host "Log:   $LogFile" -ForegroundColor Red
    Write-Host '================================================' -ForegroundColor Red
    # Full error record (message, position, stack) - goes to console and log.
    Write-Host ($_ | Out-String) -ForegroundColor DarkGray
    if ($transcriptOn) { try { Stop-Transcript | Out-Null } catch {} }
    exit 1
}

if ($transcriptOn) { try { Stop-Transcript | Out-Null } catch {} }
exit 0
