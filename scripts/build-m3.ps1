[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [switch]$SkipM2Build,
    [switch]$SkipSmokeTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$m2Artifacts = Join-Path $repositoryRoot 'out\m2'
$output = Join-Path $repositoryRoot "out\m3\x64\$Configuration"
$artifacts = @('gnirehtet-core.exe', 'gnirehtet-cli.exe', 'gnirehtet-ng.exe', 'gnirehtet-ng.apk')

Push-Location $repositoryRoot
try {
    . (Join-Path $PSScriptRoot 'dev-shell.ps1')
    if (-not $SkipM2Build) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'build-m2.ps1')
        if ($LASTEXITCODE -ne 0) { throw "Controller, relay and Android build failed ($LASTEXITCODE)." }
    }
    foreach ($artifact in $artifacts) {
        if (-not (Test-Path -LiteralPath (Join-Path $m2Artifacts $artifact))) {
            throw "Required M2 artifact is missing: $artifact"
        }
    }

    $cargoArgs = @('build', '--locked', '--manifest-path', 'windows\Cargo.toml')
    $profile = 'debug'
    if ($Configuration -eq 'Release') {
        $cargoArgs += '--release'
        $profile = 'release'
    }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Slint desktop build failed ($LASTEXITCODE)." }

    New-Item -ItemType Directory -Force -Path $output | Out-Null
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "$profile\gnirehtet-ui.exe") -Destination (Join-Path $output 'GnirehtetNg.Gui.exe') -Force
    foreach ($artifact in $artifacts) {
        Copy-Item -LiteralPath (Join-Path $m2Artifacts $artifact) -Destination $output -Force
    }
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'assets\app-icon.ico') -Destination $output -Force

    if (-not $SkipSmokeTest) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'test-m3-offline.ps1') -Configuration $Configuration
        if ($LASTEXITCODE -ne 0) { throw "Desktop smoke test failed ($LASTEXITCODE)." }
    }
    Write-Output "Desktop artifacts: $output"
} finally {
    Pop-Location
}
