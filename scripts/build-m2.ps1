[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifactDirectory = Join-Path $repositoryRoot 'out\m2'
$androidArtifact = Join-Path $repositoryRoot 'app\build\outputs\apk\debug\gnirehtet-ng-debug.apk'
$rustRelease = Join-Path $repositoryRoot '.cargo-target\release'

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE"
    }
}

Push-Location $repositoryRoot
try {
    . (Join-Path $PSScriptRoot 'dev-shell.ps1')

    Invoke-Checked -Description 'Android verification build' -Command {
        & .\gradlew.bat --no-daemon :app:check :app:assembleDebug
    }

    Push-Location (Join-Path $repositoryRoot 'relay-rust')
    try {
        Invoke-Checked -Description 'Rust formatting check' -Command { cargo fmt -- --check }
        Invoke-Checked -Description 'Rust Clippy checks' -Command { cargo clippy --locked --all-targets }
        Invoke-Checked -Description 'Rust tests' -Command { cargo test --locked --no-fail-fast }
        Invoke-Checked -Description 'Rust release build' -Command { cargo build --locked --release --bins }
    } finally {
        Pop-Location
    }

    New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null
    Copy-Item -LiteralPath $androidArtifact -Destination (Join-Path $artifactDirectory 'gnirehtet-ng.apk') -Force
    foreach ($binary in @('gnirehtet-ng.exe', 'gnirehtet-core.exe', 'gnirehtet-cli.exe')) {
        Copy-Item -LiteralPath (Join-Path $rustRelease $binary) -Destination (Join-Path $artifactDirectory $binary) -Force
    }

    $help = & (Join-Path $artifactDirectory 'gnirehtet-cli.exe') help 2>&1 | Out-String
    foreach ($command in @('status', 'start', 'stop', 'restart', 'devices', 'logs', 'diagnose')) {
        if ($help -notmatch "(?m)^\s*gnirehtet-cli .*\b$command\b") {
            throw "M2 CLI help does not advertise required command '$command'."
        }
    }

    Invoke-Checked -Description 'M2 offline IPC and diagnostics integration' -Command {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'test-m2-offline.ps1')
    }

    Write-Host ''
    Write-Host 'M2 artifacts are ready:'
    Get-ChildItem -LiteralPath $artifactDirectory | Select-Object Name, Length | Format-Table -AutoSize
} finally {
    Pop-Location
}
