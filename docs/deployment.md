# Portable deployment and daily use

## First run

Extract the Windows x64 portable ZIP from the release assets. Keep the files together. Launch `GnirehtetNg.Gui.exe` or `run.bat`. The folder contains the GUI, Rust controller, CLI and relay, signed Android APK, ADB files, and Windows runtime files.

Enable USB debugging on the phone and authorize the computer. If multiple phones are connected, select the intended device. Click **Start**, then approve the VPN dialog on the phone. The controller installs the bundled app when required. Android allows one active VPN per user, so close another VPN first if it occupies that slot.

## Controls and cleanup

- **Stop** ends the current session. **Restart** tears it down and establishes it again.
- Closing the GUI exits by default. If the GUI started the controller, exiting cleans up the managed session and stops the ADB server it uses. This can interrupt other tools using that server. Enable the tray setting if closing the window should keep the session active.
- Delete the extracted folder to remove the desktop files. Local settings and logs are stored under `%LOCALAPPDATA%\GnirehtetNg`. Remove the Android app through Android Settings when you no longer need it.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| The phone is missing or unauthorized | Use a data-capable USB cable, unlock the phone, and accept the USB debugging authorization. Check the Devices page. |
| VPN consent timed out | Accept the consent dialog and click **Start** again if Windows has returned an error. |
| The VPN connects but a specific app still reports offline | Test a browser with Wi-Fi and mobile data off. Some apps require connectivity capabilities or traffic outside the IPv4 TCP/UDP path. |
| APK installation fails after switching builds | Android updates require a matching signing certificate. If you are switching from a locally signed build to a different signer, uninstall the old client first. This clears its settings. |

### HyperOS background behavior

Some HyperOS builds have stopped the VPN after the Android app moved to the background. On devices exposing the existing `MILLET_NO_RESTRICT_APP` system setting, Windows **Start** and **Restart** try to add this Android package to the list over ADB. The Android dashboard also offers Android's battery-optimization exemption. Background behavior varies by ROM and version.

Use **Diagnostics** in Windows to export a local ZIP when reporting a problem. It can include device identifiers, system logs, and network details. Review it before attaching it to a public issue.
