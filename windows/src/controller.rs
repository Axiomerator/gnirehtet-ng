use relaylib::controller::{
    PROTOCOL_VERSION,
    ipc::{Request, send_request_checked},
    model::{ControllerSnapshot, DeviceInfo},
    structured_log::LogEvent,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    env, fs, io,
    os::windows::{ffi::OsStringExt, io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{
        Pipes::GetNamedPipeServerProcessId,
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
    },
};

pub enum Job {
    Poll { devices: bool, after: Option<u64> },
    Command(&'static str, Value),
}

pub struct Poll {
    pub snapshot: ControllerSnapshot,
    pub devices: Option<Vec<DeviceInfo>>,
    pub logs: Vec<LogEvent>,
}

pub enum Event {
    Polled(Result<Box<Poll>, String>),
    Completed(&'static str, Result<Value, String>),
}

pub fn spawn() -> (mpsc::Sender<Job>, mpsc::Receiver<Event>) {
    let (jobs, work) = mpsc::channel();
    let (events, results) = mpsc::channel();
    thread::spawn(move || {
        let mut controller = Controller::new();
        while let Ok(job) = work.recv() {
            let event = match job {
                Job::Poll { devices, after } => {
                    Event::Polled(controller.poll(devices, after).map(Box::new))
                }
                Job::Command(name, payload) => {
                    Event::Completed(name, controller.call(name, payload))
                }
            };
            if events.send(event).is_err() {
                break;
            }
        }
    });
    (jobs, results)
}

struct Controller {
    executable: Result<PathBuf, String>,
    launched: bool,
    previous_pid: Option<u32>,
}

impl Controller {
    fn new() -> Self {
        let executable = env::current_exe()
            .and_then(|path| fs::canonicalize(path.with_file_name("gnirehtet-core.exe")))
            .map_err(|error| format!("Could not locate gnirehtet-core.exe: {error}"));
        Self {
            executable,
            launched: false,
            previous_pid: None,
        }
    }

    fn poll(&mut self, devices: bool, after: Option<u64>) -> Result<Poll, String> {
        let snapshot = match self.get("status", json!({})) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if !self.launched {
                    self.start()?;
                    self.launched = true;
                }
                return Err(error);
            }
        };
        let snapshot: ControllerSnapshot = snapshot;
        let core_changed = self.previous_pid != Some(snapshot.core_pid);
        self.previous_pid = Some(snapshot.core_pid);
        self.launched = false;
        let devices = (devices || core_changed)
            .then(|| self.get("devices", json!({})).ok())
            .flatten();
        let logs = after.map_or_else(Vec::new, |sequence| {
            let after_sequence = if core_changed || sequence == 0 {
                None
            } else {
                Some(sequence)
            };
            self.get(
                "logs",
                json!({ "limit": 250, "after_sequence": after_sequence }),
            )
            .unwrap_or_default()
        });
        Ok(Poll {
            snapshot,
            devices,
            logs,
        })
    }

    fn start(&self) -> Result<(), String> {
        let executable = self.executable.as_ref().map_err(Clone::clone)?;
        let directory = executable.parent().expect("core path is absolute");
        let mut command = Command::new(executable);
        command
            .args(["--parent-pid", &std::process::id().to_string()])
            .current_dir(directory)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (variable, relative) in [
            ("ADB", r"adb\adb.exe"),
            ("GNIREHTET_APK", "gnirehtet-ng.apk"),
            ("GNIREHTET_NG_RELAY", "gnirehtet-ng.exe"),
        ] {
            let path = directory.join(relative);
            if env::var_os(variable).is_none() && path.is_file() {
                command.env(variable, path);
            }
        }
        command
            .spawn()
            .map(drop)
            .map_err(|error| format!("Could not launch the controller: {error}"))
    }

    fn get<T: DeserializeOwned>(&self, name: &str, payload: Value) -> Result<T, String> {
        self.call(name, payload).and_then(|response| {
            serde_json::from_value(response).map_err(|error| error.to_string())
        })
    }

    fn call(&self, name: &str, payload: Value) -> Result<Value, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let executable = self.executable.as_ref().map_err(Clone::clone)?;
        let request = Request {
            version: PROTOCOL_VERSION,
            id: format!(
                "slint-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            message_type: name.into(),
            payload,
        };
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        let response = loop {
            match send_request_checked(&request, |pipe| verify_server(pipe, executable)) {
                Err(error)
                    if matches!(error.raw_os_error(), Some(2 | 231))
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(30));
                }
                response => break response,
            }
        }
        .map_err(|error| format!("Controller {name}: {error}"))?;
        if response.version != PROTOCOL_VERSION || response.id != request.id {
            return Err("The controller returned a mismatched response.".into());
        }
        if response.ok {
            Ok(response.payload)
        } else {
            Err(response.payload["message"]
                .as_str()
                .unwrap_or("The controller rejected the request.")
                .into())
        }
    }
}

fn verify_server(pipe: &fs::File, expected: &Path) -> io::Result<()> {
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut name = vec![0; 32_768];
    let mut len = name.len() as u32;
    let result = unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut len) };
    let error = io::Error::last_os_error();
    unsafe {
        CloseHandle(process);
    }
    if result == 0 {
        return Err(error);
    }
    let actual = fs::canonicalize(PathBuf::from(std::ffi::OsString::from_wide(
        &name[..len as usize],
    )))?;
    if !actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe belongs to a different controller executable",
        ));
    }
    Ok(())
}
