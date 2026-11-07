[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CliArguments
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifactDirectory = Join-Path $repositoryRoot 'out\m2'
$cli = Join-Path $artifactDirectory 'gnirehtet-cli.exe'

Push-Location $repositoryRoot
try {
    . (Join-Path $PSScriptRoot 'dev-shell.ps1')
    if (-not (Test-Path -LiteralPath $cli)) {
        throw 'M2 artifacts are missing. Run scripts\build-m2.ps1 first.'
    }

    $env:ADB = Join-Path $env:ANDROID_SDK_ROOT 'platform-tools\adb.exe'
    $env:GNIREHTET_APK = Join-Path $artifactDirectory 'gnirehtet-ng.apk'
    $env:GNIREHTET_NG_RELAY = Join-Path $artifactDirectory 'gnirehtet-ng.exe'
    $env:GNIREHTET_NG_STATE_DIR = Join-Path $repositoryRoot 'out\m2-state'

    if (-not $CliArguments -or $CliArguments.Count -eq 0) {
        $CliArguments = @('status')
    }
    & $cli @CliArguments
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
