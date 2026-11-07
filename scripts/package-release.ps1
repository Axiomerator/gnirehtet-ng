# Packages a self-contained Windows release bundle: GUI, controller, CLI, relay,
# signed release APK, pinned ADB, icon, and daily-use launch scripts, then zips it.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$product = ConvertFrom-StringData (Get-Content -Raw -LiteralPath (Join-Path $repositoryRoot 'config\product.properties'))
$version = $product.productVersion
$m2 = Join-Path $repositoryRoot 'out\m2'
$guiDir = Join-Path $repositoryRoot 'out\m3\x64\Release'
$adbTools = Join-Path $repositoryRoot '.tools\android-sdk\platform-tools'
$nugetPackages = Join-Path $repositoryRoot '.tools\nuget-packages'
$windowsAssets = Join-Path $repositoryRoot 'windows\GnirehtetNg.Gui\obj\project.assets.json'
$releaseApk = Join-Path $repositoryRoot 'app\build\outputs\apk\release\gnirehtet-ng-release.apk'
$bundle = Join-Path $repositoryRoot "out\release\GnirehtetNG-$version"
$zip = Join-Path $repositoryRoot "out\release\GnirehtetNG-$version-windows-x64.zip"

foreach ($required in @(
        (Join-Path $m2 'gnirehtet-core.exe'),
        (Join-Path $m2 'gnirehtet-cli.exe'),
        (Join-Path $m2 'gnirehtet-ng.exe'),
        (Join-Path $guiDir 'GnirehtetNg.Gui.exe'),
        (Join-Path $guiDir 'app-icon.ico'),
        $windowsAssets,
        $releaseApk,
        (Join-Path $adbTools 'adb.exe'),
        (Join-Path $adbTools 'AdbWinApi.dll'),
        (Join-Path $adbTools 'AdbWinUsbApi.dll'),
        (Join-Path $adbTools 'NOTICE.txt'))) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "Required release artifact is missing: $required"
    }
}

$libraries = (Get-Content -Raw -LiteralPath $windowsAssets | ConvertFrom-Json).libraries
$runtimePackages = $libraries.PSObject.Properties | Where-Object {
    $_.Name -match '^Microsoft\.(WindowsAppSDK(\.[^/]+)?|Windows\.AI\.MachineLearning|Web\.WebView2)/'
}
if (-not ($runtimePackages | Where-Object Name -Like 'Microsoft.WindowsAppSDK.Runtime/*')) {
    throw 'The Windows App SDK runtime is missing from the NuGet restore. Run scripts\build-m3.ps1 -Configuration Release.'
}
$runtimeNotices = foreach ($package in $runtimePackages) {
    $packageDir = Join-Path $nugetPackages $package.Value.path
    $documents = Get-ChildItem -LiteralPath $packageDir -File | Where-Object {
        $_.Name -match '^(LICENSE|NOTICE|ThirdPartyNotices)(\.txt)?$'
    }
    if (-not ($documents | Where-Object Name -Match '^LICENSE(\.txt)?$')) {
        throw "License file is missing for $($package.Name). Restore the Windows packages again."
    }
    [PSCustomObject]@{
        Path = $package.Value.path
        Documents = $documents
    }
}

$env:JAVA_HOME = Join-Path $repositoryRoot '.tools\jdk17'
$apkSigner = Join-Path $repositoryRoot '.tools\android-sdk\build-tools\36.0.0\apksigner.bat'
& $apkSigner verify $releaseApk
if ($LASTEXITCODE -ne 0) {
    throw 'The release APK must be signed. See DEVELOP.md for release signing.'
}

if (Test-Path -LiteralPath $bundle) {
    # Best effort: a running ADB daemon started from this folder can lock files.
    Get-ChildItem -LiteralPath $bundle -Recurse -File -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $bundle -Recurse -Directory -ErrorAction SilentlyContinue |
        Sort-Object { $_.FullName.Length } -Descending |
        Remove-Item -Force -ErrorAction SilentlyContinue
}
New-Item -ItemType Directory -Force -Path $bundle | Out-Null
$adbDir = Join-Path $bundle 'adb'
New-Item -ItemType Directory -Force -Path $adbDir | Out-Null

# The self-contained WinUI deployment needs the whole staging directory
# (runtime DLLs, .pri/.xbf resources, language folders); copy everything except
# debug symbols, linker artifacts, and the large Windows ML runtime stack that
# this app never uses. Everything else stays: XAML startup resolves SDK metadata
# and workload manifests, and trimming them has proven fragile.
$excludePattern = @(
    '\.(pdb|lib|exp)$',
    'onnxruntime\.dll$',
    'DirectML\.dll$',
    'NPUDetect\.dll$',
    'PerceptiveStreaming\.dll$'
) -join '|'
foreach ($file in (Get-ChildItem -LiteralPath $guiDir -Recurse -File)) {
    $relative = $file.FullName.Substring($guiDir.Length + 1)
    if ($relative -match $excludePattern) {
        continue
    }
    $destination = Join-Path $bundle $relative
    $destinationDir = Split-Path $destination -Parent
    New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $destination
}

Copy-Item -LiteralPath $releaseApk -Destination (Join-Path $bundle 'gnirehtet-ng.apk') -Force
Copy-Item -LiteralPath (Join-Path $adbTools 'adb.exe') -Destination $adbDir
Copy-Item -LiteralPath (Join-Path $adbTools 'AdbWinApi.dll') -Destination $adbDir
Copy-Item -LiteralPath (Join-Path $adbTools 'AdbWinUsbApi.dll') -Destination $adbDir
Copy-Item -LiteralPath (Join-Path $adbTools 'NOTICE.txt') -Destination $adbDir
foreach ($document in @('LICENSE', 'NOTICE', 'docs\deployment.md')) {
    Copy-Item -LiteralPath (Join-Path $repositoryRoot $document) -Destination $bundle
}

$licenseDirectory = Join-Path $bundle 'licenses'
foreach ($notice in $runtimeNotices) {
    $destination = Join-Path $licenseDirectory $notice.Path
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    foreach ($document in $notice.Documents) {
        Copy-Item -LiteralPath $document.FullName -Destination $destination
    }
}

# Use the x64 redistributable files from the build's Visual Studio installation.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$runtime = & $vswhere -latest -version '[17.0,18.0)' -products * `
    -requires Microsoft.Component.MSBuild Microsoft.VisualStudio.ComponentGroup.UWP.VC `
    -find 'VC\Redist\MSVC\**\x64\Microsoft.VC143.CRT\vcruntime140.dll' | Where-Object { $_ -notmatch '\\onecore\\' } | Select-Object -First 1
if (-not $runtime) {
    throw 'Install the x64 VC++ redistributable files in Visual Studio 2022.'
}
$runtimeDirectory = Split-Path -Parent $runtime
foreach ($runtimeDll in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
    Copy-Item -LiteralPath (Join-Path $runtimeDirectory $runtimeDll) -Destination $bundle
}

$launcher = @'
param()
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$env:ADB = Join-Path $root 'adb\adb.exe'
$env:GNIREHTET_APK = Join-Path $root 'gnirehtet-ng.apk'
$env:GNIREHTET_NG_RELAY = Join-Path $root 'gnirehtet-ng.exe'
$process = New-Object System.Diagnostics.Process
$process.StartInfo.FileName = Join-Path $root 'GnirehtetNg.Gui.exe'
$process.StartInfo.UseShellExecute = $true
$process.Start() | Out-Null
'@
[System.IO.File]::WriteAllText((Join-Path $bundle 'run.ps1'), $launcher, (New-Object System.Text.UTF8Encoding($false)))
$bat = "@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0run.ps1`"`r`n"
[System.IO.File]::WriteAllText((Join-Path $bundle 'run.bat'), $bat, (New-Object System.Text.UTF8Encoding($false)))

if (Test-Path -LiteralPath $zip) {
    Remove-Item -LiteralPath $zip -Force
}
Compress-Archive -Path (Join-Path $bundle '*') -DestinationPath $zip -CompressionLevel Optimal
Write-Output "Release bundle: $bundle"
Get-ChildItem -LiteralPath $bundle | Select-Object Name, Length | Format-Table -AutoSize
Write-Output "Zip: $zip"
