# Gnirehtet NG local IPC v1

The Windows controller listens on `\\.\pipe\gnirehtet-ng-v1`. The pipe rejects remote clients and restricts access to the controller user's SID. Messages are UTF-8 JSON objects delimited by a newline, with a 64 KiB limit in each direction.

```json
{"version":1,"id":"request-id","type":"get_status","payload":{}}
```

The response echoes `id`. Success uses `type: "result"`. Failure uses `type: "error"` with a fault containing `code`, `message`, `retryable`, `component`, and `operation`. Clients can use snapshot `revision` to discard stale results. The desktop client polls for status, device lists, and incremental logs.

| CLI | IPC type |
| --- | --- |
| `status` | `get_status` or `status` |
| `start` | `start` |
| `stop` | `stop` |
| `restart` | `restart` |
| `devices` | `get_devices` or `devices` |
| `logs` | `get_logs` or `logs` |
| `diagnose` | `export_diagnostics` or `diagnose` |
| `metrics` | `get_metrics` |
| `shutdown-core` | `shutdown_core` |
| `hyperos-status [SERIAL]` | `hyperos_status` |
| `hyperos-protect [SERIAL]` | `hyperos_protect` |
| `hyperos-unprotect [SERIAL]` | `hyperos_unprotect` |

`shutdown_core` stops the controller-owned session and exits the core. The HyperOS commands inspect or update the vendor-specific no-restriction list. Specify a device serial when working with multiple devices.

Metrics include lifecycle and recovery counters, raw IPv4 byte and packet totals, and sampled transfer rates. See [ipc-v1.schema.json](ipc-v1.schema.json) for the request envelope.
