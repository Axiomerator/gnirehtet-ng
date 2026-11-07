/*
 * Gnirehtet NG control plane additions.
 *
 * Licensed under the Apache License, Version 2.0.
 */

pub mod adb;
pub mod diagnostics;
pub mod ipc;
pub mod model;
pub mod service;
pub mod session;
pub mod structured_log;

pub const PROTOCOL_VERSION: u32 = 1;
pub const PIPE_NAME: &str = r"\\.\pipe\gnirehtet-ng-v1";
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub const CLIENT_PACKAGE: &str = env!("GNIREHTET_NG_ANDROID_APPLICATION_ID");
pub const CLIENT_ACTIVITY: &str = concat!(
    env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"),
    "/",
    env!("GNIREHTET_NG_ANDROID_ACTIVITY_CLASS")
);
pub const CLIENT_SERVICE: &str = concat!(
    env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"),
    "/com.genymobile.gnirehtet.GnirehtetService"
);
pub const CLIENT_TRANSPARENT_ACTIVITY: &str = concat!(
    env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"),
    "/com.genymobile.gnirehtet.GnirehtetActivity"
);
pub const CLIENT_ACTION_START_VPN: &str =
    concat!(env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"), ".START_VPN");
pub const CLIENT_ACTION_START: &str =
    concat!(env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"), ".START");
pub const CLIENT_ACTION_STOP: &str = concat!(env!("GNIREHTET_NG_ANDROID_APPLICATION_ID"), ".STOP");
pub const REQUIRED_APK_VERSION_CODE: &str = env!("GNIREHTET_NG_ANDROID_VERSION_CODE");
