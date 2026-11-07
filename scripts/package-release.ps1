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
$releaseApk = Join-Path $repositoryRoot 'app\build\outputs\apk\release\gnirehtet-ng-release.apk'
$bundle = Join-Path $repositoryRoot "out\release\GnirehtetNG-$version"
$zip = Join-Path $repositoryRoot "out\release\GnirehtetNG-$version-windows-x64.zip"

foreach ($required in @(
        (Join-Path $m2 'gnirehtet-core.exe'),
        (Join-Path $m2 'gnirehtet-cli.exe'),
        (Join-Path $m2 'gnirehtet-ng.exe'),
        (Join-Path $guiDir 'GnirehtetNg.Gui.exe'),
        (Join-Path $guiDir 'app-icon.ico'),
        $releaseApk,
        (Join-Path $adbTools 'adb.exe'),
        (Join-Path $adbTools 'AdbWinApi.dll'),
        (Join-Path $adbTools 'AdbWinUsbApi.dll'),
        (Join-Path $adbTools 'NOTICE.txt'))) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "Required release artifact is missing: $required"
    }
}

$cargoRegistry = Join-Path $repositoryRoot '.tools\cargo\registry\src'
$lockfile = Get-Content -Raw -LiteralPath (Join-Path $repositoryRoot 'windows\Cargo.lock')
$slintVersion = [regex]::Match($lockfile, 'name = "slint"\r?\nversion = "([^"]+)"').Groups[1].Value
$slintLicense = Get-ChildItem -Path "$cargoRegistry\*\slint-$slintVersion\LICENSES\LicenseRef-Slint-Royalty-free-2.0.md" -File | Select-Object -First 1
if (-not $slintLicense) { throw 'Build the desktop app to restore the pinned Slint license.' }
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

foreach ($artifact in @('GnirehtetNg.Gui.exe', 'app-icon.ico')) {
    Copy-Item -LiteralPath (Join-Path $guiDir $artifact) -Destination $bundle -Force
}
foreach ($artifact in @('gnirehtet-core.exe', 'gnirehtet-cli.exe', 'gnirehtet-ng.exe')) {
    Copy-Item -LiteralPath (Join-Path $m2 $artifact) -Destination $bundle -Force
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
New-Item -ItemType Directory -Force -Path $licenseDirectory | Out-Null
Copy-Item -LiteralPath $slintLicense.FullName -Destination $licenseDirectory
foreach ($notice in @('Mochi-Third-Party-Notices.md', 'LICENSE-Cresto.txt', 'NOTICE-Cresto.txt')) {
    Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\licenses\$notice") -Destination $licenseDirectory
}

# Use the x64 redistributable files from the build's Visual Studio installation.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$runtime = & $vswhere -latest -version '[17.0,18.0)' -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
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
