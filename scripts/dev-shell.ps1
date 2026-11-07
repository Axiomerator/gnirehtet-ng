[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$toolsRoot = Join-Path $repoRoot '.tools'

$requiredDirectories = @(
    $toolsRoot,
    (Join-Path $toolsRoot 'rustup'),
    (Join-Path $toolsRoot 'cargo'),
    (Join-Path $toolsRoot 'jdk17'),
    (Join-Path $toolsRoot 'android-sdk'),
    (Join-Path $toolsRoot 'downloads'),
    (Join-Path $toolsRoot 'nuget'),
    (Join-Path $toolsRoot 'nuget-packages'),
    (Join-Path $repoRoot '.cache'),
    (Join-Path $repoRoot '.gradle-home'),
    (Join-Path $repoRoot '.cargo-target')
)

foreach ($directory in $requiredDirectories) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}

$env:RUSTUP_HOME = Join-Path $toolsRoot 'rustup'
$env:CARGO_HOME = Join-Path $toolsRoot 'cargo'
$env:JAVA_HOME = Join-Path $toolsRoot 'jdk17'
$env:ANDROID_SDK_ROOT = Join-Path $toolsRoot 'android-sdk'
$env:ANDROID_HOME = $env:ANDROID_SDK_ROOT
$env:GRADLE_USER_HOME = Join-Path $repoRoot '.gradle-home'
$env:CARGO_TARGET_DIR = Join-Path $repoRoot '.cargo-target'
$env:NUGET_PACKAGES = Join-Path $toolsRoot 'nuget-packages'
$env:NUGET_HTTP_CACHE_PATH = Join-Path $repoRoot '.cache\nuget-http'

$projectPathEntries = @(
    (Join-Path $env:CARGO_HOME 'bin'),
    (Join-Path $toolsRoot 'nuget'),
    (Join-Path $env:JAVA_HOME 'bin'),
    (Join-Path $env:ANDROID_SDK_ROOT 'platform-tools'),
    (Join-Path $env:ANDROID_SDK_ROOT 'cmdline-tools\latest\bin')
)

$existingEntries = @($env:Path -split ';' | Where-Object { $_ })
$normalizedExisting = @{}
foreach ($entry in $existingEntries) {
    $normalizedExisting[$entry.TrimEnd('\').ToLowerInvariant()] = $true
}

$entriesToAdd = foreach ($entry in $projectPathEntries) {
    $key = $entry.TrimEnd('\').ToLowerInvariant()
    if (-not $normalizedExisting.ContainsKey($key)) {
        $normalizedExisting[$key] = $true
        $entry
    }
}

$env:Path = (@($entriesToAdd) + $existingEntries) -join ';'

Write-Host "Gnirehtet NG development environment activated for this PowerShell process."
Write-Host "Repository:       $repoRoot"
Write-Host "Rust home:       $env:RUSTUP_HOME"
Write-Host "Cargo home:      $env:CARGO_HOME"
Write-Host "Java home:       $env:JAVA_HOME"
Write-Host "Android SDK:     $env:ANDROID_SDK_ROOT"
Write-Host "Gradle user home:$env:GRADLE_USER_HOME"
