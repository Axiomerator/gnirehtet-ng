[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'dev-shell.ps1')

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
$gradleVersion = '8.13'
$distributionName = "gradle-$gradleVersion-bin"
$distributionUrl = "https://downloads.gradle.org/distributions/$distributionName.zip"
$expectedSha256 = '20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78'
$archive = Join-Path $repoRoot ".tools\downloads\$distributionName.zip"

function Assert-WorkspacePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = [System.IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a path outside the repository: $resolved"
    }
    return $resolved
}

if (-not (Test-Path -LiteralPath $archive)) {
    Write-Host "Downloading Gradle $gradleVersion from the official distribution service..."
    Invoke-WebRequest -UseBasicParsing -Uri $distributionUrl -OutFile $archive
}

$actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
    throw "Gradle archive SHA-256 mismatch. Expected $expectedSha256, got $actualSha256."
}

# This key is Gradle Wrapper's stable base-36 MD5 mapping for the pinned URL in
# gradle-wrapper.properties. Keeping it explicit avoids invoking an untrusted or
# incomplete distribution merely to discover its cache directory.
$wrapperCacheKey = '928fl043lon0uu06jzrrnc41m'
$cacheRoot = Assert-WorkspacePath (Join-Path $env:GRADLE_USER_HOME "wrapper\dists\$distributionName\$wrapperCacheKey")
$expandedDistribution = Join-Path $cacheRoot "gradle-$gradleVersion"
$okMarker = Join-Path $cacheRoot "$distributionName.zip.ok"

New-Item -ItemType Directory -Force -Path $cacheRoot | Out-Null
foreach ($staleFile in @(
    (Join-Path $cacheRoot "$distributionName.zip.part"),
    (Join-Path $cacheRoot "$distributionName.zip.lck")
)) {
    if (Test-Path -LiteralPath $staleFile) {
        Remove-Item -Force -LiteralPath $staleFile
    }
}

if (-not (Test-Path -LiteralPath (Join-Path $expandedDistribution 'bin\gradle.bat'))) {
    if (Test-Path -LiteralPath $expandedDistribution) {
        $validatedDistribution = Assert-WorkspacePath $expandedDistribution
        Remove-Item -Recurse -Force -LiteralPath $validatedDistribution
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $cacheRoot
}

New-Item -ItemType File -Force -Path $okMarker | Out-Null
& (Join-Path $expandedDistribution 'bin\gradle.bat') --version
if ($LASTEXITCODE -ne 0) {
    throw "The verified Gradle distribution failed with exit code $LASTEXITCODE."
}
