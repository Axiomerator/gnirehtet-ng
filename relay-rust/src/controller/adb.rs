use crate::controller::model::{AdbState, DeviceInfo, Fault};
use crate::controller::{CLIENT_ACTION_START, CLIENT_ACTION_STOP, CLIENT_ACTIVITY, CLIENT_PACKAGE};
use crate::subprocess::configure_background;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CAPTURE_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct AdbManager {
    executable: PathBuf,
    timeout: Duration,
    version_cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}

impl Default for AdbManager {
    fn default() -> Self {
        Self::new(
            std::env::var_os("ADB")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("adb")),
        )
    }
}

impl AdbManager {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            timeout: DEFAULT_TIMEOUT,
            version_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>, Fault> {
        let output = self.execute(None, ["devices", "-l"], "list_devices")?;
        let mut devices = parse_devices(&output.stdout);
        let mut cache = self
            .version_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for device in &mut devices {
            if device.adb_state != AdbState::Ready {
                continue;
            }
            if let Some(version) = cache.get(&device.serial) {
                device.android_version = Some(version.clone());
                continue;
            }
            let version = self
                .execute(
                    Some(&device.serial),
                    ["shell", "getprop", "ro.build.version.release"],
                    "device_version",
                )
                .ok()
                .map(|output| output.stdout.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(value) = version.as_ref() {
                cache.insert(device.serial.clone(), value.clone());
            }
            device.android_version = version;
        }
        Ok(devices)
    }

    pub fn select_device(&self, requested: Option<&str>) -> Result<DeviceInfo, Fault> {
        let devices = self.list_devices()?;
        if let Some(serial) = requested {
            let device = devices.into_iter().find(|device| device.serial == serial);
            return match device {
                Some(device) if device.adb_state == AdbState::Ready => Ok(device),
                Some(device) => Err(device_state_fault(&device)),
                None => Err(Fault::new(
                    "usb_disconnected",
                    format!("Android device '{serial}' is not visible to ADB"),
                    true,
                    "ADB",
                    "select_device",
                    Some(serial.to_string()),
                )),
            };
        }

        let mut ready = devices
            .iter()
            .filter(|device| device.adb_state == AdbState::Ready);
        match (ready.next(), ready.next()) {
            (Some(device), None) => Ok(device.clone()),
            (Some(_), Some(_)) => Err(Fault::new(
                "multiple_devices",
                "More than one authorized Android device is connected; specify a serial",
                false,
                "ADB",
                "select_device",
                None,
            )),
            (None, _) if devices.is_empty() => Err(Fault::new(
                "usb_disconnected",
                "No Android device is visible to ADB",
                true,
                "ADB",
                "select_device",
                None,
            )),
            (None, _) => Err(device_state_fault(&devices[0])),
        }
    }

    pub fn settings_get(&self, serial: &str, key: &str) -> Result<String, Fault> {
        let output = self.execute(
            Some(serial),
            ["shell", "settings", "--user", "0", "get", "system", key],
            "settings_get",
        )?;
        Ok(output.stdout.trim().to_string())
    }

    pub fn settings_put(&self, serial: &str, key: &str, value: &str) -> Result<(), Fault> {
        self.execute(
            Some(serial),
            [
                "shell", "settings", "--user", "0", "put", "system", key, value,
            ],
            "settings_put",
        )?;
        Ok(())
    }

    pub fn kill_server(&self) -> Result<(), Fault> {
        self.execute(None, ["kill-server"], "kill_server")
            .map(|_| ())
    }

    pub fn package_version(&self, serial: &str) -> Result<Option<String>, Fault> {
        let output = self.execute(
            Some(serial),
            ["shell", "dumpsys", "package", CLIENT_PACKAGE],
            "package_version",
        )?;
        Ok(parse_version_code(&output.stdout).map(str::to_string))
    }

    pub fn install(&self, serial: &str, apk: &Path) -> Result<(), Fault> {
        let args = vec![
            OsString::from("install"),
            OsString::from("-r"),
            apk.as_os_str().to_os_string(),
        ];
        self.execute_os(Some(serial), args, Duration::from_secs(90), "install")?;
        Ok(())
    }

    pub fn create_reverse(&self, serial: &str, port: u16) -> Result<(), Fault> {
        self.execute(
            Some(serial),
            [
                "reverse".to_string(),
                "localabstract:gnirehtet".to_string(),
                format!("tcp:{port}"),
            ],
            "create_reverse",
        )?;
        Ok(())
    }

    pub fn remove_reverse(&self, serial: &str) -> Result<(), Fault> {
        self.execute(
            Some(serial),
            ["reverse", "--remove", "localabstract:gnirehtet"],
            "remove_reverse",
        )?;
        Ok(())
    }

    pub fn reverse_is_ready(&self, serial: &str, port: u16) -> Result<bool, Fault> {
        let output = self.execute(Some(serial), ["reverse", "--list"], "list_reverse")?;
        Ok(reverse_list_contains(&output.stdout, serial, port))
    }

    pub fn start_client(
        &self,
        serial: &str,
        dns_servers: Option<&str>,
        routes: Option<&str>,
    ) -> Result<(), Fault> {
        let mut args = vec![
            "shell".to_string(),
            "am".to_string(),
            "start".to_string(),
            "-a".to_string(),
            CLIENT_ACTION_START.to_string(),
            "-n".to_string(),
            CLIENT_ACTIVITY.to_string(),
        ];
        if let Some(dns_servers) = dns_servers {
            args.extend(["--esa".into(), "dnsServers".into(), dns_servers.into()]);
        }
        if let Some(routes) = routes {
            args.extend(["--esa".into(), "routes".into(), routes.into()]);
        }
        self.execute(Some(serial), args, "start_client")?;
        Ok(())
    }

    /// Restore the VPN through the transparent control activity with the silent flag:
    /// the activity starts the service and finishes without opening the dashboard, so
    /// watchdog recoveries are invisible to the user.
    pub fn restart_client_invisibly(
        &self,
        serial: &str,
        dns_servers: Option<&str>,
        routes: Option<&str>,
    ) -> Result<(), Fault> {
        let mut args = vec![
            "shell".to_string(),
            "am".to_string(),
            "start".to_string(),
            // NEW_TASK | NO_ANIMATION | NOT_FOCUSABLE: the transparent activity runs
            // without taking input focus or showing anything on screen.
            "-f".to_string(),
            "0x10010008".to_string(),
            "-a".to_string(),
            CLIENT_ACTION_START.to_string(),
            "-n".to_string(),
            crate::controller::CLIENT_TRANSPARENT_ACTIVITY.to_string(),
            "--ez".to_string(),
            "silent".to_string(),
            "true".to_string(),
        ];
        if let Some(dns_servers) = dns_servers {
            args.extend(["--esa".into(), "dnsServers".into(), dns_servers.into()]);
        }
        if let Some(routes) = routes {
            args.extend(["--esa".into(), "routes".into(), routes.into()]);
        }
        self.execute(Some(serial), args, "restart_client_invisibly")?;
        Ok(())
    }

    pub fn stop_client(&self, serial: &str) -> Result<(), Fault> {
        self.execute(
            Some(serial),
            [
                "shell",
                "am",
                "start",
                "-a",
                CLIENT_ACTION_STOP,
                "-n",
                CLIENT_ACTIVITY,
            ],
            "stop_client",
        )?;
        Ok(())
    }

    pub fn client_pid(&self, serial: &str) -> Result<Option<u32>, Fault> {
        let args = vec![
            OsString::from("-s"),
            OsString::from(serial),
            OsString::from("shell"),
            OsString::from("pidof"),
            OsString::from(CLIENT_PACKAGE),
        ];
        let output = run_program(&self.executable, &args, self.timeout).map_err(|error| {
            map_process_error(error, "client_pid", Some(serial.to_string()), "ADB")
        })?;
        // Android's pidof returns status 1 with no output when a process is absent.
        if !output.status.success() && output.stdout.trim().is_empty() {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(Fault::new(
                classify_adb_failure(&output.stderr),
                format!(
                    "ADB operation 'client_pid' failed: {}",
                    concise_output(&output.stderr, &output.stdout)
                ),
                true,
                "ADB",
                "client_pid",
                Some(serial.to_string()),
            ));
        }
        Ok(output
            .stdout
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok()))
    }

    pub fn service_is_active(&self, serial: &str) -> Result<bool, Fault> {
        // Ground truth comes from the client's own dump() output: the service prints
        // "GNIREHTET_NG_SERVICE\nrunning=true" only while its VPN object is alive.
        // Service records and the lingering tun0 interface both survive a background
        // kill, so neither is trustworthy on its own.
        let component = format!(
            "{}/com.genymobile.gnirehtet.GnirehtetService",
            CLIENT_PACKAGE
        );
        let output = self.execute(
            Some(serial),
            ["shell", "dumpsys", "activity", "service", &component],
            "service_health",
        )?;
        Ok(
            output.stdout.contains("GNIREHTET_NG_SERVICE")
                && output.stdout.contains("running=true"),
        )
    }

    /// Restart the client VPN service directly, without launching the dashboard activity,
    /// so watchdog recoveries stay invisible to the user.
    pub fn restart_client_service(
        &self,
        serial: &str,
        dns_servers: Option<&str>,
        routes: Option<&str>,
    ) -> Result<(), Fault> {
        let mut args = vec![
            "shell".to_string(),
            "am".to_string(),
            "start-foreground-service".to_string(),
            "-a".to_string(),
            crate::controller::CLIENT_ACTION_START_VPN.to_string(),
            "-n".to_string(),
            crate::controller::CLIENT_SERVICE.to_string(),
        ];
        if let Some(dns_servers) = dns_servers {
            args.extend(["--esa".into(), "dnsServers".into(), dns_servers.into()]);
        }
        if let Some(routes) = routes {
            args.extend(["--esa".into(), "routes".into(), routes.into()]);
        }
        let output = self.execute(Some(serial), args, "restart_client_service")?;
        // The am command prints this error while still exiting successfully when the
        // shell lacks the permission to start a VpnService; treat it as a failure so
        // the recovery falls back to the activity path instead of looping.
        if output.stdout.contains("Requires permission")
            || output.stderr.contains("Requires permission")
        {
            return Err(Fault::new(
                "client_start_blocked",
                "Shell cannot start the VPN service directly; the activity path is required",
                true,
                "ANDROID",
                "restart_client_service",
                Some(serial.to_string()),
            ));
        }
        Ok(())
    }

    pub fn vpn_health(&self, serial: &str) -> Result<(bool, bool), Fault> {
        let output = self.execute(
            Some(serial),
            ["shell", "dumpsys", "connectivity"],
            "vpn_health",
        )?;
        Ok(parse_vpn_health(&output.stdout, CLIENT_PACKAGE))
    }

    pub fn application_exit_info(&self, serial: &str) -> Result<String, Fault> {
        Ok(self
            .execute(
                Some(serial),
                ["shell", "dumpsys", "activity", "exit-info", CLIENT_PACKAGE],
                "application_exit_info",
            )?
            .stdout)
    }

    pub fn shell_capture<I, S>(
        &self,
        serial: &str,
        args: I,
        operation: &str,
    ) -> Result<String, Fault>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut all_args = vec![OsString::from("shell")];
        all_args.extend(args.into_iter().map(Into::into));
        Ok(self
            .execute_os(Some(serial), all_args, self.timeout, operation)?
            .stdout)
    }

    pub fn execute<I, S>(
        &self,
        serial: Option<&str>,
        args: I,
        operation: &str,
    ) -> Result<CommandOutput, Fault>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.execute_os(serial, args, self.timeout, operation)
    }

    fn execute_os<I, S>(
        &self,
        serial: Option<&str>,
        args: I,
        timeout: Duration,
        operation: &str,
    ) -> Result<CommandOutput, Fault>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut adb_args = Vec::new();
        if let Some(serial) = serial {
            adb_args.extend([OsString::from("-s"), OsString::from(serial)]);
        }
        adb_args.extend(args.into_iter().map(Into::into));
        let output = run_program(&self.executable, &adb_args, timeout).map_err(|error| {
            map_process_error(error, operation, serial.map(str::to_string), "ADB")
        })?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Fault::new(
                classify_adb_failure(&output.stderr),
                format!(
                    "ADB operation '{operation}' failed: {}",
                    concise_output(&output.stderr, &output.stdout)
                ),
                true,
                "ADB",
                operation,
                serial.map(str::to_string),
            )
            .with_detail("exit_code", output.status.code().unwrap_or(-1)))
        }
    }
}

#[derive(Debug)]
pub enum ProcessError {
    Spawn(std::io::Error),
    Wait(std::io::Error),
    Timeout(Duration),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "process spawn failed: {error}"),
            Self::Wait(error) => write!(formatter, "process wait failed: {error}"),
            Self::Timeout(timeout) => {
                write!(formatter, "process exceeded {} ms", timeout.as_millis())
            }
        }
    }
}

pub fn run_program(
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
) -> Result<CommandOutput, ProcessError> {
    let started = Instant::now();
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_background(&mut command);
    let mut child = command.spawn().map_err(ProcessError::Spawn)?;

    let stdout = child.stdout.take().expect("piped stdout missing");
    let stderr = child.stderr.take().expect("piped stderr missing");
    let stdout_reader = thread::spawn(move || read_capped(stdout));
    let stderr_reader = thread::spawn(move || read_capped(stderr));

    let status = loop {
        match child.try_wait().map_err(ProcessError::Wait)? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::Timeout(timeout));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(CommandOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        elapsed: started.elapsed(),
    })
}

fn read_capped<R: Read>(mut stream: R) -> Vec<u8> {
    let mut result = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) if result.len() < MAX_CAPTURE_BYTES => {
                let keep = count.min(MAX_CAPTURE_BYTES - result.len());
                result.extend_from_slice(&buffer[..keep]);
            }
            Ok(_) => {}
        }
    }
    result
}

fn map_process_error(
    error: ProcessError,
    operation: &str,
    device_id: Option<String>,
    component: &str,
) -> Fault {
    match error {
        ProcessError::Spawn(error) if error.kind() == std::io::ErrorKind::NotFound => Fault::new(
            "adb_not_found",
            "ADB executable was not found",
            false,
            component,
            operation,
            device_id,
        ),
        ProcessError::Spawn(error) | ProcessError::Wait(error) => Fault::new(
            "process_io_error",
            format!("Could not execute {operation}: {error}"),
            true,
            component,
            operation,
            device_id,
        ),
        ProcessError::Timeout(timeout) => Fault::new(
            "adb_timeout",
            format!(
                "ADB operation '{operation}' exceeded {} ms",
                timeout.as_millis()
            ),
            true,
            component,
            operation,
            device_id,
        )
        .with_detail("timeout_ms", timeout.as_millis() as u64),
    }
}

fn classify_adb_failure(stderr: &str) -> &'static str {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("unauthorized") {
        "adb_unauthorized"
    } else if lower.contains("offline") {
        "device_offline"
    } else if lower.contains("no devices") || lower.contains("not found") {
        "usb_disconnected"
    } else if lower.contains("cannot connect") || lower.contains("daemon") {
        "adb_server_unavailable"
    } else {
        "adb_command_failed"
    }
}

fn concise_output(stderr: &str, stdout: &str) -> String {
    let value = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    value.chars().take(512).collect()
}

fn device_state_fault(device: &DeviceInfo) -> Fault {
    let (code, message, retryable) = match device.adb_state {
        AdbState::Unauthorized => (
            "adb_unauthorized",
            "Authorize this computer on the Android device",
            true,
        ),
        AdbState::Offline => ("device_offline", "The Android device is offline", true),
        _ => ("device_not_ready", "The Android device is not ready", true),
    };
    Fault::new(
        code,
        message,
        retryable,
        "ADB",
        "select_device",
        Some(device.serial.clone()),
    )
}

pub fn parse_devices(output: &str) -> Vec<DeviceInfo> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let raw_state = parts.next()?;
            let adb_state = match raw_state {
                "device" => AdbState::Ready,
                "unauthorized" => AdbState::Unauthorized,
                "offline" => AdbState::Offline,
                _ => AdbState::UsbFound,
            };
            let mut product = None;
            let mut model = None;
            let mut device = None;
            let mut transport_id = None;
            for value in parts {
                if let Some((key, value)) = value.split_once(':') {
                    match key {
                        "product" => product = Some(value.to_string()),
                        "model" => model = Some(value.replace('_', " ")),
                        "device" => device = Some(value.to_string()),
                        "transport_id" => transport_id = Some(value.to_string()),
                        _ => {}
                    }
                }
            }
            Some(DeviceInfo {
                serial: serial.to_string(),
                adb_state,
                product,
                model,
                device,
                transport_id,
                android_version: None,
            })
        })
        .collect()
}

pub fn parse_version_code(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("versionCode=")
            .and_then(|value| value.split_whitespace().next())
    })
}

fn is_adb_transport(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "usb" | "usbffs" | "tcpip" | "cdc" | "local" | "any"
    )
}

pub fn reverse_list_contains(output: &str, serial: &str, port: u16) -> bool {
    let remote = format!("tcp:{port}");
    output.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.contains(&"localabstract:gnirehtet")
            && fields.contains(&remote.as_str())
            && (fields.len() == 2
                || fields.first() == Some(&serial)
                || fields
                    .first()
                    .map_or(false, |value| is_adb_transport(value)))
    })
}

pub fn parse_vpn_health(output: &str, package: &str) -> (bool, bool) {
    let lines: Vec<_> = output.lines().collect();
    let package_index = match lines.iter().position(|line| line.contains(package)) {
        Some(index) => index,
        None => return (false, false),
    };
    let start = package_index.saturating_sub(40);
    let end = (package_index + 80).min(lines.len());
    let section = lines[start..end].join("\n");
    // Only the network header of an actual VPN transport starts with "ni{VPN";
    // mobile/Wi-Fi blocks read "ni{MOBILE", "ni{WIFI" and must not match.
    let vpn = section.contains("ni{VPN");
    let validated = vpn && section.contains("VALIDATED");
    (vpn, validated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_adb_device_states_and_metadata() {
        let input = "List of devices attached\nready-1 device product:aurorapro model:24031PN0DC device:aurora transport_id:7\nphone-2 unauthorized transport_id:8\nphone-3 offline\n\n";
        let devices = parse_devices(input);
        assert_eq!(3, devices.len());
        assert_eq!(AdbState::Ready, devices[0].adb_state);
        assert_eq!(Some("24031PN0DC"), devices[0].model.as_deref());
        assert_eq!(AdbState::Unauthorized, devices[1].adb_state);
        assert_eq!(AdbState::Offline, devices[2].adb_state);
    }

    #[test]
    fn verifies_serial_scoped_and_legacy_reverse_output() {
        assert!(reverse_list_contains(
            "serial-1 localabstract:gnirehtet tcp:31416\n",
            "serial-1",
            31416
        ));
        assert!(reverse_list_contains(
            "localabstract:gnirehtet tcp:31416\n",
            "serial-1",
            31416
        ));
        assert!(!reverse_list_contains(
            "serial-2 localabstract:gnirehtet tcp:31416\n",
            "serial-1",
            31416
        ));
    }

    #[test]
    fn accepts_adb_37_transport_prefixed_reverse_output() {
        assert!(reverse_list_contains(
            "UsbFfs localabstract:gnirehtet tcp:31416\n",
            "df6bfa8a",
            31416
        ));
        assert!(reverse_list_contains(
            "Tcpip localabstract:gnirehtet tcp:31416\n",
            "df6bfa8a",
            31416
        ));
        assert!(!reverse_list_contains(
            "UsbFfs localabstract:gnirehtet tcp:31417\n",
            "df6bfa8a",
            31416
        ));
    }

    #[test]
    fn parses_vpn_validation_near_our_package() {
        let dump = "NetworkAgentInfo{network{100} handle{0} ni{VPN CONNECTED extra: tunnel}\n  VALIDATED\n  owner=org.gnirehtetng.client\n";
        assert_eq!((true, true), parse_vpn_health(dump, CLIENT_PACKAGE));
        assert_eq!((false, false), parse_vpn_health(dump, "missing.package"));
        // mobile blocks mention "NOT_VPN" and must never count as a VPN
        let mobile = "NetworkAgentInfo{network{101} handle{0} ni{MOBILE[LTE] CONNECTED}\n  NOT_VPN VALIDATED\n  owner=org.gnirehtetng.client\n";
        assert_eq!((false, false), parse_vpn_health(mobile, CLIENT_PACKAGE));
    }

    #[test]
    fn subprocess_deadline_terminates_a_hung_command() {
        let args = [
            OsString::from("-NoProfile"),
            OsString::from("-Command"),
            OsString::from("Start-Sleep -Milliseconds 500"),
        ];
        let result = run_program(
            Path::new("powershell.exe"),
            &args,
            Duration::from_millis(50),
        );
        assert!(matches!(result, Err(ProcessError::Timeout(_))));
    }

    #[test]
    fn adb_failure_classification_preserves_actionable_states() {
        assert_eq!(
            "adb_unauthorized",
            classify_adb_failure("device unauthorized")
        );
        assert_eq!("device_offline", classify_adb_failure("device offline"));
        assert_eq!(
            "adb_server_unavailable",
            classify_adb_failure("cannot connect to daemon")
        );
        assert_eq!(
            "adb_command_failed",
            classify_adb_failure("permission denied")
        );
    }
}
