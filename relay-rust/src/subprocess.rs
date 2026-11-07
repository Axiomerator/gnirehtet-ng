/*
 * Gnirehtet NG background process helpers.
 *
 * Licensed under the Apache License, Version 2.0.
 */

use std::process::Command;

/// Configure a child process that is an implementation detail of the desktop app.
///
/// A GUI-launched controller has no console of its own. Without CREATE_NO_WINDOW,
/// Windows may create a short-lived console window every time it polls ADB or
/// starts the relay. Keep the default behavior on other platforms.
pub fn configure_background(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use windows_sys::Win32::System::Console::GetConsoleWindow;

    const PROBE_ENV: &str = "GNIREHTET_NG_CONSOLE_PROBE";

    #[test]
    fn background_children_do_not_allocate_a_console() {
        if std::env::var_os(PROBE_ENV).is_some() {
            assert!(unsafe { GetConsoleWindow() }.is_null());
            return;
        }

        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "subprocess::tests::background_children_do_not_allocate_a_console",
                "--nocapture",
            ])
            .env(PROBE_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        configure_background(&mut command);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "background console probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
