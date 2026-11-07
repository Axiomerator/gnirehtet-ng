# Ensures the host ADB server daemon is running WITHOUT making the calling shell's
# captured output hang.
#
# Observed failure mode: any shell command that cold-starts the ADB server daemon (via a
# child `adb` process) never returns to the capturing tool. Spawning through WMI attaches
# the daemon to the WMI provider service tree instead of the caller's process tree, so the
# caller's stdout capture is never held open by the long-lived daemon.
#
# Usage: call this whenever the ADB server may be down (e.g. after the controller's
# GUI-exit cleanup ran `adb kill-server`). Commands that only talk to a warm daemon are safe.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$adb = Join-Path $repositoryRoot '.tools\android-sdk\platform-tools\adb.exe'
if (-not (Test-Path -LiteralPath $adb)) {
    throw "adb.exe not found at $adb"
}

$commandLine = '"' + $adb + '" start-server'
$result = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine = $commandLine
} -ErrorAction Stop
if ($result.ReturnValue -ne 0) {
    throw "WMI process creation failed with return value $($result.ReturnValue)"
}
Write-Output "ADB server daemon requested via WMI (PID $($result.ProcessId))"
