use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdbState {
    Unknown,
    NoDevice,
    UsbFound,
    Unauthorized,
    Offline,
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApkState {
    Unknown,
    Missing,
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Preparing,
    Starting,
    ReverseMissing,
    TunnelReady,
    ClientStarting,
    VpnPermissionRequired,
    VpnReady,
    RelayConnected,
    InternetReady,
    Stopping,
    Recovering,
    Faulted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Unknown,
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub serial: String,
    pub adb_state: AdbState,
    pub product: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub component: String,
    pub operation: String,
    pub device_id: Option<String>,
    #[serde(default)]
    pub details: Map<String, Value>,
}

impl Fault {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        component: impl Into<String>,
        operation: impl Into<String>,
        device_id: Option<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            component: component.into(),
            operation: operation.into(),
            device_id,
            details: Map::new(),
        }
    }

    pub fn with_detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_string(), value.into());
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub session_started_unix_ms: Option<i64>,
    pub reconnect_count: u64,
    pub recovery_attempts: u64,
    pub recovery_successes: u64,
    pub adb_timeouts: u64,
    pub reverse_repairs: u64,
    pub client_restarts: u64,
    pub relay_restarts: u64,
    pub watchdog_ticks: u64,
    pub bytes_from_device: u64,
    pub bytes_to_device: u64,
    pub packets_from_device: u64,
    pub packets_to_device: u64,
    pub bytes_from_device_per_second: u64,
    pub bytes_to_device_per_second: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrafficTotals {
    pub bytes_from_device: u64,
    pub bytes_to_device: u64,
    pub packets_from_device: u64,
    pub packets_to_device: u64,
}

#[derive(Debug, Default)]
pub struct TrafficTracker {
    totals: TrafficTotals,
    last_totals: Option<TrafficTotals>,
    last_sampled_at: Option<Instant>,
    bytes_from_device_per_second: u64,
    bytes_to_device_per_second: u64,
}

pub type SharedTraffic = Arc<Mutex<TrafficTracker>>;

impl TrafficTracker {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, totals: TrafficTotals) {
        self.update_at(totals, Instant::now());
    }

    fn update_at(&mut self, totals: TrafficTotals, sampled_at: Instant) {
        if let (Some(previous), Some(previous_at)) = (self.last_totals, self.last_sampled_at) {
            let elapsed = sampled_at.saturating_duration_since(previous_at);
            if !elapsed.is_zero() {
                let nanos = elapsed.as_nanos();
                self.bytes_from_device_per_second = rate_per_second(
                    totals
                        .bytes_from_device
                        .saturating_sub(previous.bytes_from_device),
                    nanos,
                );
                self.bytes_to_device_per_second = rate_per_second(
                    totals
                        .bytes_to_device
                        .saturating_sub(previous.bytes_to_device),
                    nanos,
                );
            }
        }
        self.totals = totals;
        self.last_totals = Some(totals);
        self.last_sampled_at = Some(sampled_at);
    }

    pub fn apply(&self, metrics: &mut Metrics) {
        metrics.bytes_from_device = self.totals.bytes_from_device;
        metrics.bytes_to_device = self.totals.bytes_to_device;
        metrics.packets_from_device = self.totals.packets_from_device;
        metrics.packets_to_device = self.totals.packets_to_device;
        metrics.bytes_from_device_per_second = self.bytes_from_device_per_second;
        metrics.bytes_to_device_per_second = self.bytes_to_device_per_second;
    }
}

fn rate_per_second(bytes: u64, elapsed_nanos: u128) -> u64 {
    if elapsed_nanos == 0 {
        return 0;
    }
    let rate = u128::from(bytes).saturating_mul(1_000_000_000) / elapsed_nanos;
    rate.min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControllerSnapshot {
    pub revision: u64,
    pub core_pid: u32,
    pub adb_state: AdbState,
    pub apk_state: ApkState,
    pub session_state: SessionState,
    pub reverse_health: HealthState,
    pub process_health: HealthState,
    pub vpn_health: HealthState,
    pub relay_health: HealthState,
    pub internet_health: HealthState,
    pub selected_device: Option<DeviceInfo>,
    pub relay_pid: Option<u32>,
    pub relay_port: u16,
    pub auto_reconnect: bool,
    pub recovery_attempt: u32,
    pub next_recovery_unix_ms: Option<i64>,
    pub last_fault: Option<Fault>,
    pub metrics: Metrics,
}

impl Default for ControllerSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            core_pid: std::process::id(),
            adb_state: AdbState::Unknown,
            apk_state: ApkState::Unknown,
            session_state: SessionState::Idle,
            reverse_health: HealthState::Unknown,
            process_health: HealthState::Unknown,
            vpn_health: HealthState::Unknown,
            relay_health: HealthState::Unknown,
            internet_health: HealthState::Unknown,
            selected_device: None,
            relay_pid: None,
            relay_port: 31416,
            auto_reconnect: true,
            recovery_attempt: 0,
            next_recovery_unix_ms: None,
            last_fault: None,
            metrics: Metrics::default(),
        }
    }
}

impl ControllerSnapshot {
    pub fn transition(&mut self, expected_revision: u64, next: SessionState) -> bool {
        if self.revision != expected_revision {
            return false;
        }
        self.session_state = next;
        self.revision += 1;
        true
    }

    pub fn force_transition(&mut self, next: SessionState) -> u64 {
        self.session_state = next;
        self.revision += 1;
        self.revision
    }

    pub fn record_fault(&mut self, fault: Fault, state: SessionState) {
        self.last_fault = Some(fault);
        self.force_transition(state);
    }
}

pub const RECOVERY_BACKOFF_SECONDS: &[u64] = &[1, 2, 5, 10, 30];
pub const MAX_CONSECUTIVE_RECOVERY_FAILURES: u32 = 5;

pub fn recovery_delay(attempt: u32) -> Option<std::time::Duration> {
    if attempt == 0 || attempt > MAX_CONSECUTIVE_RECOVERY_FAILURES {
        return None;
    }
    Some(std::time::Duration::from_secs(
        RECOVERY_BACKOFF_SECONDS[(attempt - 1) as usize],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_transition_is_ignored() {
        let mut snapshot = ControllerSnapshot::default();
        assert!(snapshot.transition(0, SessionState::Preparing));
        assert!(!snapshot.transition(0, SessionState::InternetReady));
        assert_eq!(SessionState::Preparing, snapshot.session_state);
        assert_eq!(1, snapshot.revision);
    }

    #[test]
    fn recovery_schedule_is_bounded() {
        let seconds: Vec<_> = (1..=5)
            .map(|attempt| recovery_delay(attempt).unwrap().as_secs())
            .collect();
        assert_eq!(vec![1, 2, 5, 10, 30], seconds);
        assert!(recovery_delay(0).is_none());
        assert!(recovery_delay(6).is_none());
    }

    #[test]
    fn traffic_tracker_computes_rates_and_handles_counter_reset() {
        let start = Instant::now();
        let mut tracker = TrafficTracker::default();
        tracker.update_at(
            TrafficTotals {
                bytes_from_device: 100,
                bytes_to_device: 200,
                packets_from_device: 1,
                packets_to_device: 2,
            },
            start,
        );
        tracker.update_at(
            TrafficTotals {
                bytes_from_device: 2_100,
                bytes_to_device: 1_200,
                packets_from_device: 11,
                packets_to_device: 12,
            },
            start + std::time::Duration::from_secs(2),
        );
        let mut metrics = Metrics::default();
        tracker.apply(&mut metrics);
        assert_eq!(1_000, metrics.bytes_from_device_per_second);
        assert_eq!(500, metrics.bytes_to_device_per_second);
        assert_eq!(11, metrics.packets_from_device);

        tracker.update_at(
            TrafficTotals::default(),
            start + std::time::Duration::from_secs(3),
        );
        tracker.apply(&mut metrics);
        assert_eq!(0, metrics.bytes_from_device_per_second);
        assert_eq!(0, metrics.bytes_to_device_per_second);
    }
}
