[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [switch]$SkipM2Build
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$solution = Join-Path $repositoryRoot 'windows\GnirehtetNg.Windows.sln'
$packages = Join-Path $repositoryRoot '.tools\nuget-packages'
$nuget = Join-Path $repositoryRoot '.tools\nuget\nuget.exe'
$m2Artifacts = Join-Path $repositoryRoot 'out\m2'
$output = Join-Path $repositoryRoot "out\m3\x64\$Configuration"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE."
    }
}

Push-Location $repositoryRoot
try {
    . (Join-Path $PSScriptRoot 'dev-shell.ps1')

    if (-not $SkipM2Build) {
        Invoke-Checked -Description 'M2 controller and relay build' -Command {
            & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'build-m2.ps1')
        }
    }
    foreach ($artifact in @('gnirehtet-core.exe', 'gnirehtet-cli.exe', 'gnirehtet-ng.exe', 'gnirehtet-ng.apk')) {
        if (-not (Test-Path -LiteralPath (Join-Path $m2Artifacts $artifact))) {
            throw "Required M2 artifact is missing: $artifact"
        }
    }
    if (-not (Test-Path -LiteralPath $nuget)) {
        $nugetVersion = '7.9.0'
        $nugetUri = "https://dist.nuget.org/win-x86-commandline/v$nugetVersion/nuget.exe"
        Write-Host "Downloading NuGet CLI $nugetVersion..."
        Invoke-WebRequest -UseBasicParsing -Uri $nugetUri -OutFile $nuget
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $nuget
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft') {
        throw "NuGet CLI signature is not a valid Microsoft signature: $($signature.Status)"
    }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio Installer vswhere.exe was not found.'
    }
    $msbuild = & $vswhere -latest -version '[17.0,18.0)' -products * `
        -requires Microsoft.Component.MSBuild Microsoft.VisualStudio.ComponentGroup.UWP.VC `
        -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
    if (-not $msbuild) {
        throw 'VS 2022 MSBuild with the UWP C++ component group was not found.'
    }

    Invoke-Checked -Description 'WinUI package restore' -Command {
        & $nuget restore $solution `
            -MSBuildPath (Split-Path -Parent $msbuild) `
            -PackagesDirectory $packages `
            -Source 'https://api.nuget.org/v3/index.json' `
            -NonInteractive -DirectDownload -NoHttpCache -Verbosity quiet
    }
    Invoke-Checked -Description 'M3 WinUI build' -Command {
        & $msbuild $solution /m /t:Rebuild `
            "/p:Configuration=$Configuration" /p:Platform=x64 `
            "/p:RestorePackagesPath=$packages" /verbosity:minimal
    }

    foreach ($artifact in @('gnirehtet-core.exe', 'gnirehtet-cli.exe', 'gnirehtet-ng.exe', 'gnirehtet-ng.apk')) {
        Copy-Item -LiteralPath (Join-Path $m2Artifacts $artifact) -Destination $output -Force
    }
    $icon = Join-Path $repositoryRoot 'assets\app-icon.ico'
    if (-not (Test-Path -LiteralPath $icon)) {
        throw "Required icon asset is missing: $icon"
    }
    Copy-Item -LiteralPath $icon -Destination (Join-Path $output 'app-icon.ico') -Force

    Invoke-Checked -Description 'M3 no-device runtime smoke test' -Command {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
            (Join-Path $PSScriptRoot 'test-m3-offline.ps1') -Configuration $Configuration
    }

    Write-Host ''
    Write-Host "M3 $Configuration development artifacts are ready:"
    Get-ChildItem -LiteralPath $output -Filter 'GnirehtetNg.Gui.exe' |
        Select-Object Name, Length, LastWriteTime | Format-Table -AutoSize
} finally {
    Pop-Location
}
