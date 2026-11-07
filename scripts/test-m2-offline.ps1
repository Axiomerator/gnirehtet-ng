[CmdletBinding()]
param(
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifactDirectory = Join-Path $repositoryRoot 'out\m2'
$cli = Join-Path $artifactDirectory 'gnirehtet-cli.exe'
$stateDirectory = Join-Path $repositoryRoot ("out\m2-offline\{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$corePid = $null

function Invoke-CliText {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [switch]$ExpectFailure
    )

    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $cli --json @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    $text = ($output | Out-String).Trim()
    if ($ExpectFailure) {
        if ($exitCode -eq 0) {
            throw "CLI command '$($Arguments -join ' ')' unexpectedly succeeded."
        }
    } elseif ($exitCode -ne 0) {
        throw "CLI command '$($Arguments -join ' ')' failed with exit code ${exitCode}: $text"
    }
    return $text
}

function Invoke-CliJson {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $text = Invoke-CliText -Arguments $Arguments
    try {
        return $text | ConvertFrom-Json
    } catch {
        throw "CLI command '$($Arguments -join ' ')' returned invalid JSON: $text"
    }
}

Push-Location $repositoryRoot
try {
    $existingCore = Get-Process -Name 'gnirehtet-core' -ErrorAction SilentlyContinue
    if ($existingCore) {
        throw "A gnirehtet-core process is already running (PID $($existingCore.Id -join ', ')). Stop it before the isolated offline test."
    }

    if ($Build) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'build-m2.ps1')
        if ($LASTEXITCODE -ne 0) {
            throw "M2 build failed with exit code $LASTEXITCODE."
        }
    }
    if (-not (Test-Path -LiteralPath $cli)) {
        throw 'M2 artifacts are missing. Run scripts\build-m2.ps1 or pass -Build.'
    }

    . (Join-Path $PSScriptRoot 'dev-shell.ps1')
    $env:ADB = Join-Path $env:ANDROID_SDK_ROOT 'platform-tools\adb.exe'
    $env:GNIREHTET_APK = Join-Path $artifactDirectory 'gnirehtet-ng.apk'
    $env:GNIREHTET_NG_RELAY = Join-Path $artifactDirectory 'gnirehtet-ng.exe'
    $env:GNIREHTET_NG_STATE_DIR = $stateDirectory

    Write-Host '[offline] status / core auto-start'
    $status = Invoke-CliJson -Arguments @('status')
    if ($status.session_state -ne 'idle') {
        throw "Fresh core did not report idle state: $($status.session_state)"
    }
    $corePid = [int]$status.core_pid

    Write-Host '[offline] devices'
    $devices = Invoke-CliJson -Arguments @('devices')
    $deviceCount = @($devices).Count
    if ($deviceCount -eq 0) {
        foreach ($command in @('start', 'restart')) {
            Write-Host "[offline] expected no-device failure: $command"
            $failure = Invoke-CliText -Arguments @($command) -ExpectFailure
            if ($failure -notmatch 'usb_disconnected') {
                throw "No-device '$command' did not return usb_disconnected: $failure"
            }
        }
    } else {
        Write-Warning 'An ADB device appeared; start/restart were skipped to avoid changing it.'
    }

    Write-Host '[offline] stop / clear inactive fault'
    $stopped = Invoke-CliJson -Arguments @('stop')
    if ($stopped.session_state -ne 'idle' -or $null -ne $stopped.last_fault) {
        throw 'Stop did not clear the inactive/faulted controller state.'
    }

    Write-Host '[offline] metrics'
    $metrics = Invoke-CliJson -Arguments @('metrics')
    foreach ($field in @(
        'bytes_from_device',
        'bytes_to_device',
        'packets_from_device',
        'packets_to_device',
        'bytes_from_device_per_second',
        'bytes_to_device_per_second'
    )) {
        if ($null -eq $metrics.$field) {
            throw "Metrics response is missing '$field'."
        }
    }

    Write-Host '[offline] structured logs'
    $logs = Invoke-CliJson -Arguments @('logs', '--limit', '200')
    if (@($logs).Count -eq 0) {
        throw 'Structured log query returned no events.'
    }

    Write-Host '[offline] diagnostic export and Expand-Archive'
    $diagnostic = Invoke-CliJson -Arguments @('diagnose')
    if (-not (Test-Path -LiteralPath $diagnostic.path)) {
        throw "Diagnostic bundle was not created: $($diagnostic.path)"
    }
    $expanded = Join-Path $stateDirectory 'expanded-diagnostics'
    Expand-Archive -LiteralPath $diagnostic.path -DestinationPath $expanded
    foreach ($name in @(
        'windows.log',
        'android-logcat.txt',
        'state.json',
        'versions.json',
        'adb-devices.txt',
        'adb-reverse.txt',
        'android-exit-info.txt',
        'android-network.txt',
        'settings-redacted.json'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $expanded $name))) {
            throw "Diagnostic bundle is missing '$name'."
        }
    }

    Write-Host '[offline] graceful core shutdown'
    $shutdown = Invoke-CliJson -Arguments @('shutdown-core')
    if ($shutdown.session_state -ne 'idle') {
        throw "Graceful shutdown returned non-idle state: $($shutdown.session_state)"
    }
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if (-not (Get-Process -Id $corePid -ErrorAction SilentlyContinue)) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (Get-Process -Id $corePid -ErrorAction SilentlyContinue) {
        throw "Controller PID $corePid did not exit after shutdown-core."
    }
    $corePid = $null

    Write-Host ''
    Write-Host 'M2 offline integration checks passed.'
    Write-Host "Evidence directory: $stateDirectory"
    Write-Host "Diagnostic bundle: $($diagnostic.path)"
} finally {
    if ($null -ne $corePid -and (Get-Process -Id $corePid -ErrorAction SilentlyContinue)) {
        & $cli shutdown-core 2>&1 | Out-Null
    }
    Pop-Location
}
