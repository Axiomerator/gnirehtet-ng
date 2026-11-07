use crate::controller::adb::AdbManager;
use crate::controller::model::{ControllerSnapshot, Fault};
use crate::controller::structured_log::SharedLog;
use chrono::Utc;
use serde_json::json;
use std::convert::TryFrom;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_DIAGNOSTIC_FILE_BYTES: usize = 2 * 1024 * 1024;

pub fn export_diagnostics(
    state_directory: &Path,
    snapshot: &ControllerSnapshot,
    adb: &AdbManager,
    log: &SharedLog,
) -> Result<PathBuf, Fault> {
    let destination_directory = state_directory.join("diagnostics");
    fs::create_dir_all(&destination_directory).map_err(diagnostic_io_fault)?;
    let destination = destination_directory.join(format!(
        "diagnostics-{}.zip",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));

    let serial = snapshot
        .selected_device
        .as_ref()
        .map(|device| device.serial.as_str());
    let state = serde_json::to_vec_pretty(snapshot).map_err(diagnostic_json_fault)?;
    let versions = serde_json::to_vec_pretty(&json!({
        "product": "Gnirehtet NG",
        "core_version": env!("CARGO_PKG_VERSION"),
        "protocol_version": crate::controller::PROTOCOL_VERSION,
        "android_package": crate::controller::CLIENT_PACKAGE,
        "required_apk_version_code": crate::controller::REQUIRED_APK_VERSION_CODE,
        "adb_executable": adb.executable().display().to_string(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
    .map_err(diagnostic_json_fault)?;
    let settings = serde_json::to_vec_pretty(&json!({
        "selected_serial": serial.map(redact_serial),
        "relay_port": snapshot.relay_port,
        "auto_reconnect": snapshot.auto_reconnect,
    }))
    .map_err(diagnostic_json_fault)?;

    let adb_devices = capture(|| {
        adb.execute(None, ["devices", "-l"], "diagnose_devices")
            .map(|output| output.stdout)
    });
    let adb_reverse = capture(|| match serial {
        Some(serial) => adb
            .execute(Some(serial), ["reverse", "--list"], "diagnose_reverse")
            .map(|output| output.stdout),
        None => Ok("No selected device".to_string()),
    });
    let exit_info = capture(|| match serial {
        Some(serial) => adb.application_exit_info(serial),
        None => Ok("No selected device".to_string()),
    });
    let android_network = capture(|| match serial {
        Some(serial) => adb.shell_capture(
            serial,
            ["dumpsys", "connectivity"],
            "diagnose_android_network",
        ),
        None => Ok("No selected device".to_string()),
    });
    let android_logcat = capture(|| match serial {
        Some(serial) => {
            adb.shell_capture(serial, ["logcat", "-d", "-t", "2000"], "diagnose_logcat")
        }
        None => Ok("No selected device".to_string()),
    });

    let windows_log = {
        let store = log.lock().map_err(|_| {
            Fault::new(
                "internal_error",
                "Structured log lock is poisoned",
                false,
                "CORE",
                "export_diagnostics",
                serial.map(str::to_string),
            )
        })?;
        read_capped(&store.directory().join("windows.log"))
            .unwrap_or_else(|error| format!("Could not read windows.log: {error}").into_bytes())
    };

    let files = vec![
        ("windows.log", windows_log),
        ("android-logcat.txt", android_logcat.into_bytes()),
        ("state.json", state),
        ("versions.json", versions),
        ("adb-devices.txt", adb_devices.into_bytes()),
        ("adb-reverse.txt", adb_reverse.into_bytes()),
        ("android-exit-info.txt", exit_info.into_bytes()),
        ("android-network.txt", android_network.into_bytes()),
        ("settings-redacted.json", settings),
    ];
    write_store_zip(&destination, &files).map_err(diagnostic_io_fault)?;
    Ok(destination)
}

fn capture<F>(operation: F) -> String
where
    F: FnOnce() -> Result<String, Fault>,
{
    match operation() {
        Ok(value) => value.chars().take(MAX_DIAGNOSTIC_FILE_BYTES).collect(),
        Err(fault) => format!("{}: {}", fault.code, fault.message),
    }
}

fn redact_serial(serial: &str) -> String {
    let suffix: String = serial
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("***{suffix}")
}

fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    if bytes.len() <= MAX_DIAGNOSTIC_FILE_BYTES {
        Ok(bytes)
    } else {
        Ok(bytes[bytes.len() - MAX_DIAGNOSTIC_FILE_BYTES..].to_vec())
    }
}

struct CentralEntry {
    name: Vec<u8>,
    crc32: u32,
    size: u32,
    offset: u32,
}

fn write_store_zip(path: &Path, files: &[(&str, Vec<u8>)]) -> io::Result<()> {
    let mut output = File::create(path)?;
    let mut offset = 0u32;
    let mut entries = Vec::new();
    for (name, data) in files {
        let name = name.as_bytes().to_vec();
        let size = u32::try_from(data.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "diagnostic file too large"))?;
        let crc32 = crc32(data);
        let mut header = Vec::with_capacity(30 + name.len());
        push_u32(&mut header, 0x0403_4b50);
        push_u16(&mut header, 20);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, crc32);
        push_u32(&mut header, size);
        push_u32(&mut header, size);
        push_u16(&mut header, name.len() as u16);
        push_u16(&mut header, 0);
        header.extend_from_slice(&name);
        output.write_all(&header)?;
        output.write_all(data)?;
        entries.push(CentralEntry {
            name,
            crc32,
            size,
            offset,
        });
        offset = offset
            .checked_add(header.len() as u32)
            .and_then(|value| value.checked_add(size))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ZIP offset overflow"))?;
    }

    let central_offset = offset;
    for entry in &entries {
        let mut header = Vec::with_capacity(46 + entry.name.len());
        push_u32(&mut header, 0x0201_4b50);
        push_u16(&mut header, 20);
        push_u16(&mut header, 20);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, entry.crc32);
        push_u32(&mut header, entry.size);
        push_u32(&mut header, entry.size);
        push_u16(&mut header, entry.name.len() as u16);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, 0);
        push_u32(&mut header, entry.offset);
        header.extend_from_slice(&entry.name);
        output.write_all(&header)?;
        offset += header.len() as u32;
    }
    let central_size = offset - central_offset;
    let mut end = Vec::with_capacity(22);
    push_u32(&mut end, 0x0605_4b50);
    push_u16(&mut end, 0);
    push_u16(&mut end, 0);
    push_u16(&mut end, entries.len() as u16);
    push_u16(&mut end, entries.len() as u16);
    push_u32(&mut end, central_size);
    push_u32(&mut end, central_offset);
    push_u16(&mut end, 0);
    output.write_all(&end)?;
    output.flush()
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn diagnostic_io_fault(error: io::Error) -> Fault {
    Fault::new(
        "diagnostic_export_failed",
        format!("Could not write diagnostic bundle: {error}"),
        false,
        "CORE",
        "export_diagnostics",
        None,
    )
}

fn diagnostic_json_fault(error: serde_json::Error) -> Fault {
    Fault::new(
        "diagnostic_export_failed",
        format!("Could not serialize diagnostic data: {error}"),
        false,
        "CORE",
        "export_diagnostics",
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_valid_store_zip_structure() {
        let path = std::env::temp_dir().join(format!(
            "gnirehtet-ng-diagnostic-{}-{}.zip",
            std::process::id(),
            Utc::now().timestamp_nanos()
        ));
        write_store_zip(&path, &[("state.json", b"{}".to_vec())]).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(&0x0403_4b50u32.to_le_bytes()));
        assert!(bytes
            .windows(4)
            .any(|value| value == 0x0201_4b50u32.to_le_bytes()));
        assert!(bytes.ends_with(&[0, 0]));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn crc_matches_standard_vector() {
        assert_eq!(0xcbf4_3926, crc32(b"123456789"));
    }
}
