[CmdletBinding()]
param(
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'
$adbInterfaceGuid = '{F72FE0D4-CBCB-407D-8814-9ED673D0DD6B}'
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This script must run from an elevated PowerShell process.'
}

$adbDevices = @(Get-PnpDevice -PresentOnly | Where-Object {
        $_.InstanceId -like 'USB\VID_*&MI_*' -and $_.FriendlyName -match 'ADB'
    })
if ($adbDevices.Count -ne 1) {
    throw "Expected exactly one present USB ADB interface, found $($adbDevices.Count)."
}

$device = $adbDevices[0]
$hardwareIds = @(Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_HardwareIds').Data
if (-not ($hardwareIds | Where-Object { $_ -match '^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}' })) {
    throw "Refusing to modify an unexpected device: $($device.InstanceId)"
}

$deviceParametersPath = "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Enum\$($device.InstanceId)\Device Parameters"
if ($Remove) {
    $currentValue = (Get-ItemProperty -LiteralPath $deviceParametersPath -Name 'DeviceInterfaceGUIDs' -ErrorAction SilentlyContinue).DeviceInterfaceGUIDs
    if ($currentValue -contains $adbInterfaceGuid) {
        Remove-ItemProperty -LiteralPath $deviceParametersPath -Name 'DeviceInterfaceGUIDs'
        Write-Host "Removed ADB interface GUID from $($device.InstanceId)"
    } else {
        Write-Host 'The standard ADB interface GUID is not present; nothing to remove.'
    }
} else {
    New-ItemProperty -LiteralPath $deviceParametersPath -Name 'DeviceInterfaceGUIDs' -PropertyType MultiString `
        -Value @($adbInterfaceGuid) -Force | Out-Null
    Write-Host "Registered the standard ADB interface GUID for $($device.InstanceId)"
}

& pnputil.exe /restart-device $device.InstanceId
if ($LASTEXITCODE -ne 0) {
    throw "Could not restart ADB interface; pnputil exited with $LASTEXITCODE"
}
