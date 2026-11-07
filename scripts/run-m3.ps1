[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug'
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = Join-Path $repositoryRoot "out\m3\x64\$Configuration"
$gui = Join-Path $output 'GnirehtetNg.Gui.exe'
if (-not (Test-Path -LiteralPath $gui)) {
    throw "M3 $Configuration artifacts are missing. Run scripts\build-m3.ps1 first."
}

. (Join-Path $PSScriptRoot 'dev-shell.ps1')
$env:ADB = Join-Path $env:ANDROID_SDK_ROOT 'platform-tools\adb.exe'
$env:GNIREHTET_APK = Join-Path $output 'gnirehtet-ng.apk'
$env:GNIREHTET_NG_RELAY = Join-Path $output 'gnirehtet-ng.exe'
foreach ($path in @($env:ADB, $env:GNIREHTET_APK, $env:GNIREHTET_NG_RELAY)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required runtime artifact is missing: $path`nRun scripts\build-m3.ps1 first."
    }
}

Start-Process -FilePath $gui
Write-Output "GnirehtetNg.Gui launched"
