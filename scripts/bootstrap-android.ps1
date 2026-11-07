[CmdletBinding()]
param(
    [string]$CommandLineToolsRevision = '15859902',
    [string]$BuildToolsVersion = '36.0.0'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'dev-shell.ps1')

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$downloads = Join-Path $repoRoot '.tools\downloads'
$jdkArchive = Join-Path $downloads 'microsoft-jdk-17.0.20.1-windows-x64.zip'
$jdkChecksumFile = "$jdkArchive.sha256"
$jdkUrl = 'https://aka.ms/download-jdk/microsoft-jdk-17.0.20.1-windows-x64.zip'
$jdkChecksumUrl = "$jdkUrl.sha256sum.txt"
$commandLineToolsArchive = Join-Path $downloads "commandlinetools-win-$CommandLineToolsRevision`_latest.zip"
$commandLineToolsUrl = "https://dl.google.com/android/repository/commandlinetools-win-$CommandLineToolsRevision`_latest.zip"
$commandLineToolsSha256 = '90ae805d20434428bffcb699c290860f19bb5f66a67e6b330067e3de801fb04a'
$repoPrefix = $repoRoot.TrimEnd('\') + '\'

function Assert-WorkspacePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = [System.IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a path outside the repository: $resolved"
    }
    return $resolved
}

function Assert-Sha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path. Expected $Expected, got $actual."
    }
}

if (-not (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin\java.exe'))) {
    Write-Host 'Downloading Microsoft Build of OpenJDK 17...'
    if (-not (Test-Path -LiteralPath $jdkArchive)) {
        Invoke-WebRequest -UseBasicParsing -Uri $jdkUrl -OutFile $jdkArchive
    }
    Invoke-WebRequest -UseBasicParsing -Uri $jdkChecksumUrl -OutFile $jdkChecksumFile
    $expectedJdkHash = ((Get-Content -Raw -LiteralPath $jdkChecksumFile).Trim() -split '\s+')[0]
    Assert-Sha256 -Path $jdkArchive -Expected $expectedJdkHash

    $extractRoot = Assert-WorkspacePath (Join-Path $repoRoot '.cache\jdk17-extract')
    if (Test-Path -LiteralPath $extractRoot) {
        Remove-Item -Recurse -Force -LiteralPath $extractRoot
    }
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    Expand-Archive -LiteralPath $jdkArchive -DestinationPath $extractRoot
    $jdkRoot = Get-ChildItem -LiteralPath $extractRoot -Directory | Select-Object -First 1
    if ($null -eq $jdkRoot -or -not (Test-Path -LiteralPath (Join-Path $jdkRoot.FullName 'bin\java.exe'))) {
        throw 'The downloaded JDK archive did not contain the expected bin\java.exe.'
    }
    $localJdk = Assert-WorkspacePath $env:JAVA_HOME
    if (Test-Path -LiteralPath $localJdk) {
        Remove-Item -Recurse -Force -LiteralPath $localJdk
    }
    New-Item -ItemType Directory -Force -Path $localJdk | Out-Null
    Get-ChildItem -Force -LiteralPath $jdkRoot.FullName | Move-Item -Destination $env:JAVA_HOME
    Remove-Item -Recurse -Force -LiteralPath $extractRoot
}

$sdkManager = Join-Path $env:ANDROID_SDK_ROOT 'cmdline-tools\latest\bin\sdkmanager.bat'
if (-not (Test-Path -LiteralPath $sdkManager)) {
    Write-Host 'Downloading Android SDK command-line tools...'
    if (-not (Test-Path -LiteralPath $commandLineToolsArchive)) {
        Invoke-WebRequest -UseBasicParsing -Uri $commandLineToolsUrl -OutFile $commandLineToolsArchive
    }
    Assert-Sha256 -Path $commandLineToolsArchive -Expected $commandLineToolsSha256

    $extractRoot = Assert-WorkspacePath (Join-Path $repoRoot '.cache\android-command-tools-extract')
    if (Test-Path -LiteralPath $extractRoot) {
        Remove-Item -Recurse -Force -LiteralPath $extractRoot
    }
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    Expand-Archive -LiteralPath $commandLineToolsArchive -DestinationPath $extractRoot
    $latestRoot = Assert-WorkspacePath (Join-Path $env:ANDROID_SDK_ROOT 'cmdline-tools\latest')
    if (Test-Path -LiteralPath $latestRoot) {
        Remove-Item -Recurse -Force -LiteralPath $latestRoot
    }
    New-Item -ItemType Directory -Force -Path $latestRoot | Out-Null
    Get-ChildItem -Force -LiteralPath (Join-Path $extractRoot 'cmdline-tools') | Move-Item -Destination $latestRoot
    Remove-Item -Recurse -Force -LiteralPath $extractRoot
}

Write-Host 'Accepting Android SDK licenses for this project-local SDK...'
$licenseAnswers = ((1..200 | ForEach-Object { 'y' }) -join "`n") + "`n"
$licenseAnswers | & $sdkManager --sdk_root=$env:ANDROID_SDK_ROOT --licenses | Out-Host
if ($LASTEXITCODE -ne 0) {
    throw "sdkmanager --licenses failed with exit code $LASTEXITCODE."
}

Write-Host 'Installing pinned Android SDK packages...'
& $sdkManager --sdk_root=$env:ANDROID_SDK_ROOT --channel=0 'platform-tools' 'platforms;android-36' "build-tools;$BuildToolsVersion"
if ($LASTEXITCODE -ne 0) {
    throw "sdkmanager package installation failed with exit code $LASTEXITCODE."
}

& (Join-Path $env:JAVA_HOME 'bin\java.exe') -version
& (Join-Path $env:ANDROID_SDK_ROOT 'platform-tools\adb.exe') version
& $sdkManager --sdk_root=$env:ANDROID_SDK_ROOT --list_installed
