/*
 * Gnirehtet NG additive relay observability.
 *
 * Licensed under the Apache License, Version 2.0.
 */

use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) const METRICS_PREFIX: &str = "GNIREHTET_NG_METRICS ";
const METRICS_ENVIRONMENT_VARIABLE: &str = "GNIREHTET_NG_EMIT_TRAFFIC_METRICS";

static BYTES_FROM_DEVICE: AtomicU64 = AtomicU64::new(0);
static BYTES_TO_DEVICE: AtomicU64 = AtomicU64::new(0);
static PACKETS_FROM_DEVICE: AtomicU64 = AtomicU64::new(0);
static PACKETS_TO_DEVICE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct TrafficTotals {
    pub bytes_from_device: u64,
    pub bytes_to_device: u64,
    pub packets_from_device: u64,
    pub packets_to_device: u64,
}

pub(crate) fn record_from_device(bytes: u64) {
    BYTES_FROM_DEVICE.fetch_add(bytes, Ordering::Relaxed);
    PACKETS_FROM_DEVICE.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_to_device(bytes: u64) {
    BYTES_TO_DEVICE.fetch_add(bytes, Ordering::Relaxed);
    PACKETS_TO_DEVICE.fetch_add(1, Ordering::Relaxed);
}

fn totals() -> TrafficTotals {
    TrafficTotals {
        bytes_from_device: BYTES_FROM_DEVICE.load(Ordering::Relaxed),
        bytes_to_device: BYTES_TO_DEVICE.load(Ordering::Relaxed),
        packets_from_device: PACKETS_FROM_DEVICE.load(Ordering::Relaxed),
        packets_to_device: PACKETS_TO_DEVICE.load(Ordering::Relaxed),
    }
}

pub(crate) fn start_reporter_if_requested() {
    if std::env::var_os(METRICS_ENVIRONMENT_VARIABLE).is_none() {
        return;
    }
    if let Err(error) = thread::Builder::new()
        .name("gnirehtet-traffic-metrics".to_string())
        .spawn(|| loop {
            thread::sleep(Duration::from_secs(1));
            if let Ok(encoded) = serde_json::to_string(&totals()) {
                let mut stdout = io::stdout().lock();
                let _ = writeln!(stdout, "{METRICS_PREFIX}{encoded}");
                let _ = stdout.flush();
            }
        })
    {
        log::warn!(target: "TrafficMetrics", "Could not start traffic reporter: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_report_has_stable_json_field_names() {
        let encoded = serde_json::to_string(&TrafficTotals {
            bytes_from_device: 10,
            bytes_to_device: 20,
            packets_from_device: 1,
            packets_to_device: 2,
        })
        .unwrap();
        assert!(encoded.contains("\"bytes_from_device\":10"));
        assert!(encoded.contains("\"packets_to_device\":2"));
    }

    #[test]
    fn packet_recording_updates_monotonic_totals() {
        let before = totals();
        record_from_device(40);
        record_to_device(60);
        let after = totals();
        assert_eq!(40, after.bytes_from_device - before.bytes_from_device);
        assert_eq!(60, after.bytes_to_device - before.bytes_to_device);
        assert_eq!(1, after.packets_from_device - before.packets_from_device);
        assert_eq!(1, after.packets_to_device - before.packets_to_device);
    }
}
