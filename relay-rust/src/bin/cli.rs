use relaylib::controller::ipc::{send_request, Request, Response};
use relaylib::controller::session::SessionConfig;
use relaylib::controller::PROTOCOL_VERSION;
use serde_json::{json, Value};
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
};

fn main() {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => {}
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
}

fn run(mut args: Vec<String>) -> Result<(), String> {
    let json_output = take_flag(&mut args, "--json");
    let command = args.first().cloned().unwrap_or_else(|| "help".to_string());
    if !args.is_empty() {
        args.remove(0);
    }
    if command == "help" || command == "--help" || command == "-h" {
        print_usage();
        return Ok(());
    }

    let payload = match command.as_str() {
        "status" | "stop" | "devices" | "diagnose" | "metrics" | "shutdown-core" => {
            ensure_no_args(&args)?;
            json!({})
        }
        "hyperos-status" | "hyperos-protect" | "hyperos-unprotect" => {
            serde_json::to_value(json!({ "serial": args.first().cloned() }))
                .map_err(|error| error.to_string())?
        }
        "start" | "restart" => {
            serde_json::to_value(parse_session_config(args)?).map_err(|error| error.to_string())?
        }
        "logs" => parse_log_filter(args)?,
        other => {
            return Err(format!(
                "unknown command '{other}'\n\nRun gnirehtet-cli help"
            ))
        }
    };
    let request = Request {
        version: PROTOCOL_VERSION,
        id: format!("cli-{}", std::process::id()),
        message_type: match command.as_str() {
            "metrics" => "get_metrics".to_string(),
            "shutdown-core" => "shutdown_core".to_string(),
            "hyperos-status" => "hyperos_status".to_string(),
            "hyperos-protect" => "hyperos_protect".to_string(),
            "hyperos-unprotect" => "hyperos_unprotect".to_string(),
            _ => command.clone(),
        },
        payload,
    };
    let response = request_with_core_start(&request)?;
    if !response.ok {
        let code = response.payload["code"].as_str().unwrap_or("unknown_error");
        let message = response.payload["message"]
            .as_str()
            .unwrap_or("controller command failed");
        return Err(format!("{code}: {message}"));
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&response.payload).map_err(|error| error.to_string())?
        );
    } else {
        print_human(&command, &response.payload)?;
    }
    Ok(())
}

fn request_with_core_start(request: &Request) -> Result<Response, String> {
    if let Ok(response) = send_request(request) {
        return Ok(response);
    }
    start_core()?;
    let mut last_error = None;
    for _ in 0..40 {
        match send_request(request) {
            Ok(response) => return Ok(response),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "controller did not open its named pipe: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn start_core() -> Result<(), String> {
    let core = sibling_executable("gnirehtet-core.exe")?;
    if !core.is_file() {
        return Err(format!(
            "controller executable was not found at {}",
            core.display()
        ));
    }
    let application: Vec<u16> = core
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let startup = STARTUPINFOW {
        cb: mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW | DETACHED_PROCESS,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(format!(
            "could not start {}: {}",
            core.display(),
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

fn sibling_executable(filename: &str) -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("could not locate CLI executable: {error}"))?
        .parent()
        .map(|directory| directory.join(filename))
        .ok_or_else(|| "CLI executable has no parent directory".to_string())
}

fn parse_session_config(mut args: Vec<String>) -> Result<SessionConfig, String> {
    let mut config = SessionConfig {
        auto_reconnect: !take_flag(&mut args, "--no-auto"),
        ..SessionConfig::default()
    };
    let index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-p" | "--port" => {
                config.port = take_value(&mut args, index, "port")?
                    .parse()
                    .map_err(|_| "port must be an integer from 1 to 65535".to_string())?;
                if config.port == 0 {
                    return Err("port must be non-zero".to_string());
                }
            }
            "-d" | "--dns" => {
                config.dns_servers = Some(take_value(&mut args, index, "DNS servers")?);
            }
            "-r" | "--routes" => {
                config.routes = Some(take_value(&mut args, index, "routes")?);
            }
            value if !value.starts_with('-') && config.serial.is_none() => {
                config.serial = Some(args.remove(index));
            }
            value => return Err(format!("unexpected start option '{value}'")),
        }
        if index > args.len() {
            break;
        }
    }
    Ok(config)
}

fn parse_log_filter(mut args: Vec<String>) -> Result<Value, String> {
    let mut limit = 200usize;
    let mut minimum_level: Option<String> = None;
    let mut component: Option<String> = None;
    let mut device_id: Option<String> = None;
    let index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" => {
                limit = take_value(&mut args, index, "log limit")?
                    .parse()
                    .map_err(|_| "log limit must be an integer".to_string())?;
                limit = limit.min(5_000);
            }
            "--level" => {
                minimum_level = Some(take_value(&mut args, index, "log level")?.to_uppercase());
            }
            "--component" => {
                component = Some(take_value(&mut args, index, "component")?.to_uppercase());
            }
            "--device" => device_id = Some(take_value(&mut args, index, "device")?),
            value => return Err(format!("unexpected logs option '{value}'")),
        }
    }
    Ok(json!({
        "limit": limit,
        "minimum_level": minimum_level,
        "component": component,
        "device_id": device_id
    }))
}

fn take_value(args: &mut Vec<String>, index: usize, name: &str) -> Result<String, String> {
    if index + 1 >= args.len() {
        return Err(format!("missing {name}"));
    }
    args.remove(index);
    Ok(args.remove(index))
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(index) = args.iter().position(|value| value == flag) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn ensure_no_args(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected argument '{}'", args[0]))
    }
}

fn print_human(command: &str, payload: &Value) -> Result<(), String> {
    match command {
        "hyperos-status" | "hyperos-protect" | "hyperos-unprotect" => {
            println!(
                "Device:    {}",
                payload["serial"].as_str().unwrap_or("unknown")
            );
            println!(
                "Protected: {}",
                payload["protected"].as_bool().unwrap_or(false)
            );
            if let Some(changed) = payload["changed"].as_bool() {
                println!("Changed:   {}", changed);
            }
            println!(
                "List:      {}",
                payload["raw"].as_str().unwrap_or("(empty)")
            );
            return Ok(());
        }
        "status" | "start" | "stop" | "restart" => {
            println!(
                "State:     {}",
                payload["session_state"].as_str().unwrap_or("unknown")
            );
            println!(
                "Device:    {}",
                payload["selected_device"]["model"]
                    .as_str()
                    .or_else(|| payload["selected_device"]["serial"].as_str())
                    .unwrap_or("none")
            );
            println!(
                "ADB/VPN:   {}/{}",
                payload["adb_state"].as_str().unwrap_or("unknown"),
                payload["vpn_health"].as_str().unwrap_or("unknown")
            );
            println!(
                "Relay/net: {}/{}",
                payload["relay_health"].as_str().unwrap_or("unknown"),
                payload["internet_health"].as_str().unwrap_or("unknown")
            );
            println!("Revision:   {}", payload["revision"]);
            if !payload["last_fault"].is_null() {
                println!(
                    "Last fault: {} — {}",
                    payload["last_fault"]["code"].as_str().unwrap_or("unknown"),
                    payload["last_fault"]["message"].as_str().unwrap_or("")
                );
            }
        }
        "devices" => {
            let devices = payload.as_array().ok_or("invalid devices response")?;
            if devices.is_empty() {
                println!("No ADB devices found.");
            } else {
                println!("STATE          SERIAL                   MODEL");
                for device in devices {
                    println!(
                        "{:<14} {:<24} {}",
                        device["adb_state"].as_str().unwrap_or("unknown"),
                        device["serial"].as_str().unwrap_or(""),
                        device["model"].as_str().unwrap_or("")
                    );
                }
            }
        }
        "logs" => {
            for event in payload.as_array().ok_or("invalid logs response")? {
                println!(
                    "{} {:<5} {:<10} {}",
                    event["timestamp"].as_str().unwrap_or(""),
                    event["level"].as_str().unwrap_or(""),
                    event["component"].as_str().unwrap_or(""),
                    event["message"].as_str().unwrap_or("")
                );
            }
        }
        "diagnose" => println!(
            "Diagnostic bundle: {}",
            payload["path"].as_str().unwrap_or("unknown")
        ),
        "metrics" => {
            println!(
                "Device -> network: {} bytes, {} packets, {} B/s",
                payload["bytes_from_device"],
                payload["packets_from_device"],
                payload["bytes_from_device_per_second"]
            );
            println!(
                "Network -> device: {} bytes, {} packets, {} B/s",
                payload["bytes_to_device"],
                payload["packets_to_device"],
                payload["bytes_to_device_per_second"]
            );
        }
        "shutdown-core" => println!("Controller stopped cleanly."),
        _ => println!("{}", serde_json::to_string_pretty(payload).unwrap()),
    }
    Ok(())
}

fn print_usage() {
    println!(
        "Gnirehtet NG controller CLI\n\n\
Usage:\n  \
gnirehtet-cli [--json] status\n  \
gnirehtet-cli [--json] start [SERIAL] [-p PORT] [-d DNS] [-r ROUTES] [--no-auto]\n  \
gnirehtet-cli [--json] stop\n  \
gnirehtet-cli [--json] restart [SERIAL] [-p PORT] [-d DNS] [-r ROUTES] [--no-auto]\n  \
gnirehtet-cli [--json] devices\n  \
gnirehtet-cli [--json] logs [--limit N] [--level LEVEL] [--component NAME] [--device SERIAL]\n  \
gnirehtet-cli [--json] diagnose\n  \
gnirehtet-cli [--json] metrics\n  \
gnirehtet-cli [--json] shutdown-core"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_options_in_any_order() {
        let config = parse_session_config(vec![
            "-p".into(),
            "32000".into(),
            "device-1".into(),
            "--no-auto".into(),
            "-d".into(),
            "1.1.1.1".into(),
        ])
        .unwrap();
        assert_eq!(Some("device-1"), config.serial.as_deref());
        assert_eq!(32000, config.port);
        assert!(!config.auto_reconnect);
    }

    #[test]
    fn rejects_missing_option_value() {
        assert!(parse_session_config(vec!["-p".into()]).is_err());
    }
}
