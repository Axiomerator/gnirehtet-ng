# System architecture

## Control plane

The WinUI 3 GUI and `gnirehtet-cli.exe` send JSON requests to `gnirehtet-core.exe` over a local Windows named pipe. The Rust controller selects an authorized device, installs the Android client when needed, starts the relay, and creates a device-scoped ADB reverse mapping. Windows renders the controller's snapshots, traffic measurements, and structured logs.

The controller tracks device availability, APK, reverse mapping, Android process, VPN service, relay and connectivity health. Its watchdog classifies failures and applies bounded retries when auto-reconnect is enabled. If connectivity degrades while the tunnel is healthy, the controller reports the degraded status. The controller can export a local diagnostic ZIP.

The GUI starts its sibling controller with the GUI's process ID. That controller cleans up when the GUI exits. The CLI can also start a controller independently. See the [IPC reference](../../protocol/README.md) for the request envelope and commands.

## Data plane

```text
Android applications
  ↕ IPv4 TCP/UDP
Android VpnService and TUN
  ↕ raw IPv4 packets
localabstract:gnirehtet
  ↕ adb reverse over USB
Windows Rust relay, loopback port 31416 by default
  ↕ host TCP/UDP sockets
Internet or LAN
```

Android owns VPN authorization and the TUN interface. The Android activity shows status and sends start/stop requests to the service. The Rust relay retains upstream packet parsing, flow routing, TCP state handling, and backpressure.

The NG relay sends a four-byte client ID, a two-byte port, and a 256-byte NUL-padded host name before packets. The Android client tolerates a missing extension using a short read timeout. Use the matched NG client and relay shipped together. The tunnel framing is separate from the desktop IPC protocol.

## Repository boundaries

The first NG baseline is derived from [upstream commit `1eb2e58`](https://github.com/Genymobile/gnirehtet/commit/1eb2e58bc91e268dba62561d11e3bebd9b100883). Keep packet-level behavior and tests in the relay, and session policy in the controller. Windows binaries and the signed Android APK are distributed as a portable ZIP release asset.
