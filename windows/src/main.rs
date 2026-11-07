#![windows_subsystem = "windows"]

mod controller;

use controller::{Event, Job, Poll};
use relaylib::controller::{
    model::{AdbState, ApkState, ControllerSnapshot, DeviceInfo, HealthState, SessionState},
    structured_log::{LogEvent, LogLevel},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use slint::{
    CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode,
    VecModel, Weak,
};
use std::{
    cell::RefCell,
    collections::HashSet,
    env, fs, io,
    path::PathBuf,
    process::Command,
    rc::Rc,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

slint::include_modules!();

const LOG_CAPACITY: usize = 50_000;
const VISIBLE_LOG_LIMIT: usize = 2_000;
const SOURCES: [&str; 10] = [
    "", "CORE", "ADB", "DEVICE", "RELAY", "ANDROID", "VPN", "NETWORK", "RECOVERY", "UI",
];

#[derive(Deserialize, Serialize)]
#[serde(default)]
struct Settings {
    port: i32,
    dns_servers: String,
    routes: String,
    auto_reconnect: bool,
    minimize_to_tray: bool,
    appearance: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 31416,
            dns_servers: String::new(),
            routes: String::new(),
            auto_reconnect: true,
            minimize_to_tray: false,
            appearance: 0,
        }
    }
}

impl Settings {
    fn load() -> io::Result<Self> {
        match fs::read(state_dir()?.join("ui-settings.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    fn apply(&self, ui: &MainWindow) {
        ui.set_port(self.port.clamp(1024, 65535));
        ui.set_dns(self.dns_servers.as_str().into());
        ui.set_routes(self.routes.as_str().into());
        ui.set_auto_reconnect(self.auto_reconnect);
        ui.set_minimize_to_tray(self.minimize_to_tray);
        ui.set_appearance(self.appearance.clamp(0, 2));
    }

    fn from_ui(ui: &MainWindow) -> Self {
        Self {
            port: ui.get_port(),
            dns_servers: ui.get_dns().trim().into(),
            routes: ui.get_routes().trim().into(),
            auto_reconnect: ui.get_auto_reconnect(),
            minimize_to_tray: ui.get_minimize_to_tray(),
            appearance: ui.get_appearance(),
        }
    }

    fn save(&self) -> io::Result<()> {
        let directory = state_dir()?;
        fs::create_dir_all(&directory)?;
        let temporary = directory.join(format!("ui-settings-{}.tmp", std::process::id()));
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, directory.join("ui-settings.json"))
    }
}

struct App {
    jobs: mpsc::Sender<Job>,
    results: mpsc::Receiver<Event>,
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
    snapshot: Option<ControllerSnapshot>,
    logs: Vec<LogEvent>,
    visible: Vec<usize>,
    selected_logs: HashSet<u64>,
    sequence: u64,
    pending: Option<&'static str>,
    refresh_devices: bool,
    polling: bool,
    ticks: u32,
    tray: Weak<Tray>,
}

impl App {
    fn new(tray: &Tray) -> Self {
        let (jobs, results) = controller::spawn();
        Self {
            jobs,
            results,
            devices: Vec::new(),
            selected: None,
            snapshot: None,
            logs: Vec::new(),
            visible: Vec::new(),
            selected_logs: HashSet::new(),
            sequence: 0,
            pending: None,
            refresh_devices: true,
            polling: false,
            ticks: 0,
            tray: tray.as_weak(),
        }
    }

    fn tick(&mut self, ui: &MainWindow) {
        while let Ok(event) = self.results.try_recv() {
            match event {
                Event::Polled(result) => {
                    self.polling = false;
                    match result {
                        Ok(poll) => self.update(ui, *poll),
                        Err(error) => {
                            ui.set_ready(false);
                            ui.set_session("Controller unavailable".into());
                            ui.set_session_tone(2);
                            ui.set_summary(error.into());
                        }
                    }
                }
                Event::Completed(name, result) => {
                    ui.set_busy(false);
                    match result {
                        Ok(payload) if name == "diagnose" => {
                            ui.set_diagnostics_path(
                                payload["path"].as_str().unwrap_or_default().into(),
                            );
                            message(ui, "Diagnostics bundle exported.", 1);
                        }
                        Ok(payload) => {
                            if let Ok(snapshot) = serde_json::from_value(payload) {
                                self.snapshot = Some(snapshot);
                                self.render_status(ui);
                            }
                        }
                        Err(error) => message(ui, error, 3),
                    }
                    self.refresh_devices = true;
                }
            }
        }

        if let Some(tray) = self.tray.upgrade() {
            tray.set_status(ui.get_session());
            tray.set_start_enabled(
                ui.get_ready() && !ui.get_busy() && !ui.get_running() && !ui.get_waiting(),
            );
            tray.set_stop_enabled(!ui.get_busy() && (ui.get_running() || ui.get_waiting()));
            tray.set_restart_enabled(ui.get_ready() && !ui.get_busy() && ui.get_running());
        }
        self.ticks = self.ticks.wrapping_add(1);
        if !self.polling && !ui.get_busy() && (self.refresh_devices || self.ticks.is_multiple_of(4))
        {
            let devices =
                self.refresh_devices || self.ticks.is_multiple_of(16) || self.pending.is_some();
            self.refresh_devices = false;
            let after = (!ui.get_paused()).then_some(self.sequence);
            self.polling = self.jobs.send(Job::Poll { devices, after }).is_ok();
        }
    }

    fn update(&mut self, ui: &MainWindow, poll: Poll) {
        let new_core = self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.core_pid != poll.snapshot.core_pid);
        self.snapshot = Some(poll.snapshot);
        ui.set_ready(true);
        if let Some(devices) = poll.devices {
            if self.devices != devices || new_core {
                self.devices = devices;
                self.render_devices(ui);
            }
        }
        if new_core {
            self.sequence = 0;
            self.logs.clear();
            self.selected_logs.clear();
        }
        if (!ui.get_paused() && !poll.logs.is_empty()) || new_core {
            if let Some(last) = poll.logs.last() {
                self.sequence = last.sequence;
            }
            self.logs.extend(poll.logs);
            if self.logs.len() > LOG_CAPACITY {
                for event in self.logs.drain(..self.logs.len() - LOG_CAPACITY) {
                    self.selected_logs.remove(&event.sequence);
                }
            }
            self.render_logs(ui, true);
        }
        if let Some(command) = self.pending {
            if self.device_ready() {
                self.pending = None;
                self.action(ui, command);
            }
        }
        self.render_status(ui);
    }

    fn device_ready(&self) -> bool {
        self.devices.iter().any(|device| {
            device.adb_state == AdbState::Ready
                && self
                    .selected
                    .as_ref()
                    .is_none_or(|serial| serial == &device.serial)
        })
    }

    fn render_devices(&mut self, ui: &MainWindow) {
        let index = self.selected.as_ref().and_then(|serial| {
            self.devices
                .iter()
                .position(|device| &device.serial == serial)
        });
        let disconnected = self.selected.as_ref().filter(|_| index.is_none());
        ui.set_device_names(model(
            std::iter::once(SharedString::from("Automatic device selection")).chain(
                self.devices
                    .iter()
                    .map(|device| format!("{} · {}", device_name(device), device.serial).into())
                    .chain(disconnected.map(|serial| format!("{serial} · disconnected").into())),
            ),
        ));
        ui.set_device_details(model(self.devices.iter().map(|device| {
            Reading {
                label: device_name(device).into(),
                value: adb_label(&device.adb_state).into(),
                detail: format!(
                    "{}  ·  {}",
                    device.serial,
                    device.android_version.as_ref().map_or_else(
                        || "Android version unavailable".into(),
                        |version| format!("Android {version}")
                    )
                )
                .into(),
                tone: adb_tone(&device.adb_state),
            }
        })));
        ui.set_device_index(index.map_or_else(
            || {
                if disconnected.is_some() {
                    self.devices.len() as i32 + 1
                } else {
                    0
                }
            },
            |index| index as i32 + 1,
        ));
    }

    fn select_device(&mut self, ui: &MainWindow, index: i32) {
        if index > self.devices.len() as i32 {
            return;
        }
        self.selected = index
            .checked_sub(1)
            .and_then(|index| self.devices.get(index as usize))
            .map(|device| device.serial.clone());
        ui.set_device_index(index);
        self.render_status(ui);
    }

    fn render_status(&self, ui: &MainWindow) {
        let authorized = self
            .devices
            .iter()
            .filter(|device| device.adb_state == AdbState::Ready)
            .count();
        let hint = if let Some(serial) = self
            .selected
            .as_ref()
            .filter(|serial| !self.devices.iter().any(|device| &device.serial == *serial))
        {
            format!(
                "Selected device {serial} is disconnected. Choose Automatic to use another device."
            )
        } else if self.devices.is_empty() {
            "Connect your Android device by USB and allow USB debugging.".into()
        } else {
            format!(
                "{authorized} authorized  ·  {} detected",
                self.devices.len()
            )
        };
        ui.set_device_hint(hint.into());
        ui.set_waiting(self.pending.is_some());
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let (state, summary, tone) = if self.pending.is_some() {
            (
                "Waiting for USB",
                "The connection will start when the selected device is authorized.",
                2,
            )
        } else {
            session_label(&snapshot.session_state)
        };
        if ui.get_ready() {
            ui.set_session(state.into());
            let detail = snapshot.selected_device.as_ref().map_or_else(
                || summary.into(),
                |device| format!("{}  ·  {summary}", device_name(device)),
            );
            ui.set_summary(detail.into());
            ui.set_session_tone(tone);
        }
        ui.set_running(snapshot.session_state != SessionState::Idle);
        ui.set_fault(
            snapshot
                .last_fault
                .as_ref()
                .map_or_else(SharedString::default, |fault| {
                    format!("{} · {}", fault.code, fault.message).into()
                }),
        );
        ui.set_controller_detail(
            format!(
                "Controller PID {}  ·  Snapshot {}",
                snapshot.core_pid, snapshot.revision
            )
            .into(),
        );
        let metrics = &snapshot.metrics;
        let uptime = metrics
            .session_started_unix_ms
            .map(|start| {
                let seconds = (now().as_millis() as i64).saturating_sub(start).max(0) / 1000;
                format!(
                    "{:02}:{:02}:{:02}",
                    seconds / 3600,
                    seconds / 60 % 60,
                    seconds % 60
                )
            })
            .unwrap_or_else(|| "—".into());
        ui.set_metrics(model([
            reading(
                "To Android",
                &format!("{}/s", bytes(metrics.bytes_to_device_per_second)),
                &format!("{} total", bytes(metrics.bytes_to_device)),
                0,
            ),
            reading(
                "From Android",
                &format!("{}/s", bytes(metrics.bytes_from_device_per_second)),
                &format!("{} total", bytes(metrics.bytes_from_device)),
                0,
            ),
            reading(
                "Session uptime",
                &uptime,
                &format!("Relay port {}", snapshot.relay_port),
                0,
            ),
            reading(
                "Reconnects",
                &metrics.reconnect_count.to_string(),
                &format!("{} recovery attempts", metrics.recovery_attempts),
                0,
            ),
        ]));
        let displayed_adb = if snapshot.session_state == SessionState::Idle {
            self.devices
                .iter()
                .find(|device| {
                    self.selected
                        .as_ref()
                        .is_none_or(|serial| serial == &device.serial)
                })
                .map_or(&snapshot.adb_state, |device| &device.adb_state)
        } else {
            &snapshot.adb_state
        };
        ui.set_health(model([
            reading(
                "USB",
                if matches!(snapshot.adb_state, AdbState::NoDevice | AdbState::Unknown)
                    && self.devices.is_empty()
                {
                    "Waiting"
                } else {
                    "Detected"
                },
                "A device is visible to the ADB server.",
                0,
            ),
            reading(
                "ADB",
                adb_label(displayed_adb),
                "Android debugging connection and authorization.",
                adb_tone(displayed_adb),
            ),
            reading(
                "App",
                match snapshot.apk_state {
                    ApkState::Ready => "Ready",
                    ApkState::Missing => "Missing",
                    ApkState::Unknown => "—",
                },
                "Gnirehtet NG Android client.",
                if snapshot.apk_state == ApkState::Ready {
                    1
                } else {
                    0
                },
            ),
            health("Tunnel", &snapshot.reverse_health, "ADB reverse socket."),
            health(
                "Process",
                &snapshot.process_health,
                "Android client process.",
            ),
            health("VPN", &snapshot.vpn_health, "Android VPN interface."),
            health(
                "Relay",
                &snapshot.relay_health,
                "Traffic relay on this computer.",
            ),
            health(
                "Internet",
                &snapshot.internet_health,
                "Connectivity through the reverse tunnel.",
            ),
        ]));
    }

    fn action(&mut self, ui: &MainWindow, action: &str) {
        match action {
            "dismiss" => ui.set_message("".into()),
            "refresh" => self.refresh_devices = true,
            "save" => self.save(ui, true),
            "clear" => {
                self.logs.clear();
                self.selected_logs.clear();
                self.render_logs(ui, true);
            }
            "copy-selected" | "copy-all" | "export-log" => self.output_logs(ui, action),
            "show-diagnostics" => {
                let path = ui.get_diagnostics_path();
                if let Err(error) = Command::new("explorer.exe")
                    .arg(format!("/select,{path}"))
                    .spawn()
                {
                    message(ui, error.to_string(), 3);
                }
            }
            "quit" => {
                let _ = slint::quit_event_loop();
            }
            "stop" if self.pending.take().is_some() => self.render_status(ui),
            "start" | "stop" | "restart" | "diagnose" if !ui.get_busy() && ui.get_ready() => {
                let command = match action {
                    "start" => "start",
                    "stop" => "stop",
                    "restart" => "restart",
                    _ => "diagnose",
                };
                if matches!(command, "start" | "restart") && !self.device_ready() {
                    if ui.get_auto_reconnect() {
                        self.pending = Some(command);
                        self.refresh_devices = true;
                        self.render_status(ui);
                    } else {
                        message(ui, "Connect and authorize the selected Android device.", 2);
                    }
                    return;
                }
                let payload = if matches!(command, "start" | "restart") {
                    let settings = Settings::from_ui(ui);
                    json!({
                        "serial": self.selected,
                        "port": settings.port,
                        "dns_servers": nonempty(&settings.dns_servers),
                        "routes": nonempty(&settings.routes),
                        "auto_reconnect": settings.auto_reconnect,
                    })
                } else {
                    json!({})
                };
                ui.set_message("".into());
                if self.jobs.send(Job::Command(command, payload)).is_ok() {
                    ui.set_busy(true);
                } else {
                    message(ui, "The controller worker has stopped.", 3);
                }
            }
            _ => {}
        }
    }

    fn save(&mut self, ui: &MainWindow, explicit: bool) {
        let settings = Settings::from_ui(ui);
        if !settings.auto_reconnect && self.pending.take().is_some() {
            self.render_status(ui);
        }
        if let Some(tray) = self.tray.upgrade() {
            let _ = if settings.minimize_to_tray {
                tray.show()
            } else {
                tray.hide()
            };
        }
        match settings.save() {
            Ok(()) if explicit => {
                message(ui, "Saved. Session defaults apply on Start or Restart.", 1)
            }
            Ok(()) => {}
            Err(error) => message(ui, format!("Could not save settings: {error}"), 3),
        }
    }

    fn render_logs(&mut self, ui: &MainWindow, new_events: bool) {
        let minimum = [
            None,
            Some(LogLevel::Trace),
            Some(LogLevel::Debug),
            Some(LogLevel::Info),
            Some(LogLevel::Warn),
            Some(LogLevel::Error),
        ]
        .get(ui.get_level() as usize)
        .copied()
        .flatten();
        let source = SOURCES
            .get(ui.get_component_filter() as usize)
            .copied()
            .unwrap_or_default();
        let serial = ui.get_serial_filter().trim().to_ascii_lowercase();
        let search = ui.get_search_filter().trim().to_lowercase();
        self.visible =
            self.logs
                .iter()
                .enumerate()
                .filter(|(_, event)| {
                    minimum.is_none_or(|level| event.level >= level)
                        && (source.is_empty() || event.component == source)
                        && (serial.is_empty()
                            || event.device_id.as_deref().is_some_and(|device| {
                                device.to_ascii_lowercase().contains(&serial)
                            }))
                        && (search.is_empty() || log_text(event).to_lowercase().contains(&search))
                })
                .take(VISIBLE_LOG_LIMIT)
                .map(|(index, _)| index)
                .collect();
        self.visible.reverse();
        ui.set_log_rows(model(self.visible.iter().map(|&index| {
            let event = &self.logs[index];
            log_line(event, self.selected_logs.contains(&event.sequence))
        })));
        if new_events {
            ui.set_recent(model(
                self.logs
                    .iter()
                    .rev()
                    .take(5)
                    .map(|event| log_line(event, false)),
            ));
            ui.set_log_update(ui.get_log_update().wrapping_add(1));
        }
    }

    fn select_log(&mut self, ui: &MainWindow, index: usize) {
        if let Some(&log_index) = self.visible.get(index) {
            let event = &self.logs[log_index];
            let selected = if self.selected_logs.remove(&event.sequence) {
                false
            } else {
                self.selected_logs.insert(event.sequence);
                true
            };
            ui.get_log_rows()
                .set_row_data(index, log_line(event, selected));
        }
    }

    fn output_logs(&self, ui: &MainWindow, action: &str) {
        let lines = self
            .visible
            .iter()
            .map(|&index| &self.logs[index])
            .filter(|event| {
                action != "copy-selected" || self.selected_logs.contains(&event.sequence)
            })
            .map(log_text)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            message(
                ui,
                "There are no matching log entries to copy or export.",
                2,
            );
            return;
        }
        let contents = lines.join("\r\n");
        let result = if action == "export-log" {
            (|| -> io::Result<String> {
                let directory = state_dir()?.join("exports");
                fs::create_dir_all(&directory)?;
                let path = directory.join(format!("gnirehtet-log-{}.txt", now().as_millis()));
                fs::write(&path, contents)?;
                Ok(format!("Exported to {}", path.display()))
            })()
            .map_err(|error| error.to_string())
        } else {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(contents))
                .map(|()| format!("Copied {} log entries.", lines.len()))
                .map_err(|error| error.to_string())
        };
        match result {
            Ok(result) => message(ui, result, 1),
            Err(error) => message(ui, error, 3),
        }
    }
}

fn main() {
    if let Err(error) = run() {
        let path = env::temp_dir().join("gnirehtet-ng-gui-startup-error.txt");
        let _ = fs::write(path, error.to_string());
        std::process::exit(1);
    }
}

fn run() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let ui = MainWindow::new()?;
    let tray = Tray::new()?;
    let app = Rc::new(RefCell::new(App::new(&tray)));
    let product = include_str!("../../config/product.properties");
    ui.set_version(
        product
            .lines()
            .find_map(|line| line.strip_prefix("productVersion="))
            .unwrap_or("dev")
            .into(),
    );
    match Settings::load() {
        Ok(settings) => settings.apply(&ui),
        Err(error) => message(&ui, format!("Could not read saved settings: {error}"), 2),
    }
    if ui.get_minimize_to_tray() {
        tray.show()?;
    }
    app.borrow_mut().render_devices(&ui);
    let weak = ui.as_weak();
    let model = app.clone();
    ui.on_action(move |action| {
        if let Some(ui) = weak.upgrade() {
            model.borrow_mut().action(&ui, &action);
        }
    });
    let weak = ui.as_weak();
    ui.on_device_selected({
        let app = app.clone();
        move |index| {
            if let Some(ui) = weak.upgrade() {
                app.borrow_mut().select_device(&ui, index);
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_preference_changed({
        let app = app.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                app.borrow_mut().save(&ui, false);
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_filter_changed({
        let app = app.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                app.borrow_mut().render_logs(&ui, true);
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_log_selected({
        let app = app.clone();
        move |index| {
            if let Some(ui) = weak.upgrade() {
                app.borrow_mut().select_log(&ui, index as usize);
            }
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if weak.upgrade().is_some_and(|ui| !ui.get_minimize_to_tray()) {
            let _ = slint::quit_event_loop();
        }
        CloseRequestResponse::HideWindow
    });
    let weak = ui.as_weak();
    tray.on_open(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.show();
        }
    });
    let weak = ui.as_weak();
    tray.on_action(move |action| {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_action(action);
        }
    });
    let timer = Timer::default();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(ui) = weak.upgrade() {
            app.borrow_mut().tick(&ui);
        }
    });
    ui.run()
}

fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from_iter(items))
}

fn message(ui: &MainWindow, text: impl Into<SharedString>, tone: i32) {
    ui.set_message(text.into());
    ui.set_message_tone(tone);
}

fn state_dir() -> io::Result<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(|path| PathBuf::from(path).join("GnirehtetNg"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LocalAppData is unavailable"))
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn device_name(device: &DeviceInfo) -> &str {
    device
        .model
        .as_deref()
        .or(device.product.as_deref())
        .unwrap_or("Android device")
}

fn bytes(value: u64) -> String {
    let mut amount = value as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if amount < 1024.0 || unit == "TiB" {
            return if unit == "B" {
                format!("{value} B")
            } else {
                format!("{amount:.1} {unit}")
            };
        }
        amount /= 1024.0;
    }
    unreachable!()
}

fn session_label(state: &SessionState) -> (&'static str, &'static str, i32) {
    match state {
        SessionState::Idle => (
            "Ready to connect",
            "Start when your Android device is ready.",
            0,
        ),
        SessionState::Preparing => (
            "Preparing",
            "Checking the Android app and USB debugging.",
            2,
        ),
        SessionState::Starting => ("Starting", "Starting the relay and USB connection.", 2),
        SessionState::ReverseMissing => (
            "USB tunnel missing",
            "The reverse socket needs to be restored.",
            2,
        ),
        SessionState::TunnelReady => ("USB tunnel ready", "Starting the Android VPN.", 1),
        SessionState::ClientStarting => {
            ("Starting Android app", "Waiting for the Android client.", 2)
        }
        SessionState::VpnPermissionRequired => (
            "VPN approval needed",
            "Approve the VPN request on Android.",
            2,
        ),
        SessionState::VpnReady => ("VPN ready", "Waiting for the relay connection.", 1),
        SessionState::RelayConnected => ("Relay connected", "Waiting for the Internet check.", 1),
        SessionState::InternetReady => ("Internet ready", "Android traffic is routed over USB.", 1),
        SessionState::Stopping => ("Stopping", "Closing the VPN and USB tunnel.", 2),
        SessionState::Recovering => (
            "Recovering",
            "The controller is restoring the connection.",
            2,
        ),
        SessionState::Faulted => (
            "Needs attention",
            "See the connection checks or Activity log.",
            3,
        ),
    }
}

fn adb_label(state: &AdbState) -> &'static str {
    match state {
        AdbState::Unknown => "—",
        AdbState::NoDevice => "No device",
        AdbState::UsbFound => "USB found",
        AdbState::Unauthorized => "Authorize",
        AdbState::Offline => "Offline",
        AdbState::Ready => "Authorized",
    }
}

fn adb_tone(state: &AdbState) -> i32 {
    match state {
        AdbState::Ready => 1,
        AdbState::Offline | AdbState::Unauthorized => 2,
        _ => 0,
    }
}

fn reading(label: &str, value: &str, detail: &str, tone: i32) -> Reading {
    Reading {
        label: label.into(),
        value: value.into(),
        detail: detail.into(),
        tone,
    }
}

fn health(label: &str, state: &HealthState, detail: &str) -> Reading {
    let (value, tone) = match state {
        HealthState::Unknown => ("—", 0),
        HealthState::Ready => ("Ready", 1),
        HealthState::Degraded => ("Degraded", 2),
        HealthState::Unavailable => ("Unavailable", 3),
    };
    reading(label, value, detail, tone)
}

fn log_text(event: &LogEvent) -> String {
    format!(
        "{}  {}  {}  {}  {}  {}",
        event.timestamp,
        level_name(event.level),
        event.component,
        event.device_id.as_deref().unwrap_or("—"),
        event.event_type,
        event.message
    )
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

fn log_line(event: &LogEvent, selected: bool) -> LogLine {
    LogLine {
        time: event
            .timestamp
            .get(11..23)
            .unwrap_or(&event.timestamp)
            .into(),
        level: level_name(event.level).into(),
        source: event.component.as_str().into(),
        message: event.message.as_str().into(),
        detail: log_text(event).into(),
        selected,
        tone: match event.level {
            LogLevel::Error => 3,
            LogLevel::Warn => 2,
            LogLevel::Info => 1,
            _ => 0,
        },
    }
}
