[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'dev-shell.ps1')

$downloads = Join-Path ([System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))) '.tools\downloads'
$rustupInit = Join-Path $downloads 'rustup-init.exe'
$checksumFile = Join-Path $downloads 'rustup-init.exe.sha256'
$rustupUrl = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
$checksumUrl = "$rustupUrl.sha256"

if (-not (Test-Path -LiteralPath $rustupInit)) {
    Write-Host 'Downloading the official rustup installer...'
    Invoke-WebRequest -UseBasicParsing -Uri $rustupUrl -OutFile $rustupInit
}

Invoke-WebRequest -UseBasicParsing -Uri $checksumUrl -OutFile $checksumFile
$expectedHash = ((Get-Content -Raw -LiteralPath $checksumFile).Trim() -split '\s+')[0].ToUpperInvariant()
$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $rustupInit).Hash.ToUpperInvariant()
if ($actualHash -ne $expectedHash) {
    throw "rustup-init.exe SHA-256 mismatch. Expected $expectedHash, got $actualHash."
}

$rustup = Join-Path $env:CARGO_HOME 'bin\rustup.exe'
if (-not (Test-Path -LiteralPath $rustup)) {
    Write-Host 'Installing the isolated stable MSVC Rust toolchain...'
    & $rustupInit -y --no-modify-path --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        throw "rustup-init.exe failed with exit code $LASTEXITCODE."
    }
}

& $rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal
if ($LASTEXITCODE -ne 0) {
    throw "rustup toolchain install failed with exit code $LASTEXITCODE."
}
& $rustup component add rustfmt clippy --toolchain stable-x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) {
    throw "rustup component add failed with exit code $LASTEXITCODE."
}

& (Join-Path $env:CARGO_HOME 'bin\rustc.exe') --version --verbose
& (Join-Path $env:CARGO_HOME 'bin\cargo.exe') --version
