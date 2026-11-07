[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = Join-Path $repositoryRoot "out\m3\x64\$Configuration"
$guiExecutable = Join-Path $output 'GnirehtetNg.Gui.exe'
$cliExecutable = Join-Path $output 'gnirehtet-cli.exe'
$stateDirectory = Join-Path $repositoryRoot ("out\m3-offline\{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$guiProcess = $null
$corePid = $null

function Invoke-CliJson {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $text = (& $cliExecutable --json @Arguments 2>&1 | Out-String).Trim()
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    if ($exitCode -ne 0) {
        throw "CLI command '$($Arguments -join ' ')' failed with exit code ${exitCode}: $text"
    }
    try {
        return $text | ConvertFrom-Json
    } catch {
        throw "CLI command '$($Arguments -join ' ')' returned invalid JSON: $text"
    }
}

Push-Location $repositoryRoot
try {
    if (Get-Process -Name 'GnirehtetNg.Gui', 'gnirehtet-core' -ErrorAction SilentlyContinue) {
        throw 'A Gnirehtet NG GUI or controller process is already running. Stop it before the isolated M3 smoke test.'
    }
    foreach ($artifact in @(
        'GnirehtetNg.Gui.exe',
        'gnirehtet-core.exe',
        'gnirehtet-cli.exe',
        'gnirehtet-ng.exe',
        'gnirehtet-ng.apk'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $output $artifact))) {
            throw "M3 runtime artifact is missing: $artifact"
        }
    }

    . (Join-Path $PSScriptRoot 'dev-shell.ps1')
    $env:ADB = Join-Path $env:ANDROID_SDK_ROOT 'platform-tools\adb.exe'
    $env:GNIREHTET_APK = Join-Path $output 'gnirehtet-ng.apk'
    $env:GNIREHTET_NG_RELAY = Join-Path $output 'gnirehtet-ng.exe'
    $env:GNIREHTET_NG_STATE_DIR = $stateDirectory

    Write-Host '[m3-offline] launch Slint without activating a phone session'
    $launchedAt = Get-Date
    $guiProcess = Start-Process -FilePath $guiExecutable -WindowStyle Hidden -PassThru
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($guiProcess.HasExited) {
            $startupLog = Join-Path $env:TEMP 'gnirehtet-ng-gui-startup-error.txt'
            $detail = if ((Test-Path -LiteralPath $startupLog) -and (Get-Item -LiteralPath $startupLog).LastWriteTime -ge $launchedAt) { Get-Content -Raw -LiteralPath $startupLog } else { 'No startup log was written.' }
            throw "Slint exited during startup with code $($guiProcess.ExitCode). $detail"
        }
        $core = Get-Process -Name 'gnirehtet-core' -ErrorAction SilentlyContinue
        if ($core) {
            $corePid = [int]$core.Id
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $corePid) {
        throw 'Slint did not auto-start the local controller within 10 seconds.'
    }

    Write-Host '[m3-offline] verify authoritative snapshot and idle state'
    $status = Invoke-CliJson -Arguments @('status')
    if ($status.core_pid -ne $corePid -or $status.session_state -ne 'idle') {
        throw 'Slint-connected controller did not report the expected fresh idle snapshot.'
    }
    Start-Sleep -Seconds 2
    if ($guiProcess.HasExited) {
        $runtimeLog = Join-Path $env:TEMP 'gnirehtet-ng-gui-runtime-error.txt'
        $detail = if ((Test-Path -LiteralPath $runtimeLog) -and (Get-Item -LiteralPath $runtimeLog).LastWriteTime -ge $launchedAt) { Get-Content -Raw -LiteralPath $runtimeLog } else { 'No runtime log was written.' }
        throw "Slint exited while polling controller state. $detail"
    }
    $logs = Invoke-CliJson -Arguments @('logs', '--limit', '50')
    if (@($logs).Count -eq 0) {
        throw 'Controller structured logs remained empty during the GUI smoke test.'
    }

    Write-Host '[m3-offline] verify quiet no-device standby across a discovery interval'
    $baselineSequence = (@($logs) | Measure-Object -Property sequence -Maximum).Maximum
    Start-Sleep -Seconds 6
    $standbyStatus = Invoke-CliJson -Arguments @('status')
    if ($standbyStatus.session_state -ne 'idle' -or $standbyStatus.recovery_attempt -ne 0) {
        throw 'No-device standby unexpectedly entered a session or recovery cycle.'
    }
    $standbyLogs = Invoke-CliJson -Arguments @('logs', '--limit', '50')
    $standbySequence = (@($standbyLogs) | Measure-Object -Property sequence -Maximum).Maximum
    if ($standbySequence -ne $baselineSequence) {
        throw 'No-device standby emitted repeated controller activity instead of remaining quiet.'
    }

    Write-Host '[m3-offline] verify controller cleanup after GUI process exit'
    Stop-Process -Id $guiProcess.Id
    if (-not $guiProcess.WaitForExit(5000)) {
        throw 'Slint did not exit after the test stopped it.'
    }
    $guiProcess = $null
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if (-not (Get-Process -Id $corePid -ErrorAction SilentlyContinue)) { break }
        Start-Sleep -Milliseconds 100
    }
    if (Get-Process -Id $corePid -ErrorAction SilentlyContinue) {
        throw "Controller PID $corePid remained after shutdown-core."
    }
    $corePid = $null

    Write-Host ''
    Write-Host 'M3 no-device Slint smoke test passed.'
    Write-Host "Evidence directory: $stateDirectory"
} finally {
    if ($null -ne $guiProcess -and -not $guiProcess.HasExited) {
        Stop-Process -Id $guiProcess.Id -ErrorAction SilentlyContinue
    }
    if ($null -ne $corePid -and (Get-Process -Id $corePid -ErrorAction SilentlyContinue)) {
        Stop-Process -Id $corePid -ErrorAction SilentlyContinue
    }
    Pop-Location
}
