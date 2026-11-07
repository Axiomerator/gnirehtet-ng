# Gnirehtet NG

_Modified for Gnirehtet NG by the Gnirehtet NG contributors._

Gnirehtet NG shares a Windows computer's internet connection with an Android phone over USB. The phone uses Android's VPN API, and the computer forwards IPv4 TCP and UDP traffic through ADB. Neither device needs root access.

## Download and run

1. On the repository's **Releases** page, download `GnirehtetNG-<version>-windows-x64.zip` from the release assets and extract the full folder.
2. On the phone, enable USB debugging. Connect it with a data cable, unlock it, and approve the computer's USB debugging authorization.
3. Run `GnirehtetNg.Gui.exe` or `run.bat`, select the phone, and click **Start**.
4. Approve Android's VPN prompt. The controller installs the bundled Android app when required.

The portable build targets **Windows 11 x64**. The Android app targets API 36 and requires Android 6.0 (API 23) or later. Compatibility with individual Android apps and vendor ROMs varies. To check USB-only connectivity, turn off Wi-Fi and mobile data on the phone and open a web page.

The desktop dashboard shows connection status, traffic, logs, and diagnostics. Both dashboards offer System, Light, and Dark themes. Closing the window exits by default and tears down a session managed by that window's controller. Enable **Close to notification area** to keep it running.

See the [user guide](docs/deployment.md) for troubleshooting, Android updates, and HyperOS behavior.

## Source layout

| Path | Purpose |
| --- | --- |
| `windows/` | Slint and Rust desktop dashboard |
| `relay-rust/` | Windows controller, CLI, and packet relay |
| `app/` | Android VPN service and Mochi Compose dashboard |
| `relay-java/` | Inherited Java relay and its tests |
| `scripts/` | Development setup, builds, tests, and portable packaging |

See [DEVELOP.md](DEVELOP.md) for building, testing, and preparing a release. See the [system overview](docs/architecture/system.md) for component boundaries and the [IPC reference](protocol/README.md) for the local command protocol.

## License

Gnirehtet NG is an independent fork of [Genymobile/gnirehtet](https://github.com/Genymobile/gnirehtet). See the [Apache 2.0 license](LICENSE) and [NOTICE](NOTICE).
