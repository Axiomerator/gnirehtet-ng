use crate::controller::model::Fault;
use crate::controller::service::ControllerService;
use crate::controller::structured_log::LogLevel;
use crate::controller::{MAX_MESSAGE_BYTES, PIPE_NAME, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_IO_PENDING,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FlushFileBuffers, OPEN_EXISTING};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const PIPE_TYPE_BYTE: u32 = 0;
const PIPE_READMODE_BYTE: u32 = 0;
const PIPE_WAIT: u32 = 0;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const CORE_MUTEX_NAME: &str = r"Local\GnirehtetNgCoreV1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub ok: bool,
    pub payload: Value,
}

impl Response {
    pub fn success(id: String, payload: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            message_type: "result".to_string(),
            ok: true,
            payload,
        }
    }

    pub fn error(id: String, fault: Fault) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            message_type: "error".to_string(),
            ok: false,
            payload: serde_json::to_value(fault).unwrap_or_else(|_| {
                json!({
                    "code": "internal_error",
                    "message": "Could not serialize controller fault"
                })
            }),
        }
    }
}

#[derive(Deserialize)]
struct LogsRequest {
    #[serde(default = "default_log_limit")]
    limit: usize,
    minimum_level: Option<LogLevel>,
    component: Option<String>,
    device_id: Option<String>,
    after_sequence: Option<u64>,
    before_sequence: Option<u64>,
}

fn default_log_limit() -> usize {
    200
}

pub struct CoreInstanceGuard(HANDLE);

impl Drop for CoreInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn acquire_core_instance() -> io::Result<CoreInstanceGuard> {
    let name = wide_null(CORE_MUTEX_NAME);
    let handle = unsafe { CreateMutexW(ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "another Gnirehtet NG controller is already running",
        ));
    }
    Ok(CoreInstanceGuard(handle))
}

pub fn serve(service: ControllerService) -> io::Result<()> {
    loop {
        match serve_one(&service) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => eprintln!("IPC request failed: {error}"),
        }
    }
}

fn serve_one(service: &ControllerService) -> io::Result<bool> {
    let pipe = create_server_pipe()?;
    let connected = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) };
    if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
        return Err(io::Error::last_os_error());
    }

    let (response, shutdown) = read_request(&pipe)
        .map(|line| dispatch_line(&line, service))
        .unwrap_or_else(|error| {
            (
                Response::error(
                    String::new(),
                    Fault::new(
                        if error.kind() == io::ErrorKind::InvalidData {
                            "message_too_large"
                        } else {
                            "invalid_request"
                        },
                        error.to_string(),
                        false,
                        "CORE",
                        "ipc_read",
                        None,
                    ),
                ),
                false,
            )
        });
    write_response(&pipe, &response)?;
    // DisconnectNamedPipe discards buffered data the client has not read yet, so wait
    // (bounded) for the client to consume the response before disconnecting. A client
    // that never reads only delays this request by the timeout instead of wedging the
    // single-threaded pipe server forever.
    unsafe {
        let event = CreateEventW(ptr::null(), 1, 0, ptr::null());
        if !event.is_null() {
            let mut overlapped: OVERLAPPED = mem::zeroed();
            overlapped.hEvent = event;
            if FlushFileBuffers(pipe.as_raw_handle()) == 0 && GetLastError() == ERROR_IO_PENDING {
                let _ = WaitForSingleObject(event, 2_000);
            }
            let _ = CloseHandle(event);
        }
        DisconnectNamedPipe(pipe.as_raw_handle());
    }
    Ok(shutdown)
}

pub fn process_line(line: &str, service: &ControllerService) -> Response {
    dispatch_line(line, service).0
}

fn dispatch_line(line: &str, service: &ControllerService) -> (Response, bool) {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return (
                Response::error(
                    String::new(),
                    Fault::new(
                        "invalid_request",
                        format!("Request is not valid JSON: {error}"),
                        false,
                        "CORE",
                        "ipc_parse",
                        None,
                    ),
                ),
                false,
            )
        }
    };
    if request.version != PROTOCOL_VERSION {
        return (
            Response::error(
                request.id,
                Fault::new(
                    "ipc_version_mismatch",
                    format!(
                        "Protocol version {} is unsupported; expected {PROTOCOL_VERSION}",
                        request.version
                    ),
                    false,
                    "CORE",
                    "ipc_version",
                    None,
                ),
            ),
            false,
        );
    }
    if let Err(fault) = validate_request(&request) {
        return (Response::error(request.id, fault), false);
    }

    let mut shutdown = false;
    let result: Result<Value, Fault> = match request.message_type.as_str() {
        "get_status" | "status" => serde_json::to_value(service.snapshot()).map_err(json_fault),
        "get_metrics" => serde_json::to_value(service.snapshot().metrics).map_err(json_fault),
        "get_devices" | "devices" => service
            .devices()
            .and_then(|devices| serde_json::to_value(devices).map_err(json_fault)),
        "start" => parse_payload(request.payload)
            .and_then(|config| service.start_session(config))
            .and_then(|snapshot| serde_json::to_value(snapshot).map_err(json_fault)),
        "stop" => service
            .stop_session()
            .and_then(|snapshot| serde_json::to_value(snapshot).map_err(json_fault)),
        "restart" => parse_payload(request.payload)
            .and_then(|config| service.restart_session(config))
            .and_then(|snapshot| serde_json::to_value(snapshot).map_err(json_fault)),
        "hyperos_status" | "hyperos_protect" | "hyperos_unprotect" => {
            let serial = request
                .payload
                .as_object()
                .and_then(|object| object.get("serial"))
                .and_then(|value| value.as_str())
                .map(String::from);
            match request.message_type.as_str() {
                "hyperos_status" => service.hyperos_status(serial),
                "hyperos_protect" => service.hyperos_protect(serial),
                _ => service.hyperos_unprotect(serial),
            }
        }
        "get_logs" | "logs" => parse_payload::<LogsRequest>(request.payload).and_then(|filter| {
            let events = service.logs(
                filter.limit,
                filter.minimum_level,
                filter.component.as_deref(),
                filter.device_id.as_deref(),
                filter.after_sequence,
                filter.before_sequence,
            );
            serde_json::to_value(fit_logs_response(events)).map_err(json_fault)
        }),
        "export_diagnostics" | "diagnose" => service.diagnose().map(|path| {
            json!({
                "path": path.display().to_string()
            })
        }),
        "shutdown_core" => service.shutdown().and_then(|snapshot| {
            shutdown = true;
            serde_json::to_value(snapshot).map_err(json_fault)
        }),
        other => Err(Fault::new(
            "unsupported_command",
            format!("Command '{other}' is not implemented"),
            false,
            "CORE",
            "dispatch",
            None,
        )),
    };
    let response = match result {
        Ok(payload) => Response::success(request.id, payload),
        Err(fault) => Response::error(request.id, fault),
    };
    (response, shutdown)
}

fn validate_request(request: &Request) -> Result<(), Fault> {
    if request.id.chars().count() > 256 {
        return Err(Fault::new(
            "invalid_request",
            "Request id exceeds 256 characters",
            false,
            "CORE",
            "ipc_validate",
            None,
        ));
    }
    if !request.payload.is_object() {
        return Err(Fault::new(
            "invalid_request",
            "Request payload must be a JSON object",
            false,
            "CORE",
            "ipc_validate",
            None,
        ));
    }
    Ok(())
}

fn parse_payload<T>(value: Value) -> Result<T, Fault>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(value).map_err(|error| {
        Fault::new(
            "invalid_request",
            format!("Invalid command payload: {error}"),
            false,
            "CORE",
            "ipc_payload",
            None,
        )
    })
}

fn json_fault(error: serde_json::Error) -> Fault {
    Fault::new(
        "internal_error",
        format!("Could not serialize controller response: {error}"),
        false,
        "CORE",
        "ipc_serialize",
        None,
    )
}

fn fit_logs_response(
    mut events: Vec<crate::controller::structured_log::LogEvent>,
) -> Vec<crate::controller::structured_log::LogEvent> {
    const PAYLOAD_BUDGET: usize = MAX_MESSAGE_BYTES - 1024;
    let fit = |count: usize| -> bool {
        serde_json::to_vec(&events[events.len() - count..])
            .map(|encoded| encoded.len() <= PAYLOAD_BUDGET)
            .unwrap_or(false)
    };
    if events.is_empty() || fit(events.len()) {
        return events;
    }
    let mut low = 0usize;
    let mut high = events.len();
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if fit(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    events.split_off(events.len() - low)
}

pub fn send_request(request: &Request) -> io::Result<Response> {
    send_request_checked(request, |_| Ok(()))
}

pub fn send_request_checked(
    request: &Request,
    verify_peer: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<Response> {
    let pipe = open_client_pipe()?;
    verify_peer(&pipe)?;
    let mut encoded = serde_json::to_vec(request)?;
    if encoded.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request exceeds the 64 KiB IPC limit",
        ));
    }
    encoded.push(b'\n');
    (&pipe).write_all(&encoded)?;
    (&pipe).flush()?;
    let line = read_request(&pipe)?;
    serde_json::from_str(&line).map_err(io::Error::other)
}

fn create_server_pipe() -> io::Result<File> {
    let name = wide_null(PIPE_NAME);
    let security_descriptor = current_user_security_descriptor()?;
    let mut security_attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor.0,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_MESSAGE_BYTES as u32,
            MAX_MESSAGE_BYTES as u32,
            0,
            &mut security_attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

struct LocalAllocation(HLOCAL);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

fn current_user_security_descriptor() -> io::Result<LocalAllocation> {
    let sid = current_user_sid_string()?;
    let sddl = wide_null(&format!("D:P(A;;GA;;;{sid})"));
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(LocalAllocation(descriptor))
    }
}

fn current_user_sid_string() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = CoreInstanceGuard(token);

    let mut required = 0u32;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let word_size = mem::size_of::<usize>();
    let mut buffer = vec![0usize; (required as usize).div_ceil(word_size)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut string_sid = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let string_sid_allocation = LocalAllocation(string_sid.cast());
    let mut length = 0usize;
    while unsafe { *string_sid.add(length) } != 0 {
        length += 1;
    }
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(string_sid, length) })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "current user SID is invalid"));
    drop(string_sid_allocation);
    value
}

fn open_client_pipe() -> io::Result<File> {
    let name = wide_null(PIPE_NAME);
    unsafe {
        WaitNamedPipeW(name.as_ptr(), 1_000);
    }
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

fn read_request(pipe: &File) -> io::Result<String> {
    let mut line = String::new();
    let mut reader = BufReader::new(pipe);
    let count = reader
        .by_ref()
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_line(&mut line)?;
    if count > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC message exceeds 64 KiB",
        ));
    }
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "IPC peer closed before sending a request",
        ));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn write_response(pipe: &File, response: &Response) -> io::Result<()> {
    let mut encoded = serde_json::to_vec(response)?;
    if encoded.len() > MAX_MESSAGE_BYTES {
        encoded = serde_json::to_vec(&Response::error(
            response.id.clone(),
            Fault::new(
                "response_too_large",
                "Controller response exceeds the 64 KiB IPC limit; request fewer logs",
                false,
                "CORE",
                "ipc_write",
                None,
            ),
        ))?;
    }
    encoded.push(b'\n');
    let mut writer = pipe;
    writer.write_all(&encoded)?;
    writer.flush()
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_error_preserves_stable_fault_code() {
        let response = Response::error(
            "test".to_string(),
            Fault::new(
                "adb_timeout",
                "deadline exceeded",
                true,
                "ADB",
                "test",
                None,
            ),
        );
        assert!(!response.ok);
        assert_eq!("adb_timeout", response.payload["code"]);
    }

    #[test]
    fn current_user_pipe_security_descriptor_is_constructible() {
        let sid = current_user_sid_string().unwrap();
        assert!(sid.starts_with("S-1-"));
        let descriptor = current_user_security_descriptor().unwrap();
        assert!(!descriptor.0.is_null());
    }

    #[test]
    fn request_validation_matches_schema_bounds() {
        let mut request = Request {
            version: PROTOCOL_VERSION,
            id: "x".repeat(257),
            message_type: "status".to_string(),
            payload: json!({}),
        };
        assert_eq!(
            "invalid_request",
            validate_request(&request).unwrap_err().code
        );
        request.id = "valid".to_string();
        request.payload = json!([]);
        assert_eq!(
            "invalid_request",
            validate_request(&request).unwrap_err().code
        );

        let extra = r#"{"version":1,"id":"x","type":"status","payload":{},"extra":true}"#;
        assert!(serde_json::from_str::<Request>(extra).is_err());
    }

    #[test]
    fn logs_response_is_trimmed_to_fit_the_ipc_budget() {
        use crate::controller::structured_log::LogEvent;
        use crate::controller::structured_log::LogLevel;
        let big_message = "x".repeat(4 * 1024);
        let events: Vec<_> = (0..64)
            .map(|index| {
                LogEvent::new(
                    LogLevel::Info,
                    "RELAY",
                    None,
                    &format!("event-{index}"),
                    &big_message,
                )
            })
            .collect();
        let fitted = fit_logs_response(events);
        assert!(!fitted.is_empty());
        assert!(fitted.len() < 64);
        assert_eq!("event-63", fitted.last().unwrap().event_type);
        let encoded = serde_json::to_vec(&fitted).unwrap();
        assert!(encoded.len() <= MAX_MESSAGE_BYTES - 1024);
    }

    #[test]
    fn logs_response_keeps_short_payloads_untouched() {
        use crate::controller::structured_log::LogEvent;
        use crate::controller::structured_log::LogLevel;
        let events: Vec<_> = (0..4)
            .map(|index| {
                LogEvent::new(
                    LogLevel::Info,
                    "CORE",
                    None,
                    &format!("event-{index}"),
                    "small",
                )
            })
            .collect();
        assert_eq!(4, fit_logs_response(events).len());
    }
}
