use crate::controller::adb::AdbManager;
use crate::controller::diagnostics::export_diagnostics;
use crate::controller::model::{
    recovery_delay, AdbState, ControllerSnapshot, Fault, SessionState, SharedTraffic,
    TrafficTracker, MAX_CONSECUTIVE_RECOVERY_FAILURES,
};
use crate::controller::session::{log_fault, DeviceSession, RecoveryAction, SessionConfig};
use crate::controller::structured_log::{emit, LogEvent, LogLevel, LogStore, SharedLog};
use chrono::Utc;
use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const WATCHDOG_INTERVAL: Duration = Duration::from_secs(2);

const HYPEROS_NO_RESTRICT_KEY: &str = "MILLET_NO_RESTRICT_APP";

fn resolve_hyperos_serial(adb: &AdbManager, serial: Option<&str>) -> Result<String, Fault> {
    if let Some(serial) = serial {
        return Ok(serial.to_string());
    }
    adb.list_devices()?
        .into_iter()
        .find(|device| device.adb_state == AdbState::Ready)
        .map(|device| device.serial)
        .ok_or_else(|| {
            Fault::new(
                "no_device",
                "No ready ADB device",
                false,
                "ANDROID",
                "hyperos",
                None,
            )
        })
}

fn update_no_restrict_list(raw: &str, package: &str, protect: bool) -> String {
    let mut packages: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    if protect && !packages.contains(&package) {
        packages.push(package);
    } else if !protect {
        packages.retain(|item| *item != package);
    }
    packages.join(",")
}

fn hyperos_op(
    adb: &AdbManager,
    serial: Option<&str>,
    protect: bool,
    unprotect: bool,
) -> Result<serde_json::Value, Fault> {
    let serial = resolve_hyperos_serial(adb, serial)?;
    let before = adb.settings_get(&serial, HYPEROS_NO_RESTRICT_KEY)?;
    let present_before = before
        .split(',')
        .map(str::trim)
        .any(|item| item == crate::controller::CLIENT_PACKAGE);
    if !protect && !unprotect {
        return Ok(serde_json::json!({
            "serial": serial,
            "key": HYPEROS_NO_RESTRICT_KEY,
            "raw": before,
            "protected": present_before,
        }));
    }
    let after = update_no_restrict_list(&before, crate::controller::CLIENT_PACKAGE, protect);
    if after == before {
        return Ok(serde_json::json!({
            "serial": serial,
            "key": HYPEROS_NO_RESTRICT_KEY,
            "raw": before,
            "protected": present_before,
            "changed": false,
        }));
    }
    adb.settings_put(&serial, HYPEROS_NO_RESTRICT_KEY, &after)?;
    let verified = adb.settings_get(&serial, HYPEROS_NO_RESTRICT_KEY)?;
    let protected = verified
        .split(',')
        .map(str::trim)
        .any(|item| item == crate::controller::CLIENT_PACKAGE);
    Ok(serde_json::json!({
        "serial": serial,
        "key": HYPEROS_NO_RESTRICT_KEY,
        "raw": verified,
        "protected": protected,
        "changed": true,
    }))
}

fn ensure_hyperos_whitelist(adb: &AdbManager, log: &SharedLog, serial: Option<&str>) {
    let device = match resolve_hyperos_serial(adb, serial) {
        Ok(device) => device,
        Err(fault) => {
            log_fault(log, &fault, "hyperos_whitelist_check_failed");
            return;
        }
    };
    let raw = match adb.settings_get(&device, HYPEROS_NO_RESTRICT_KEY) {
        Ok(raw) => raw,
        Err(fault) => {
            log_fault(log, &fault, "hyperos_whitelist_check_failed");
            return;
        }
    };
    // Never create the key on devices without HyperOS: an absent list must stay absent.
    if raw == "null" || raw.trim().is_empty() {
        emit(
            log,
            LogEvent::new(
                LogLevel::Debug,
                "ANDROID",
                Some(device),
                "hyperos_whitelist_absent",
                "No HyperOS no-restrict list on this device; skipping",
            ),
        );
        return;
    }
    if raw
        .split(',')
        .map(str::trim)
        .any(|item| item == crate::controller::CLIENT_PACKAGE)
    {
        emit(
            log,
            LogEvent::new(
                LogLevel::Debug,
                "ANDROID",
                Some(device),
                "hyperos_whitelist_ok",
                "HyperOS no-restrict list already contains the client package",
            ),
        );
        return;
    }
    let updated = update_no_restrict_list(&raw, crate::controller::CLIENT_PACKAGE, true);
    let verified = adb
        .settings_put(&device, HYPEROS_NO_RESTRICT_KEY, &updated)
        .and_then(|_| adb.settings_get(&device, HYPEROS_NO_RESTRICT_KEY));
    match verified {
        Ok(value) if value.contains(crate::controller::CLIENT_PACKAGE) => {
            emit(
                log,
                LogEvent::new(
                    LogLevel::Info,
                    "ANDROID",
                    Some(device),
                    "hyperos_whitelist_added",
                    "Added the client package to the HyperOS no-restrict list",
                ),
            );
        }
        Ok(_) => {
            emit(
                log,
                LogEvent::new(
                    LogLevel::Warn,
                    "ANDROID",
                    Some(device),
                    "hyperos_whitelist_verify_failed",
                    "The HyperOS no-restrict list was written but the package is not present on read-back",
                ),
            );
        }
        Err(fault) => {
            log_fault(log, &fault, "hyperos_whitelist_write_failed");
        }
    }
}

fn monitor_gui_process(pid: u32, command_tx: mpsc::Sender<ControlCommand>) {
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};
    const SYNCHRONIZE_RIGHT: u32 = 0x0010_0000;
    unsafe {
        let handle = OpenProcess(SYNCHRONIZE_RIGHT, 0, pid);
        if handle.is_null() {
            return;
        }
        WaitForSingleObject(handle, u32::MAX);
        let _ = command_tx.send(ControlCommand::ParentExited);
    }
}

enum ControlCommand {
    Start(
        SessionConfig,
        mpsc::Sender<Result<ControllerSnapshot, Fault>>,
    ),
    Stop(mpsc::Sender<Result<ControllerSnapshot, Fault>>),
    Restart(
        SessionConfig,
        mpsc::Sender<Result<ControllerSnapshot, Fault>>,
    ),
    Shutdown(mpsc::Sender<Result<ControllerSnapshot, Fault>>),
    ParentExited,
    HyperosStatus(
        Option<String>,
        mpsc::Sender<Result<serde_json::Value, Fault>>,
    ),
    HyperosProtect(
        Option<String>,
        mpsc::Sender<Result<serde_json::Value, Fault>>,
    ),
    HyperosUnprotect(
        Option<String>,
        mpsc::Sender<Result<serde_json::Value, Fault>>,
    ),
}

struct PendingRecovery {
    action: RecoveryAction,
    attempt: u32,
    due: Instant,
}

#[derive(Clone)]
pub struct ControllerService {
    snapshot: Arc<Mutex<ControllerSnapshot>>,
    log: SharedLog,
    adb: AdbManager,
    state_directory: PathBuf,
    command_tx: mpsc::Sender<ControlCommand>,
    traffic: SharedTraffic,
}

impl ControllerService {
    pub fn start_default() -> Result<Self, Fault> {
        Self::start_with_parent(None)
    }

    pub fn start_with_parent(parent_pid: Option<u32>) -> Result<Self, Fault> {
        let state_directory = state_directory();
        let log = LogStore::open(state_directory.join("logs")).map_err(|error| {
            Fault::new(
                "core_start_failed",
                format!("Could not open controller log store: {error}"),
                false,
                "CORE",
                "start",
                None,
            )
        })?;
        let log = Arc::new(Mutex::new(log));
        let snapshot = Arc::new(Mutex::new(ControllerSnapshot::default()));
        let traffic = Arc::new(Mutex::new(TrafficTracker::default()));
        let adb = AdbManager::default();
        let apk_path = artifact_path("GNIREHTET_APK", "gnirehtet-ng.apk");
        let relay_executable = artifact_path("GNIREHTET_NG_RELAY", "gnirehtet-ng.exe");
        let (command_tx, command_rx) = mpsc::channel();

        let worker_snapshot = snapshot.clone();
        let worker_log = log.clone();
        let worker_adb = adb.clone();
        let worker_state_directory = state_directory.clone();
        let worker_traffic = traffic.clone();
        thread::Builder::new()
            .name("gnirehtet-controller".to_string())
            .spawn(move || {
                worker_loop(
                    command_rx,
                    worker_snapshot,
                    worker_log,
                    worker_adb,
                    apk_path,
                    relay_executable,
                    worker_state_directory,
                    worker_traffic,
                    parent_pid,
                )
            })
            .map_err(|error| {
                Fault::new(
                    "core_start_failed",
                    format!("Could not start controller worker: {error}"),
                    false,
                    "CORE",
                    "start",
                    None,
                )
            })?;

        if let Some(pid) = parent_pid {
            let monitor_tx = command_tx.clone();
            thread::Builder::new()
                .name("gnirehtet-gui-monitor".to_string())
                .spawn(move || monitor_gui_process(pid, monitor_tx))
                .map_err(|error| {
                    Fault::new(
                        "core_start_failed",
                        format!("Could not start GUI monitor: {error}"),
                        false,
                        "CORE",
                        "start",
                        None,
                    )
                })?;
        }

        let service = Self {
            snapshot,
            log,
            adb,
            state_directory,
            command_tx,
            traffic,
        };
        emit(
            &service.log,
            LogEvent::new(
                LogLevel::Info,
                "CORE",
                None,
                "core_started",
                "Gnirehtet NG controller is ready",
            )
            .field("pid", std::process::id()),
        );
        Ok(service)
    }

    pub fn snapshot(&self) -> ControllerSnapshot {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        sync_traffic(&mut snapshot, &self.traffic);
        snapshot
    }

    pub fn devices(&self) -> Result<Vec<crate::controller::model::DeviceInfo>, Fault> {
        self.adb.list_devices()
    }

    pub fn start_session(&self, config: SessionConfig) -> Result<ControllerSnapshot, Fault> {
        self.send_control(|response| ControlCommand::Start(config, response))
    }

    pub fn stop_session(&self) -> Result<ControllerSnapshot, Fault> {
        self.send_control(ControlCommand::Stop)
    }

    pub fn restart_session(&self, config: SessionConfig) -> Result<ControllerSnapshot, Fault> {
        self.send_control(|response| ControlCommand::Restart(config, response))
    }

    pub fn shutdown(&self) -> Result<ControllerSnapshot, Fault> {
        self.send_control(ControlCommand::Shutdown)
    }

    pub fn hyperos_status(&self, serial: Option<String>) -> Result<serde_json::Value, Fault> {
        self.send_control(|response| ControlCommand::HyperosStatus(serial, response))
    }

    pub fn hyperos_protect(&self, serial: Option<String>) -> Result<serde_json::Value, Fault> {
        self.send_control(|response| ControlCommand::HyperosProtect(serial, response))
    }

    pub fn hyperos_unprotect(&self, serial: Option<String>) -> Result<serde_json::Value, Fault> {
        self.send_control(|response| ControlCommand::HyperosUnprotect(serial, response))
    }

    pub fn logs(
        &self,
        limit: usize,
        minimum: Option<crate::controller::structured_log::LogLevel>,
        component: Option<&str>,
        device_id: Option<&str>,
        after_sequence: Option<u64>,
        before_sequence: Option<u64>,
    ) -> Vec<crate::controller::structured_log::LogEvent> {
        self.log
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .recent(
                limit,
                minimum,
                component,
                device_id,
                after_sequence,
                before_sequence,
            )
    }

    pub fn diagnose(&self) -> Result<PathBuf, Fault> {
        export_diagnostics(
            &self.state_directory,
            &self.snapshot(),
            &self.adb,
            &self.log,
        )
    }

    fn send_control<F, T>(&self, build: F) -> Result<T, Fault>
    where
        F: FnOnce(mpsc::Sender<Result<T, Fault>>) -> ControlCommand,
    {
        let (response_tx, response_rx) = mpsc::channel();
        self.command_tx.send(build(response_tx)).map_err(|_| {
            Fault::new(
                "core_unavailable",
                "Controller worker stopped unexpectedly",
                true,
                "CORE",
                "control_command",
                None,
            )
        })?;
        response_rx.recv().map_err(|_| {
            Fault::new(
                "core_unavailable",
                "Controller worker closed the command response channel",
                true,
                "CORE",
                "control_command",
                None,
            )
        })?
    }
}
fn worker_loop(
    command_rx: mpsc::Receiver<ControlCommand>,
    snapshot: Arc<Mutex<ControllerSnapshot>>,
    log: SharedLog,
    adb: AdbManager,
    apk_path: PathBuf,
    relay_executable: PathBuf,
    state_directory: PathBuf,
    traffic: SharedTraffic,
    parent_pid: Option<u32>,
) {
    let mut session: Option<DeviceSession> = None;
    let mut pending_recovery: Option<PendingRecovery> = None;
    let mut last_health_fault: Option<String> = None;
    let mut exhausted_fault: Option<String> = None;
    let mut adoption_cooling_until: Option<Instant> = None;
    let mut idle_ticks: u32 = 0;

    loop {
        match command_rx.recv_timeout(WATCHDOG_INTERVAL) {
            Ok(ControlCommand::Start(config, response)) => {
                pending_recovery = None;
                exhausted_fault = None;
                ensure_hyperos_whitelist(&adb, &log, config.serial.as_deref());
                if let Some(mut active) = session.take() {
                    let mut current = lock_snapshot(&snapshot);
                    active.stop(&adb, &mut current);
                    persist_snapshot(&state_directory, &current);
                }
                let mut current = lock_snapshot(&snapshot);
                let result = DeviceSession::start(
                    config,
                    &adb,
                    &apk_path,
                    &relay_executable,
                    &mut current,
                    log.clone(),
                    traffic.clone(),
                );
                match result {
                    Ok(active) => {
                        session = Some(active);
                        current.last_fault = None;
                        sync_traffic(&mut current, &traffic);
                        let _ = response.send(Ok(current.clone()));
                    }
                    Err(fault) => {
                        log_fault(&log, &fault, "session_start_failed");
                        current.record_fault(fault.clone(), SessionState::Faulted);
                        sync_traffic(&mut current, &traffic);
                        let _ = response.send(Err(fault));
                    }
                }
                persist_snapshot(&state_directory, &current);
            }
            Ok(ControlCommand::Stop(response)) => {
                pending_recovery = None;
                exhausted_fault = None;
                if let Some(mut active) = session.take() {
                    let mut current = lock_snapshot(&snapshot);
                    active.stop(&adb, &mut current);
                    current.last_fault = None;
                    sync_traffic(&mut current, &traffic);
                    persist_snapshot(&state_directory, &current);
                    let _ = response.send(Ok(current.clone()));
                } else {
                    let mut current = lock_snapshot(&snapshot);
                    reset_inactive_snapshot(&mut current);
                    traffic
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .reset();
                    sync_traffic(&mut current, &traffic);
                    persist_snapshot(&state_directory, &current);
                    let _ = response.send(Ok(current.clone()));
                }
            }
            Ok(ControlCommand::Restart(config, response)) => {
                pending_recovery = None;
                exhausted_fault = None;
                ensure_hyperos_whitelist(&adb, &log, config.serial.as_deref());
                if let Some(mut active) = session.take() {
                    let mut current = lock_snapshot(&snapshot);
                    active.stop(&adb, &mut current);
                }
                let mut current = lock_snapshot(&snapshot);
                match DeviceSession::start(
                    config,
                    &adb,
                    &apk_path,
                    &relay_executable,
                    &mut current,
                    log.clone(),
                    traffic.clone(),
                ) {
                    Ok(active) => {
                        session = Some(active);
                        current.last_fault = None;
                        sync_traffic(&mut current, &traffic);
                        let _ = response.send(Ok(current.clone()));
                    }
                    Err(fault) => {
                        log_fault(&log, &fault, "session_restart_failed");
                        current.record_fault(fault.clone(), SessionState::Faulted);
                        sync_traffic(&mut current, &traffic);
                        let _ = response.send(Err(fault));
                    }
                }
                persist_snapshot(&state_directory, &current);
            }
            Ok(ControlCommand::Shutdown(response)) => {
                let mut current = lock_snapshot(&snapshot);
                if let Some(mut active) = session.take() {
                    active.stop(&adb, &mut current);
                } else {
                    reset_inactive_snapshot(&mut current);
                }
                traffic
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .reset();
                current.last_fault = None;
                sync_traffic(&mut current, &traffic);
                emit(
                    &log,
                    LogEvent::new(
                        LogLevel::Info,
                        "CORE",
                        None,
                        "core_stopping",
                        "Gnirehtet NG controller is shutting down cleanly",
                    ),
                );
                persist_snapshot(&state_directory, &current);
                let _ = response.send(Ok(current.clone()));
                break;
            }
            Ok(ControlCommand::ParentExited) => {
                emit(
                    &log,
                    LogEvent::new(
                        LogLevel::Info,
                        "CORE",
                        None,
                        "gui_parent_exited",
                        "GUI process exited; cleaning up the session, phone app, and ADB server",
                    ),
                );
                if let Some(mut active) = session.take() {
                    let mut current = lock_snapshot(&snapshot);
                    active.stop(&adb, &mut current);
                    persist_snapshot(&state_directory, &current);
                }
                for device in adb.list_devices().unwrap_or_default() {
                    let _ = adb.execute(
                        Some(&device.serial),
                        [
                            "shell",
                            "am",
                            "force-stop",
                            crate::controller::CLIENT_PACKAGE,
                        ],
                        "gui_cleanup_force_stop",
                    );
                }
                let _ = adb.kill_server();
                std::process::exit(0);
            }
            Ok(ControlCommand::HyperosStatus(serial, response)) => {
                let _ = response.send(hyperos_op(&adb, serial.as_deref(), false, false));
            }
            Ok(ControlCommand::HyperosProtect(serial, response)) => {
                let _ = response.send(hyperos_op(&adb, serial.as_deref(), true, false));
            }
            Ok(ControlCommand::HyperosUnprotect(serial, response)) => {
                let _ = response.send(hyperos_op(&adb, serial.as_deref(), false, true));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(active) = session.as_mut() {
                    watchdog_tick(
                        active,
                        &adb,
                        &snapshot,
                        &log,
                        &state_directory,
                        &mut pending_recovery,
                        &mut last_health_fault,
                        &mut exhausted_fault,
                        &traffic,
                    );
                } else if parent_pid.is_some() {
                    // Poll phone-initiated sessions sparingly: every poll spawns ADB
                    // processes, which would otherwise keep the ADB daemon alive (and
                    // resurrect it right after a manual kill) every two seconds.
                    idle_ticks = idle_ticks.wrapping_add(1);
                    if idle_ticks >= 5 {
                        idle_ticks = 0;
                        session = adopt_phone_started_session(
                            &adb,
                            &apk_path,
                            &relay_executable,
                            &snapshot,
                            &log,
                            &state_directory,
                            &traffic,
                            &mut adoption_cooling_until,
                        );
                    }
                }
            }
        }
    }
}

/// While the GUI-owned controller is idle, a session started from the phone app itself
/// (VPN active but no controller session) is adopted through the normal start transaction,
/// so the relay and reverse come up and both sides report the same state.
fn adopt_phone_started_session(
    adb: &AdbManager,
    apk_path: &Path,
    relay_executable: &Path,
    snapshot: &Arc<Mutex<ControllerSnapshot>>,
    log: &SharedLog,
    state_directory: &Path,
    traffic: &SharedTraffic,
    cooling_until: &mut Option<Instant>,
) -> Option<DeviceSession> {
    if let Some(until) = cooling_until.as_ref() {
        if Instant::now() < *until {
            return None;
        }
    }
    let devices = adb.list_devices().ok()?;
    let ready = devices
        .into_iter()
        .find(|device| device.adb_state == AdbState::Ready)?;
    match adb.service_is_active(&ready.serial) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(_) => return None,
    }
    let mut config = SessionConfig::default();
    config.serial = Some(ready.serial.clone());
    let mut current = lock_snapshot(snapshot);
    match DeviceSession::start(
        config,
        adb,
        apk_path,
        relay_executable,
        &mut current,
        log.clone(),
        traffic.clone(),
    ) {
        Ok(session) => {
            emit(
                log,
                LogEvent::new(
                    LogLevel::Info,
                    "CORE",
                    Some(ready.serial),
                    "session_adopted",
                    "Phone-initiated VPN detected; adopting the session",
                ),
            );
            persist_snapshot(state_directory, &current);
            Some(session)
        }
        Err(fault) => {
            log_fault(log, &fault, "session_adoption_failed");
            *cooling_until = Some(Instant::now() + Duration::from_secs(10));
            persist_snapshot(state_directory, &current);
            None
        }
    }
}

fn watchdog_tick(
    session: &mut DeviceSession,
    adb: &AdbManager,
    snapshot: &Arc<Mutex<ControllerSnapshot>>,
    log: &SharedLog,
    state_directory: &Path,
    pending: &mut Option<PendingRecovery>,
    last_health_fault: &mut Option<String>,
    exhausted_fault: &mut Option<String>,
    traffic: &SharedTraffic,
) {
    if let Some(recovery) = pending.as_ref() {
        if Instant::now() < recovery.due {
            return;
        }
    }

    if let Some(recovery) = pending.take() {
        let mut current = lock_snapshot(snapshot);
        current.metrics.recovery_attempts += 1;
        emit(
            log,
            LogEvent::new(
                LogLevel::Info,
                "RECOVERY",
                Some(session.serial.clone()),
                "recovery_attempt",
                format!("Recovery attempt #{}", recovery.attempt),
            )
            .field("attempt", recovery.attempt)
            .field("reason_code", recovery.action.fault().code.clone()),
        );
        match session.recover(&recovery.action, adb, &mut current) {
            Ok(()) => {
                current.metrics.recovery_successes += 1;
                current.recovery_attempt = 0;
                current.next_recovery_unix_ms = None;
                current.last_fault = None;
                current.force_transition(SessionState::Starting);
                *last_health_fault = None;
                emit(
                    log,
                    LogEvent::new(
                        LogLevel::Info,
                        "RECOVERY",
                        Some(session.serial.clone()),
                        "recovery_succeeded",
                        "Recovery action completed; watchdog will verify end-to-end health",
                    ),
                );
            }
            Err(fault) => {
                log_fault(log, &fault, "recovery_failed");
                let exhausted_code = recovery.action.fault().code.clone();
                if schedule_recovery(
                    recovery.action,
                    recovery.attempt + 1,
                    &mut current,
                    pending,
                    log,
                ) {
                    *exhausted_fault = Some(exhausted_code);
                }
            }
        }
        sync_traffic(&mut current, traffic);
        persist_snapshot(state_directory, &current);
        return;
    }

    let mut current = lock_snapshot(snapshot);
    match session.inspect_health(adb, &mut current) {
        None => {
            current.recovery_attempt = 0;
            current.next_recovery_unix_ms = None;
            current.last_fault = None;
            *last_health_fault = None;
            *exhausted_fault = None;
        }
        Some(RecoveryAction::DiagnoseInternet(fault)) => {
            current.last_fault = Some(fault.clone());
            current.force_transition(SessionState::RelayConnected);
            if last_health_fault.as_deref() != Some(fault.code.as_str()) {
                emit(
                    log,
                    LogEvent::new(
                        LogLevel::Warn,
                        "NETWORK",
                        fault.device_id.clone(),
                        "internet_degraded",
                        &fault.message,
                    )
                    .field("code", fault.code.clone())
                    .field("recovery_action", "diagnose_without_vpn_restart"),
                );
            }
            *last_health_fault = Some(fault.code);
        }
        Some(action) => {
            let fault = action.fault().clone();
            current.last_fault = Some(fault.clone());
            if !current.auto_reconnect || !fault.retryable {
                current.force_transition(SessionState::Faulted);
                log_fault(log, &fault, "watchdog_fault");
            } else if exhausted_fault.as_deref() == Some(fault.code.as_str()) {
                current.force_transition(SessionState::Faulted);
            } else {
                schedule_recovery(action, 1, &mut current, pending, log);
            }
            *last_health_fault = Some(fault.code);
        }
    }
    sync_traffic(&mut current, traffic);
    persist_snapshot(state_directory, &current);
}

fn schedule_recovery(
    action: RecoveryAction,
    attempt: u32,
    snapshot: &mut ControllerSnapshot,
    pending: &mut Option<PendingRecovery>,
    log: &SharedLog,
) -> bool {
    if attempt > MAX_CONSECUTIVE_RECOVERY_FAILURES {
        snapshot.record_fault(action.fault().clone(), SessionState::Faulted);
        snapshot.next_recovery_unix_ms = None;
        emit(
            log,
            LogEvent::new(
                LogLevel::Error,
                "RECOVERY",
                action.fault().device_id.clone(),
                "recovery_exhausted",
                "Maximum consecutive recovery attempts reached",
            )
            .field("attempts", MAX_CONSECUTIVE_RECOVERY_FAILURES),
        );
        return true;
    }
    let delay = recovery_delay(attempt).expect("bounded recovery delay");
    snapshot.recovery_attempt = attempt;
    snapshot.next_recovery_unix_ms =
        Some(Utc::now().timestamp_millis() + i64::try_from(delay.as_millis()).unwrap_or(i64::MAX));
    snapshot.force_transition(SessionState::Recovering);
    emit(
        log,
        LogEvent::new(
            LogLevel::Warn,
            "RECOVERY",
            action.fault().device_id.clone(),
            "recovery_scheduled",
            format!(
                "Recovery attempt #{attempt} scheduled in {} seconds",
                delay.as_secs()
            ),
        )
        .field("attempt", attempt)
        .field("delay_seconds", delay.as_secs())
        .field("reason_code", action.fault().code.clone()),
    );
    *pending = Some(PendingRecovery {
        action,
        attempt,
        due: Instant::now() + delay,
    });
    false
}

fn lock_snapshot(
    snapshot: &Arc<Mutex<ControllerSnapshot>>,
) -> std::sync::MutexGuard<'_, ControllerSnapshot> {
    snapshot.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn sync_traffic(snapshot: &mut ControllerSnapshot, traffic: &SharedTraffic) {
    traffic
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .apply(&mut snapshot.metrics);
}

fn reset_inactive_snapshot(snapshot: &mut ControllerSnapshot) {
    snapshot.force_transition(SessionState::Idle);
    snapshot.reverse_health = crate::controller::model::HealthState::Unknown;
    snapshot.process_health = crate::controller::model::HealthState::Unknown;
    snapshot.vpn_health = crate::controller::model::HealthState::Unknown;
    snapshot.relay_health = crate::controller::model::HealthState::Unknown;
    snapshot.internet_health = crate::controller::model::HealthState::Unknown;
    snapshot.relay_pid = None;
    snapshot.recovery_attempt = 0;
    snapshot.next_recovery_unix_ms = None;
    snapshot.last_fault = None;
    snapshot.metrics.session_started_unix_ms = None;
}

fn persist_snapshot(state_directory: &Path, snapshot: &ControllerSnapshot) {
    if fs::create_dir_all(state_directory).is_err() {
        return;
    }
    if let Ok(encoded) = serde_json::to_vec_pretty(snapshot) {
        let temporary = state_directory.join("state.json.tmp");
        let target = state_directory.join("state.json");
        if fs::write(&temporary, encoded).is_ok() {
            if target.exists() {
                let _ = fs::remove_file(&target);
            }
            let _ = fs::rename(temporary, target);
        }
    }
}

fn state_directory() -> PathBuf {
    if let Some(value) = std::env::var_os("GNIREHTET_NG_STATE_DIR") {
        return PathBuf::from(value);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("GnirehtetNG")
}

fn artifact_path(variable: &str, filename: &str) -> PathBuf {
    if let Some(value) = std::env::var_os(variable) {
        return PathBuf::from(value);
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(filename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::model::HealthState;
    use std::sync::{Arc, Mutex};

    #[test]
    fn inactive_stop_clears_fault_and_session_health() {
        let mut snapshot = ControllerSnapshot::default();
        snapshot.record_fault(
            Fault::new(
                "usb_disconnected",
                "device unavailable",
                true,
                "ADB",
                "start",
                None,
            ),
            SessionState::Faulted,
        );
        snapshot.vpn_health = HealthState::Unavailable;
        snapshot.recovery_attempt = 3;

        reset_inactive_snapshot(&mut snapshot);

        assert_eq!(SessionState::Idle, snapshot.session_state);
        assert_eq!(HealthState::Unknown, snapshot.vpn_health);
        assert_eq!(0, snapshot.recovery_attempt);
        assert!(snapshot.last_fault.is_none());
    }

    #[test]
    fn recovery_policy_exhausts_after_five_attempts() {
        let directory = std::env::temp_dir().join(format!(
            "gnirehtet-ng-recovery-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos()
        ));
        let log = Arc::new(Mutex::new(LogStore::open(&directory).unwrap()));
        let fault = Fault::new(
            "reverse_missing",
            "reverse mapping disappeared",
            true,
            "ADB",
            "reverse_watchdog",
            Some("device-1".to_string()),
        );
        let action = RecoveryAction::RebuildReverse(fault.clone());
        let mut snapshot = ControllerSnapshot::default();
        let mut pending = None;

        assert!(
            schedule_recovery(action, 6, &mut snapshot, &mut pending, &log),
            "exhaustion must be reported"
        );

        assert_eq!(SessionState::Faulted, snapshot.session_state);
        assert_eq!(
            Some(fault.code),
            snapshot.last_fault.map(|value| value.code)
        );
        assert!(snapshot.next_recovery_unix_ms.is_none());
        assert!(pending.is_none());
        fs::remove_dir_all(directory).unwrap();
    }
}
