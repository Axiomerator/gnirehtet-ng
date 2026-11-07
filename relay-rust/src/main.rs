/*
 * Copyright (C) 2017 Genymobile
 * Modified for Gnirehtet NG by the Gnirehtet NG contributors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

extern crate chrono;
extern crate ctrlc;
#[macro_use]
extern crate log;
extern crate relaylib;

mod adb_monitor;
mod cli_args;
mod execution_error;
mod logger;

use crate::adb_monitor::AdbMonitor;
use crate::cli_args::CommandLineArguments;
use crate::execution_error::{Cmd, CommandExecutionError, ProcessIoError, ProcessStatusError};
use relaylib::subprocess::configure_background;
use std::env;
use std::process::{self, exit};
use std::thread;
use std::time::Duration;

const TAG: &str = "Main";
const CLIENT_PACKAGE: &str = env!("GNIREHTET_NG_ANDROID_APPLICATION_ID");
const CLIENT_ACTIVITY: &str = concat!(
    env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"),
    "/",
    env!("GNIREHTET_NG_ANDROID_ACTIVITY_CLASS")
);
const CLIENT_ACTION_START: &str = concat!(env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"), ".START");
const CLIENT_ACTION_STOP: &str = concat!(env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"), ".STOP");
const REQUIRED_APK_VERSION_CODE: &str = env!("GNIREHTET_NG_ANDROID_VERSION_CODE");

#[inline]
fn get_adb_path() -> String {
    if let Some(env_adb) = std::env::var_os("ADB") {
        env_adb.into_string().expect("invalid ADB value")
    } else {
        "adb".to_string()
    }
}

#[inline]
fn get_apk_path() -> String {
    if let Some(env_adb) = std::env::var_os("GNIREHTET_APK") {
        env_adb.into_string().expect("invalid GNIREHTET_APK value")
    } else {
        "gnirehtet-ng.apk".to_string()
    }
}

const COMMANDS: &[&dyn Command] = &[
    &InstallCommand,
    &UninstallCommand,
    &ReinstallCommand,
    &RunCommand,
    &AutorunCommand,
    &StartCommand,
    &AutostartCommand,
    &StopCommand,
    &RestartCommand,
    &TunnelCommand,
    &RelayCommand,
];

trait Command {
    fn command(&self) -> &'static str;
    fn accepted_parameters(&self) -> u8;
    fn description(&self) -> &'static str;
    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError>;
}

struct InstallCommand;
struct UninstallCommand;
struct ReinstallCommand;
struct RunCommand;
struct AutorunCommand;
struct StartCommand;
struct AutostartCommand;
struct StopCommand;
struct RestartCommand;
struct TunnelCommand;
struct RelayCommand;

impl Command for InstallCommand {
    fn command(&self) -> &'static str {
        "install"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
    }

    fn description(&self) -> &'static str {
        "Install the client on the Android device and exit.\n\
         If several devices are connected via adb, then serial must be\n\
         specified."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_install(args.serial())
    }
}

impl Command for UninstallCommand {
    fn command(&self) -> &'static str {
        "uninstall"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
    }

    fn description(&self) -> &'static str {
        "Uninstall the client from the Android device and exit.\n\
         If several devices are connected via adb, then serial must be\n\
         specified."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_uninstall(args.serial())
    }
}

impl Command for ReinstallCommand {
    fn command(&self) -> &'static str {
        "reinstall"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
    }

    fn description(&self) -> &'static str {
        "Uninstall then install."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_reinstall(args.serial())
    }
}

impl Command for RunCommand {
    fn command(&self) -> &'static str {
        "run"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
            | cli_args::PARAM_DNS_SERVERS
            | cli_args::PARAM_ROUTES
            | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Enable reverse tethering for exactly one device:\n  \
         - install the client if necessary;\n  \
         - start the client;\n  \
         - start the relay server;\n  \
         - on Ctrl+C, stop both the relay server and the client."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_run(
            args.serial(),
            args.dns_servers(),
            args.routes(),
            args.port(),
        )
    }
}

impl Command for AutorunCommand {
    fn command(&self) -> &'static str {
        "autorun"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_DNS_SERVERS | cli_args::PARAM_ROUTES | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Enable reverse tethering for all devices:\n  \
         - monitor devices and start clients (autostart);\n  \
         - start the relay server."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_autorun(args.dns_servers(), args.routes(), args.port())
    }
}

impl Command for StartCommand {
    fn command(&self) -> &'static str {
        "start"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
            | cli_args::PARAM_DNS_SERVERS
            | cli_args::PARAM_ROUTES
            | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Start a client on the Android device and exit.\n\
         If several devices are connected via adb, then serial must be\n\
         specified.\n\
         If -d is given, then make the Android device use the specified\n\
         DNS server(s). Otherwise, use 8.8.8.8 (Google public DNS).\n\
         If -r is given, then only reverse tether the specified routes.\n\
         Otherwise, use 0.0.0.0/0 (redirect the whole traffic).\n\
         If -p is given, then make the relay server listen on the specified\n\
         port. Otherwise, use port 31416.\n\
         If the client is already started, then do nothing, and ignore\n\
         the other parameters.\n\
         10.0.2.2 is mapped to the host 'localhost'."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_start(
            args.serial(),
            args.dns_servers(),
            args.routes(),
            args.port(),
        )
    }
}

impl Command for AutostartCommand {
    fn command(&self) -> &'static str {
        "autostart"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_DNS_SERVERS | cli_args::PARAM_ROUTES | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Listen for device connexions and start a client on every detected\n\
         device.\n\
         Accept the same parameters as the start command (excluding the\n\
         serial, which will be taken from the detected device)."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_autostart(args.dns_servers(), args.routes(), args.port())
    }
}

impl Command for StopCommand {
    fn command(&self) -> &'static str {
        "stop"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
    }

    fn description(&self) -> &'static str {
        "Stop the client on the Android device and exit.\n\
         If several devices are connected via adb, then serial must be\n\
         specified."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_stop(args.serial())
    }
}

impl Command for RestartCommand {
    fn command(&self) -> &'static str {
        "restart"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL
            | cli_args::PARAM_DNS_SERVERS
            | cli_args::PARAM_ROUTES
            | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Stop then start."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_stop(args.serial())?;
        cmd_start(
            args.serial(),
            args.dns_servers(),
            args.routes(),
            args.port(),
        )?;
        Ok(())
    }
}

impl Command for TunnelCommand {
    fn command(&self) -> &'static str {
        "tunnel"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_SERIAL | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Set up the 'adb reverse' tunnel.\n\
         If a device is unplugged then plugged back while gnirehtet is\n\
         active, resetting the tunnel is sufficient to get the\n\
         connection back."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_tunnel(args.serial(), args.port())
    }
}

impl Command for RelayCommand {
    fn command(&self) -> &'static str {
        "relay"
    }

    fn accepted_parameters(&self) -> u8 {
        cli_args::PARAM_NONE | cli_args::PARAM_PORT
    }

    fn description(&self) -> &'static str {
        "Start the relay server in the current terminal."
    }

    fn execute(&self, args: &CommandLineArguments) -> Result<(), CommandExecutionError> {
        cmd_relay(args.port())?;
        Ok(())
    }
}

fn cmd_install(serial: Option<&str>) -> Result<(), CommandExecutionError> {
    info!(target: TAG, "Installing Gnirehtet NG client...");
    exec_adb(serial, vec!["install".into(), "-r".into(), get_apk_path()])
}

fn cmd_uninstall(serial: Option<&str>) -> Result<(), CommandExecutionError> {
    info!(target: TAG, "Uninstalling Gnirehtet NG client...");
    exec_adb(serial, vec!["uninstall", CLIENT_PACKAGE])
}

fn cmd_reinstall(serial: Option<&str>) -> Result<(), CommandExecutionError> {
    cmd_uninstall(serial)?;
    cmd_install(serial)?;
    Ok(())
}

fn cmd_run(
    serial: Option<&str>,
    dns_servers: Option<&str>,
    routes: Option<&str>,
    port: u16,
) -> Result<(), CommandExecutionError> {
    // start in parallel so that the relay server is ready when the client connects
    async_start(serial, dns_servers, routes, port);

    let ctrlc_serial = serial.map(String::from);
    ctrlc::set_handler(move || {
        info!(target: TAG, "Interrupted");

        let serial = ctrlc_serial.as_ref().map(String::as_ref);
        if let Err(err) = cmd_stop(serial) {
            error!(target: TAG, "Cannot stop client: {}", err);
        }

        exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    cmd_relay(port)
}

fn cmd_autorun(
    dns_servers: Option<&str>,
    routes: Option<&str>,
    port: u16,
) -> Result<(), CommandExecutionError> {
    {
        let autostart_dns_servers = dns_servers.map(String::from);
        let autostart_routes = routes.map(String::from);
        thread::spawn(move || {
            let dns_servers = autostart_dns_servers.as_ref().map(String::as_ref);
            let routes = autostart_routes.as_ref().map(String::as_ref);
            if let Err(err) = cmd_autostart(dns_servers, routes, port) {
                error!(target: TAG, "Cannot auto start clients: {}", err);
            }
        });
    }

    cmd_relay(port)
}

fn cmd_start(
    serial: Option<&str>,
    dns_servers: Option<&str>,
    routes: Option<&str>,
    port: u16,
) -> Result<(), CommandExecutionError> {
    if must_install_client(serial)? {
        cmd_install(serial)?;
        // wait a bit after the app is installed so that intent actions are correctly
        // registered
        thread::sleep(Duration::from_millis(500));
    }

    info!(target: TAG, "Starting Gnirehtet NG client...");
    cmd_tunnel(serial, port)?;

    exec_adb(serial, create_start_adb_args(dns_servers, routes))
}

fn cmd_autostart(
    dns_servers: Option<&str>,
    routes: Option<&str>,
    port: u16,
) -> Result<(), CommandExecutionError> {
    let start_dns_servers = dns_servers.map(String::from);
    let start_routes = routes.map(String::from);
    let mut adb_monitor = AdbMonitor::new(Box::new(move |serial: &str| {
        let dns_servers = start_dns_servers.as_ref().map(String::as_ref);
        let routes = start_routes.as_ref().map(String::as_ref);
        async_start(Some(serial), dns_servers, routes, port)
    }));
    adb_monitor.monitor();
    Ok(())
}

fn cmd_stop(serial: Option<&str>) -> Result<(), CommandExecutionError> {
    info!(target: TAG, "Stopping Gnirehtet NG client...");
    exec_adb(
        serial,
        vec![
            "shell",
            "am",
            "start",
            "-a",
            CLIENT_ACTION_STOP,
            "-n",
            CLIENT_ACTIVITY,
        ],
    )
}

fn cmd_tunnel(serial: Option<&str>, port: u16) -> Result<(), CommandExecutionError> {
    exec_adb(
        serial,
        vec![
            "reverse",
            "localabstract:gnirehtet",
            format!("tcp:{}", port).as_str(),
        ],
    )
}

fn cmd_relay(port: u16) -> Result<(), CommandExecutionError> {
    info!(target: TAG, "Starting relay server on port {}...", port);
    relaylib::relay(port)?;
    Ok(())
}

fn async_start(serial: Option<&str>, dns_servers: Option<&str>, routes: Option<&str>, port: u16) {
    let start_serial = serial.map(String::from);
    let start_dns_servers = dns_servers.map(String::from);
    let start_routes = routes.map(String::from);
    thread::spawn(move || {
        let serial = start_serial.as_ref().map(String::as_ref);
        let dns_servers = start_dns_servers.as_ref().map(String::as_ref);
        let routes = start_routes.as_ref().map(String::as_ref);
        if let Err(err) = cmd_start(serial, dns_servers, routes, port) {
            error!(target: TAG, "Cannot start client: {}", err);
        }
    });
}

fn create_adb_args<S: Into<String>>(serial: Option<&str>, args: Vec<S>) -> Vec<String> {
    let mut command = Vec::<String>::new();
    if let Some(serial) = serial {
        command.push("-s".into());
        command.push(serial.to_string());
    }
    for arg in args {
        command.push(arg.into());
    }
    command
}

fn create_start_adb_args(dns_servers: Option<&str>, routes: Option<&str>) -> Vec<String> {
    let mut adb_args = vec![
        "shell".to_string(),
        "am".to_string(),
        "start".to_string(),
        "-a".to_string(),
        CLIENT_ACTION_START.to_string(),
        "-n".to_string(),
        CLIENT_ACTIVITY.to_string(),
    ];
    if let Some(dns_servers) = dns_servers {
        adb_args.extend(["--esa".into(), "dnsServers".into(), dns_servers.into()]);
    }
    if let Some(routes) = routes {
        adb_args.extend(["--esa".into(), "routes".into(), routes.into()]);
    }
    adb_args
}

fn exec_adb<S: Into<String>>(
    serial: Option<&str>,
    args: Vec<S>,
) -> Result<(), CommandExecutionError> {
    let adb_args = create_adb_args(serial, args);
    let adb = get_adb_path();
    debug!(target: TAG, "Execute: {:?} {:?}", adb, adb_args);
    let mut command = process::Command::new(&adb);
    command.args(&adb_args[..]);
    configure_background(&mut command);
    match command.status() {
        Ok(exit_status) => {
            if exit_status.success() {
                Ok(())
            } else {
                let cmd = Cmd::new(adb, adb_args);
                Err(ProcessStatusError::new(cmd, exit_status).into())
            }
        }
        Err(err) => {
            let cmd = Cmd::new(adb, adb_args);
            Err(ProcessIoError::new(cmd, err).into())
        }
    }
}

fn must_install_client(serial: Option<&str>) -> Result<bool, CommandExecutionError> {
    info!(target: TAG, "Checking Gnirehtet NG client...");
    let args = create_adb_args(serial, vec!["shell", "dumpsys", "package", CLIENT_PACKAGE]);
    let adb = get_adb_path();
    debug!(target: TAG, "Execute: {:?} {:?}", adb, args);
    let mut command = process::Command::new(&adb);
    command.args(&args[..]);
    configure_background(&mut command);
    match command.output() {
        Ok(output) => {
            if output.status.success() {
                // the "regex" crate makes the binary far bigger, so just parse the versionCode
                // manually
                let dumpsys = String::from_utf8_lossy(&output.stdout[..]);
                // read the versionCode of the installed package
                Ok(parse_installed_version_code(&dumpsys) != Some(REQUIRED_APK_VERSION_CODE))
            } else {
                let cmd = Cmd::new(adb, args);
                Err(ProcessStatusError::new(cmd, output.status).into())
            }
        }
        Err(err) => {
            let cmd = Cmd::new(adb, args);
            Err(ProcessIoError::new(cmd, err).into())
        }
    }
}

fn parse_installed_version_code(dumpsys: &str) -> Option<&str> {
    dumpsys.lines().find_map(|line| {
        line.trim()
            .strip_prefix("versionCode=")
            .and_then(|value| value.split_whitespace().next())
    })
}

fn print_usage() {
    let mut msg = "Syntax: gnirehtet-ng (".to_string();
    msg.push_str(COMMANDS[0].command());
    for command in &COMMANDS[1..] {
        msg.push('|');
        msg.push_str(command.command());
    }
    msg.push_str(") ...\n");
    for &command in COMMANDS {
        msg.push('\n');
        append_command_usage(&mut msg, command);
    }
    eprint!("{}", msg);
}

fn append_command_usage(msg: &mut String, command: &dyn Command) {
    msg.push_str("  gnirehtet-ng ");
    msg.push_str(command.command());
    let accepted_parameters = command.accepted_parameters();
    if (accepted_parameters & cli_args::PARAM_SERIAL) != 0 {
        msg.push_str(" [serial]");
    }
    if (accepted_parameters & cli_args::PARAM_DNS_SERVERS) != 0 {
        msg.push_str(" [-d DNS[,DNS2,...]]");
    }
    if (accepted_parameters & cli_args::PARAM_PORT) != 0 {
        msg.push_str(" [-p PORT]");
    }
    if (accepted_parameters & cli_args::PARAM_ROUTES) != 0 {
        msg.push_str(" [-r ROUTE[,ROUTE2,...]]");
    }
    msg.push('\n');
    for desc_line in command.description().split('\n') {
        msg.push_str("      ");
        msg.push_str(desc_line);
        msg.push('\n');
    }
}

fn print_command_usage(command: &dyn Command) {
    let mut msg = String::new();
    append_command_usage(&mut msg, command);
    eprint!("{}", msg);
}

fn main() {
    logger::init().unwrap();
    let mut args = env::args();
    // args.nth(1) will consume the two first arguments (the binary name and the command name)
    if let Some(command_name) = args.nth(1) {
        let command = COMMANDS
            .iter()
            .find(|&&command| command.command() == command_name);
        match command {
            Some(&command) => {
                // args now contains only the command parameters
                let arguments =
                    CommandLineArguments::parse(command.accepted_parameters(), args.collect());
                match arguments {
                    Ok(arguments) => {
                        if let Err(err) = command.execute(&arguments) {
                            error!(target: TAG, "Execution error: {}", err);
                            exit(3);
                        }
                    }
                    Err(err) => {
                        error!(target: TAG, "{}", err);
                        print_command_usage(command);
                        exit(2);
                    }
                }
            }
            None => {
                if command_name == "rt" {
                    error!(
                        target: TAG,
                        "The 'rt' command has been renamed to 'run'. Try 'gnirehtet-ng run' instead."
                    );
                    print_command_usage(&RunCommand);
                } else {
                    error!(target: TAG, "Unknown command: {}", command_name);
                    print_usage();
                }
                exit(1);
            }
        }
    } else {
        print_usage();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_identity_is_loaded_from_shared_configuration() {
        assert_eq!("org.gnirehtetng.client", CLIENT_PACKAGE);
        assert_eq!("org.gnirehtetng.client.START", CLIENT_ACTION_START);
        assert_eq!("org.gnirehtetng.client.STOP", CLIENT_ACTION_STOP);
        assert_eq!(
            "org.gnirehtetng.client/com.genymobile.gnirehtet.MainActivity",
            CLIENT_ACTIVITY
        );
        let configured_version = include_str!("../../config/product.properties")
            .lines()
            .find_map(|line| line.strip_prefix("androidVersionCode="))
            .expect("product configuration defines the Android version code");
        assert_eq!(configured_version, REQUIRED_APK_VERSION_CODE);
    }

    #[test]
    fn start_arguments_use_ng_identity_and_optional_network_configuration() {
        let args = create_start_adb_args(Some("1.1.1.1,8.8.8.8"), Some("0.0.0.0/0"));
        assert_eq!(
            vec![
                "shell",
                "am",
                "start",
                "-a",
                "org.gnirehtetng.client.START",
                "-n",
                "org.gnirehtetng.client/com.genymobile.gnirehtet.MainActivity",
                "--esa",
                "dnsServers",
                "1.1.1.1,8.8.8.8",
                "--esa",
                "routes",
                "0.0.0.0/0",
            ],
            args
        );
    }

    #[test]
    fn installed_version_parser_handles_android_dumpsys_output() {
        let dumpsys = "Packages:\n  Package [org.gnirehtetng.client]\n    versionCode=10 minSdk=21 targetSdk=36\n";
        assert_eq!(Some("10"), parse_installed_version_code(dumpsys));
        assert_eq!(None, parse_installed_version_code("Unable to find package"));
    }

    #[test]
    fn serial_is_inserted_before_adb_command_arguments() {
        assert_eq!(
            vec!["-s", "device-1", "reverse", "--list"],
            create_adb_args(Some("device-1"), vec!["reverse", "--list"])
        );
    }
}
