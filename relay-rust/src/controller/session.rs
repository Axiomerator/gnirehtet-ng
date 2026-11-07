use crate::controller::adb::{
    run_program, AdbManager, PhoneAction, PhoneRequest, PhoneRequests, ProcessError,
};
use crate::controller::model::{
    AdbState, ApkState, ControllerSnapshot, Fault, HealthState, SessionState, SharedTraffic,
    TrafficTotals,
};
use crate::controller::structured_log::{emit, LogEvent, LogLevel, SharedLog};
use crate::controller::REQUIRED_APK_VERSION_CODE;
use crate::subprocess::configure_background;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RELAY_START_DEADLINE: Duration = Duration::from_secs(5);
const CLIENT_START_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    pub serial: Option<String>,
    pub port: u16,
    pub dns_servers: Option<String>,
    pub routes: Option<String>,
    pub auto_reconnect: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            serial: None,
            port: 31416,
            dns_servers: None,
            routes: None,
            auto_reconnect: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum RecoveryAction {
    WaitForAdb(Fault),
    RebuildReverse(Fault),
    RestartClient(Fault),
    RestartRelay(Fault),
    DiagnoseInternet(Fault),
}

impl RecoveryAction {
    pub fn fault(&self) -> &Fault {
        match self {
            Self::WaitForAdb(fault)
            | Self::RebuildReverse(fault)
            | Self::RestartClient(fault)
            | Self::RestartRelay(fault)
            | Self::DiagnoseInternet(fault) => fault,
        }
    }
}

pub struct DeviceSession {
    pub config: SessionConfig,
    pub serial: String,
    relay: Option<Child>,
    relay_executable: PathBuf,
    owned_reverse: bool,
    client_started: bool,
    relay_disconnect_ticks: u8,
    log: SharedLog,
    traffic: SharedTraffic,
}

impl DeviceSession {
    pub fn start(
        config: SessionConfig,
        adb: &AdbManager,
        apk_path: &Path,
        relay_executable: &Path,
        snapshot: &mut ControllerSnapshot,
        log: SharedLog,
        traffic: SharedTraffic,
        replace_phone_request: bool,
    ) -> Result<Self, Fault> {
        snapshot.force_transition(SessionState::Preparing);
        snapshot.relay_port = config.port;
        snapshot.auto_reconnect = config.auto_reconnect;
        let device = adb.select_device(config.serial.as_deref())?;
        let serial = device.serial.clone();
        snapshot.adb_state = AdbState::Ready;
        snapshot.selected_device = Some(device);
        emit(
            &log,
            LogEvent::new(
                LogLevel::Info,
                "DEVICE",
                Some(serial.clone()),
                "device_selected",
                "Authorized Android device selected",
            ),
        );

        let mut session = Self {
            config,
            serial,
            relay: None,
            relay_executable: relay_executable.to_path_buf(),
            owned_reverse: false,
            client_started: false,
            relay_disconnect_ticks: 0,
            log,
            traffic,
        };

        let result = session.start_transaction(adb, apk_path, snapshot, replace_phone_request);
        if let Err(fault) = result {
            session.rollback(adb, snapshot);
            return Err(fault);
        }
        Ok(session)
    }

    fn start_transaction(
        &mut self,
        adb: &AdbManager,
        apk_path: &Path,
        snapshot: &mut ControllerSnapshot,
        replace_phone_request: bool,
    ) -> Result<(), Fault> {
        match adb.package_version(&self.serial)? {
            Some(version) if version == REQUIRED_APK_VERSION_CODE => {
                snapshot.apk_state = ApkState::Ready;
            }
            _ => {
                snapshot.apk_state = ApkState::Missing;
                emit(
                    &self.log,
                    LogEvent::new(
                        LogLevel::Info,
                        "ANDROID",
                        Some(self.serial.clone()),
                        "apk_install_started",
                        "Required Android client is missing or outdated; installing",
                    ),
                );
                if !apk_path.is_file() {
                    return Err(Fault::new(
                        "apk_missing",
                        format!("Android APK was not found at {}", apk_path.display()),
                        false,
                        "ANDROID",
                        "install",
                        Some(self.serial.clone()),
                    ));
                }
                adb.install(&self.serial, apk_path)?;
                snapshot.apk_state = ApkState::Ready;
            }
        }

        snapshot.force_transition(SessionState::Starting);
        self.spawn_relay(snapshot)?;
        self.ensure_reverse(adb, snapshot)?;
        snapshot.force_transition(SessionState::ClientStarting);
        adb.start_client(
            &self.serial,
            self.config.dns_servers.as_deref(),
            self.config.routes.as_deref(),
            replace_phone_request,
        )?;
        self.client_started = true;
        self.await_client_health(adb, snapshot)
    }

    fn spawn_relay(&mut self, snapshot: &mut ControllerSnapshot) -> Result<(), Fault> {
        let listener_guard = TcpListener::bind(("0.0.0.0", self.config.port)).map_err(|error| {
            Fault::new(
                "relay_bind_failed",
                format!(
                    "Relay port {} is unavailable before launch: {error}",
                    self.config.port
                ),
                true,
                "RELAY",
                "bind",
                Some(self.serial.clone()),
            )
        })?;
        drop(listener_guard);

        let port = self.config.port.to_string();
        self.traffic
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reset();
        let mut command = Command::new(&self.relay_executable);
        command
            .args(["relay", "-p", &port])
            .env("GNIREHTET_NG_EMIT_TRAFFIC_METRICS", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_background(&mut command);
        let mut child = command.spawn().map_err(|error| {
            Fault::new(
                "relay_start_failed",
                format!("Could not start relay process: {error}"),
                true,
                "RELAY",
                "spawn",
                Some(self.serial.clone()),
            )
        })?;
        let pid = child.id();
        if let Some(stdout) = child.stdout.take() {
            stream_relay_output(
                stdout,
                self.log.clone(),
                self.serial.clone(),
                LogLevel::Info,
                Some(self.traffic.clone()),
            );
        }
        if let Some(stderr) = child.stderr.take() {
            stream_relay_output(
                stderr,
                self.log.clone(),
                self.serial.clone(),
                LogLevel::Warn,
                None,
            );
        }
        self.relay = Some(child);
        snapshot.relay_pid = Some(pid);

        let deadline = Instant::now() + RELAY_START_DEADLINE;
        loop {
            if self.relay_exited()?.is_some() {
                return Err(Fault::new(
                    "relay_exited",
                    "Relay process exited before its listening endpoint was ready",
                    true,
                    "RELAY",
                    "startup_watchdog",
                    Some(self.serial.clone()),
                )
                .with_detail("pid", pid));
            }
            if relay_socket_state(pid, self.config.port)
                .map(|state| state.listening)
                .unwrap_or(false)
            {
                snapshot.relay_health = HealthState::Ready;
                emit(
                    &self.log,
                    LogEvent::new(
                        LogLevel::Info,
                        "RELAY",
                        Some(self.serial.clone()),
                        "relay_listening",
                        format!(
                            "Owned relay PID {pid} is listening on port {}",
                            self.config.port
                        ),
                    )
                    .field("pid", pid)
                    .field("port", self.config.port),
                );
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Fault::new(
                    "relay_bind_failed",
                    "Relay process did not acquire its listening endpoint before the deadline",
                    true,
                    "RELAY",
                    "startup_watchdog",
                    Some(self.serial.clone()),
                )
                .with_detail("pid", pid)
                .with_detail("port", self.config.port));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn ensure_reverse(
        &mut self,
        adb: &AdbManager,
        snapshot: &mut ControllerSnapshot,
    ) -> Result<(), Fault> {
        if !adb.reverse_is_ready(&self.serial, self.config.port)? {
            snapshot.force_transition(SessionState::ReverseMissing);
            adb.create_reverse(&self.serial, self.config.port)?;
            self.owned_reverse = true;
        }
        if !adb.reverse_is_ready(&self.serial, self.config.port)? {
            return Err(Fault::new(
                "reverse_failed",
                "ADB reported success but the expected reverse mapping is absent",
                true,
                "ADB",
                "verify_reverse",
                Some(self.serial.clone()),
            ));
        }
        snapshot.reverse_health = HealthState::Ready;
        snapshot.force_transition(SessionState::TunnelReady);
        emit(
            &self.log,
            LogEvent::new(
                LogLevel::Info,
                "ADB",
                Some(self.serial.clone()),
                "reverse_ready",
                "Serial-scoped ADB reverse mapping is ready",
            )
            .field("port", self.config.port),
        );
        Ok(())
    }

    fn await_client_health(
        &mut self,
        adb: &AdbManager,
        snapshot: &mut ControllerSnapshot,
    ) -> Result<(), Fault> {
        let deadline = Instant::now() + CLIENT_START_DEADLINE;
        let mut saw_process = false;
        let mut saw_vpn = false;
        loop {
            if adb.client_pid(&self.serial)?.is_some() {
                saw_process = true;
                snapshot.process_health = HealthState::Ready;
            }
            let (vpn, validated) = adb.vpn_health(&self.serial)?;
            if vpn {
                saw_vpn = true;
                snapshot.vpn_health = HealthState::Ready;
                snapshot.force_transition(SessionState::VpnReady);
            }
            let relay_state = self.current_relay_socket_state()?;
            if relay_state.connected {
                snapshot.relay_health = HealthState::Ready;
                snapshot.force_transition(SessionState::RelayConnected);
            }
            if saw_process && saw_vpn && relay_state.connected {
                snapshot.internet_health = if validated && host_internet_ready() {
                    HealthState::Ready
                } else {
                    HealthState::Degraded
                };
                if snapshot.internet_health == HealthState::Ready {
                    snapshot.force_transition(SessionState::InternetReady);
                }
                snapshot.metrics.session_started_unix_ms = Some(Utc::now().timestamp_millis());
                emit(
                    &self.log,
                    LogEvent::new(
                        LogLevel::Info,
                        "CORE",
                        Some(self.serial.clone()),
                        "session_ready",
                        "Reverse-tethering session is healthy",
                    )
                    .field("vpn_validated", validated)
                    .field(
                        "internet_ready",
                        snapshot.internet_health == HealthState::Ready,
                    ),
                );
                return Ok(());
            }
            if !vpn
                && matches!(
                    adb.phone_requests(&self.serial),
                    Ok(PhoneRequests::Pending(PhoneRequest {
                        action: PhoneAction::Stop,
                        ..
                    }))
                )
            {
                return Err(Fault::new(
                    "phone_stopped_session",
                    "The phone stopped the session during startup",
                    false,
                    "ANDROID",
                    "await_client_health",
                    Some(self.serial.clone()),
                ));
            }
            if Instant::now() >= deadline {
                let (code, message, retryable, state) = if saw_process && !saw_vpn {
                    (
                        "vpn_permission_required",
                        "Android client is running but VPN consent or service activation is pending",
                        false,
                        SessionState::VpnPermissionRequired,
                    )
                } else if !saw_process {
                    (
                        "android_process_dead",
                        "Android client process did not remain running",
                        true,
                        SessionState::Faulted,
                    )
                } else {
                    (
                        "tunnel_disconnected",
                        "Android VPN started but did not connect to the owned relay",
                        true,
                        SessionState::Faulted,
                    )
                };
                snapshot.force_transition(state);
                return Err(Fault::new(
                    code,
                    message,
                    retryable,
                    "ANDROID",
                    "await_client_health",
                    Some(self.serial.clone()),
                ));
            }
            thread::sleep(Duration::from_millis(350));
        }
    }

    pub fn inspect_health(
        &mut self,
        adb: &AdbManager,
        snapshot: &mut ControllerSnapshot,
    ) -> Option<RecoveryAction> {
        snapshot.metrics.watchdog_ticks += 1;
        if let Ok(Some(status)) = self.relay_exited() {
            snapshot.relay_health = HealthState::Unavailable;
            let fault = Fault::new(
                "relay_exited",
                format!("Owned relay process exited with {status}"),
                true,
                "RELAY",
                "pid_watchdog",
                Some(self.serial.clone()),
            );
            return Some(RecoveryAction::RestartRelay(fault));
        }

        match adb.select_device(Some(&self.serial)) {
            Ok(device) => {
                snapshot.adb_state = AdbState::Ready;
                snapshot.selected_device = Some(device);
            }
            Err(fault) => {
                snapshot.adb_state = match fault.code.as_str() {
                    "adb_unauthorized" => AdbState::Unauthorized,
                    "device_offline" => AdbState::Offline,
                    _ => AdbState::NoDevice,
                };
                return Some(RecoveryAction::WaitForAdb(fault));
            }
        }

        match adb.reverse_is_ready(&self.serial, self.config.port) {
            Ok(true) => snapshot.reverse_health = HealthState::Ready,
            Ok(false) => {
                snapshot.reverse_health = HealthState::Unavailable;
                return Some(RecoveryAction::RebuildReverse(Fault::new(
                    "reverse_missing",
                    "ADB reverse mapping disappeared",
                    true,
                    "ADB",
                    "reverse_watchdog",
                    Some(self.serial.clone()),
                )));
            }
            Err(fault) => return Some(RecoveryAction::WaitForAdb(fault)),
        }

        match adb.client_pid(&self.serial) {
            Ok(Some(_)) => snapshot.process_health = HealthState::Ready,
            Ok(None) => {
                snapshot.process_health = HealthState::Unavailable;
                return Some(RecoveryAction::RestartClient(Fault::new(
                    "android_process_dead",
                    "Android client process exited",
                    true,
                    "ANDROID",
                    "pid_watchdog",
                    Some(self.serial.clone()),
                )));
            }
            Err(fault) => return Some(RecoveryAction::WaitForAdb(fault)),
        }

        // Ground truth from the client's own dump(): true only while its VPN object
        // is alive. The lingering tun0 interface and ghost ServiceRecords both survive
        // a background kill, so neither is trustworthy on its own.
        match adb.service_is_active(&self.serial) {
            Ok(true) => {}
            Ok(false) => {
                snapshot.vpn_health = HealthState::Unavailable;
                return Some(RecoveryAction::RestartClient(Fault::new(
                    "vpn_service_stopped",
                    "Android process is alive but its VPN service stopped",
                    true,
                    "VPN",
                    "service_watchdog",
                    Some(self.serial.clone()),
                )));
            }
            Err(fault) => return Some(RecoveryAction::WaitForAdb(fault)),
        }

        let (vpn, validated) = match adb.vpn_health(&self.serial) {
            Ok(health) => health,
            Err(fault) => return Some(RecoveryAction::WaitForAdb(fault)),
        };
        snapshot.vpn_health = if vpn {
            HealthState::Ready
        } else {
            HealthState::Unavailable
        };
        if !vpn {
            return Some(RecoveryAction::RestartClient(Fault::new(
                "vpn_stopped",
                "Android VPN network disappeared",
                true,
                "VPN",
                "vpn_watchdog",
                Some(self.serial.clone()),
            )));
        }

        let relay = match self.current_relay_socket_state() {
            Ok(relay) => relay,
            Err(fault) => return Some(RecoveryAction::RestartRelay(fault)),
        };
        if !relay.listening {
            snapshot.relay_health = HealthState::Unavailable;
            return Some(RecoveryAction::RestartRelay(Fault::new(
                "relay_listener_lost",
                "Owned relay PID no longer owns its listening endpoint",
                true,
                "RELAY",
                "endpoint_watchdog",
                Some(self.serial.clone()),
            )));
        }
        if !relay.connected {
            self.relay_disconnect_ticks = self.relay_disconnect_ticks.saturating_add(1);
            snapshot.relay_health = HealthState::Degraded;
            if self.relay_disconnect_ticks >= 3 {
                self.relay_disconnect_ticks = 0;
                return Some(RecoveryAction::RestartClient(Fault::new(
                    "tunnel_disconnected",
                    "Relay listener is healthy but the Android tunnel stayed disconnected",
                    true,
                    "RELAY",
                    "connection_watchdog",
                    Some(self.serial.clone()),
                )));
            }
        } else {
            self.relay_disconnect_ticks = 0;
            snapshot.relay_health = HealthState::Ready;
        }

        snapshot.internet_health = if validated && host_internet_ready() {
            HealthState::Ready
        } else {
            HealthState::Degraded
        };
        if snapshot.internet_health != HealthState::Ready {
            return Some(RecoveryAction::DiagnoseInternet(Fault::new(
                "internet_lost",
                "VPN or Windows network validation is degraded; automatic VPN restart suppressed",
                true,
                "NETWORK",
                "internet_watchdog",
                Some(self.serial.clone()),
            )));
        }
        snapshot.force_transition(SessionState::InternetReady);
        None
    }

    pub fn recover(
        &mut self,
        action: &RecoveryAction,
        adb: &AdbManager,
        snapshot: &mut ControllerSnapshot,
    ) -> Result<(), Fault> {
        match action {
            RecoveryAction::WaitForAdb(_) => {
                adb.select_device(Some(&self.serial))?;
                self.ensure_reverse(adb, snapshot)?;
            }
            RecoveryAction::RebuildReverse(_) => {
                self.ensure_reverse(adb, snapshot)?;
                snapshot.metrics.reverse_repairs += 1;
            }
            RecoveryAction::RestartClient(fault) => {
                if fault.code == "android_process_dead" {
                    if let Ok(exit_info) = adb.application_exit_info(&self.serial) {
                        emit(
                            &self.log,
                            LogEvent::new(
                                LogLevel::Warn,
                                "ANDROID",
                                Some(self.serial.clone()),
                                "application_exit_info",
                                "Captured ApplicationExitInfo before recovery",
                            )
                            .field("output", exit_info.chars().take(4_096).collect::<String>()),
                        );
                    }
                }
                self.ensure_reverse(adb, snapshot)?;
                if let Err(silent_fault) = adb.restart_client_service(
                    &self.serial,
                    self.config.dns_servers.as_deref(),
                    self.config.routes.as_deref(),
                ) {
                    log_fault(&self.log, &silent_fault, "silent_client_restart_failed");
                    adb.restart_client_invisibly(
                        &self.serial,
                        self.config.dns_servers.as_deref(),
                        self.config.routes.as_deref(),
                    )?;
                }
                snapshot.metrics.client_restarts += 1;
                snapshot.metrics.reconnect_count += 1;
            }
            RecoveryAction::RestartRelay(_) => {
                if let Some(mut relay) = self.relay.take() {
                    let _ = relay.kill();
                    let _ = relay.wait();
                }
                self.spawn_relay(snapshot)?;
                self.ensure_reverse(adb, snapshot)?;
                if let Err(silent_fault) = adb.restart_client_service(
                    &self.serial,
                    self.config.dns_servers.as_deref(),
                    self.config.routes.as_deref(),
                ) {
                    log_fault(&self.log, &silent_fault, "silent_client_restart_failed");
                    adb.restart_client_invisibly(
                        &self.serial,
                        self.config.dns_servers.as_deref(),
                        self.config.routes.as_deref(),
                    )?;
                }
                snapshot.metrics.relay_restarts += 1;
                snapshot.metrics.reconnect_count += 1;
            }
            RecoveryAction::DiagnoseInternet(fault) => return Err(fault.clone()),
        }
        Ok(())
    }

    pub fn stop(
        &mut self,
        adb: &AdbManager,
        snapshot: &mut ControllerSnapshot,
        replace_phone_request: bool,
    ) {
        snapshot.force_transition(SessionState::Stopping);
        if self.client_started {
            if let Err(fault) = adb.stop_client(&self.serial, replace_phone_request) {
                log_fault(&self.log, &fault, "client_stop_failed");
            }
        }
        self.release(adb, snapshot);
    }

    pub fn release(&mut self, adb: &AdbManager, snapshot: &mut ControllerSnapshot) {
        if self.owned_reverse {
            if let Err(fault) = adb.remove_reverse(&self.serial) {
                log_fault(&self.log, &fault, "reverse_remove_failed");
            }
        }
        if let Some(mut relay) = self.relay.take() {
            let _ = relay.kill();
            let _ = relay.wait();
        }
        self.clear_session_health(snapshot);
        emit(
            &self.log,
            LogEvent::new(
                LogLevel::Info,
                "CORE",
                Some(self.serial.clone()),
                "session_stopped",
                "Session resources owned by the controller were stopped",
            ),
        );
    }

    fn rollback(&mut self, adb: &AdbManager, snapshot: &mut ControllerSnapshot) {
        emit(
            &self.log,
            LogEvent::new(
                LogLevel::Warn,
                "CORE",
                Some(self.serial.clone()),
                "start_rollback",
                "Rolling back resources created by the failed start transaction",
            ),
        );
        self.stop(adb, snapshot, false);
    }

    fn clear_session_health(&self, snapshot: &mut ControllerSnapshot) {
        snapshot.force_transition(SessionState::Idle);
        snapshot.reverse_health = HealthState::Unknown;
        snapshot.process_health = HealthState::Unknown;
        snapshot.vpn_health = HealthState::Unknown;
        snapshot.relay_health = HealthState::Unknown;
        snapshot.internet_health = HealthState::Unknown;
        snapshot.relay_pid = None;
        snapshot.recovery_attempt = 0;
        snapshot.next_recovery_unix_ms = None;
        snapshot.metrics.session_started_unix_ms = None;
        self.traffic
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reset();
    }

    fn relay_exited(&mut self) -> Result<Option<std::process::ExitStatus>, Fault> {
        match self.relay.as_mut() {
            Some(relay) => relay.try_wait().map_err(|error| {
                Fault::new(
                    "relay_watchdog_failed",
                    format!("Could not inspect relay PID: {error}"),
                    true,
                    "RELAY",
                    "pid_watchdog",
                    Some(self.serial.clone()),
                )
            }),
            None => Ok(None),
        }
    }

    fn current_relay_socket_state(&self) -> Result<RelaySocketState, Fault> {
        let pid = self.relay.as_ref().map(Child::id).ok_or_else(|| {
            Fault::new(
                "relay_exited",
                "Relay process handle is absent",
                true,
                "RELAY",
                "endpoint_watchdog",
                Some(self.serial.clone()),
            )
        })?;
        relay_socket_state(pid, self.config.port).map_err(|error| {
            Fault::new(
                "relay_watchdog_failed",
                format!("Could not inspect relay endpoint ownership: {error}"),
                true,
                "RELAY",
                "endpoint_watchdog",
                Some(self.serial.clone()),
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RelaySocketState {
    pub listening: bool,
    pub connected: bool,
}

pub fn parse_netstat(output: &str, pid: u32, port: u16) -> RelaySocketState {
    let port_suffix = format!(":{port}");
    let mut result = RelaySocketState::default();
    for line in output.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 4 || !fields[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if fields.last().and_then(|value| value.parse::<u32>().ok()) != Some(pid)
            || !fields[1].ends_with(&port_suffix)
        {
            continue;
        }
        if fields
            .iter()
            .any(|value| value.eq_ignore_ascii_case("LISTENING"))
        {
            result.listening = true;
        }
        if fields
            .iter()
            .any(|value| value.eq_ignore_ascii_case("ESTABLISHED"))
        {
            result.connected = true;
        }
    }
    result
}

fn relay_socket_state(pid: u32, port: u16) -> Result<RelaySocketState, ProcessError> {
    let args = [
        OsString::from("-ano"),
        OsString::from("-p"),
        OsString::from("tcp"),
    ];
    let output = run_program(Path::new("netstat.exe"), &args, Duration::from_secs(3))?;
    Ok(parse_netstat(&output.stdout, pid, port))
}

fn host_internet_ready() -> bool {
    const PROBE_ENDPOINTS: [(&str, u16); 3] =
        [("223.5.5.5", 443), ("8.8.8.8", 443), ("1.1.1.1", 443)];
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for &(host, port) in &PROBE_ENDPOINTS {
            let sender = sender.clone();
            scope.spawn(move || {
                if let Ok(address) = format!("{host}:{port}").parse::<SocketAddr>() {
                    if TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok() {
                        let _ = sender.send(());
                    }
                }
            });
        }
    });
    receiver.try_recv().is_ok()
}

trait RelayOutput: Read + Send + 'static {}
impl RelayOutput for ChildStdout {}
impl RelayOutput for ChildStderr {}

fn stream_relay_output<R: RelayOutput>(
    stream: R,
    log: SharedLog,
    serial: String,
    level: LogLevel,
    traffic: Option<SharedTraffic>,
) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let (Some(report), Some(traffic)) = (parse_traffic_report(&line), traffic.as_ref()) {
                traffic
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .update(report);
                continue;
            }
            emit(
                &log,
                LogEvent::new(
                    level,
                    "RELAY",
                    Some(serial.clone()),
                    "relay_output",
                    line.chars().take(4_096).collect::<String>(),
                ),
            );
        }
    });
}

fn parse_traffic_report(line: &str) -> Option<TrafficTotals> {
    let encoded = line.strip_prefix(crate::relay::traffic_metrics::METRICS_PREFIX)?;
    let report =
        serde_json::from_str::<crate::relay::traffic_metrics::TrafficTotals>(encoded).ok()?;
    Some(TrafficTotals {
        bytes_from_device: report.bytes_from_device,
        bytes_to_device: report.bytes_to_device,
        packets_from_device: report.packets_from_device,
        packets_to_device: report.packets_to_device,
    })
}

pub fn log_fault(log: &SharedLog, fault: &Fault, event_type: &str) {
    emit(
        log,
        LogEvent::new(
            LogLevel::Error,
            &fault.component,
            fault.device_id.clone(),
            event_type,
            &fault.message,
        )
        .field("code", fault.code.clone())
        .field("retryable", fault.retryable)
        .field("operation", fault.operation.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstat_parser_requires_matching_pid_and_local_port() {
        let output = "  TCP    0.0.0.0:31416   0.0.0.0:0      LISTENING       42\n  TCP    127.0.0.1:31416 127.0.0.1:50000 ESTABLISHED     42\n  TCP    0.0.0.0:31416   0.0.0.0:0      LISTENING       99\n";
        assert_eq!(
            RelaySocketState {
                listening: true,
                connected: true
            },
            parse_netstat(output, 42, 31416)
        );
        assert_eq!(RelaySocketState::default(), parse_netstat(output, 7, 31416));
    }

    #[test]
    fn parses_only_well_formed_relay_traffic_reports() {
        let report = parse_traffic_report(
            "GNIREHTET_NG_METRICS {\"bytes_from_device\":10,\"bytes_to_device\":20,\"packets_from_device\":1,\"packets_to_device\":2}",
        )
        .unwrap();
        assert_eq!(10, report.bytes_from_device);
        assert_eq!(20, report.bytes_to_device);
        assert!(parse_traffic_report("ordinary relay output").is_none());
        assert!(parse_traffic_report("GNIREHTET_NG_METRICS invalid").is_none());
    }
}
