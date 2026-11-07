use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const DEFAULT_CAPACITY: usize = 50_000;
const DEFAULT_FILE_LIMIT: u64 = 5 * 1024 * 1024;
const DEFAULT_FILE_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEvent {
    pub sequence: u64,
    pub timestamp: String,
    pub level: LogLevel,
    pub component: String,
    pub device_id: Option<String>,
    pub event_type: String,
    pub message: String,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

impl LogEvent {
    pub fn new(
        level: LogLevel,
        component: impl Into<String>,
        device_id: Option<String>,
        event_type: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            sequence: 0,
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            level,
            component: component.into(),
            device_id,
            event_type: event_type.into(),
            message: message.into(),
            fields: Map::new(),
        }
    }

    pub fn field(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_string(), value.into());
        self
    }
}

pub type SharedLog = Arc<Mutex<LogStore>>;

pub struct LogStore {
    events: VecDeque<LogEvent>,
    capacity: usize,
    next_sequence: u64,
    directory: PathBuf,
    file_limit: u64,
    file_count: usize,
}

impl LogStore {
    pub fn open(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        Ok(Self {
            events: VecDeque::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
            next_sequence: 1,
            directory,
            file_limit: DEFAULT_FILE_LIMIT,
            file_count: DEFAULT_FILE_COUNT,
        })
    }

    #[cfg(test)]
    fn with_limits(directory: PathBuf, capacity: usize, file_limit: u64) -> io::Result<Self> {
        let mut result = Self::open(directory)?;
        result.capacity = capacity;
        result.file_limit = file_limit;
        result.file_count = 2;
        Ok(result)
    }

    pub fn push(&mut self, mut event: LogEvent) -> io::Result<u64> {
        event.sequence = self.next_sequence;
        self.next_sequence += 1;
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        let sequence = event.sequence;
        self.events.push_back(event.clone());
        self.write_disk(&event).map(|_| sequence)
    }

    pub fn recent(
        &self,
        limit: usize,
        minimum: Option<LogLevel>,
        component: Option<&str>,
        device_id: Option<&str>,
        after_sequence: Option<u64>,
        before_sequence: Option<u64>,
    ) -> Vec<LogEvent> {
        let mut events: Vec<_> = self
            .events
            .iter()
            .rev()
            .filter(|event| minimum.map_or(true, |level| event.level >= level))
            .filter(|event| component.map_or(true, |value| event.component == value))
            .filter(|event| {
                device_id.map_or(true, |value| event.device_id.as_deref() == Some(value))
            })
            .filter(|event| after_sequence.map_or(true, |after| event.sequence > after))
            .filter(|event| before_sequence.map_or(true, |before| event.sequence < before))
            .take(limit.min(self.capacity))
            .cloned()
            .collect();
        events.reverse();
        events
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn write_disk(&self, event: &LogEvent) -> io::Result<()> {
        let path = self.directory.join("windows.log");
        if fs::metadata(&path).map_or(false, |metadata| metadata.len() >= self.file_limit) {
            self.rotate()?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        file.flush()
    }

    fn rotate(&self) -> io::Result<()> {
        let oldest = self
            .directory
            .join(format!("windows.log.{}", self.file_count));
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for index in (1..self.file_count).rev() {
            let source = self.directory.join(format!("windows.log.{index}"));
            if source.exists() {
                fs::rename(
                    source,
                    self.directory.join(format!("windows.log.{}", index + 1)),
                )?;
            }
        }
        let current = self.directory.join("windows.log");
        if current.exists() {
            fs::rename(&current, self.directory.join("windows.log.1"))?;
        }
        File::create(current)?;
        Ok(())
    }
}

pub fn emit(log: &SharedLog, event: LogEvent) {
    if let Ok(mut store) = log.lock() {
        if let Err(error) = store.push(event) {
            eprintln!("structured log write failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "gnirehtet-ng-log-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn ring_buffer_is_bounded_and_filters() {
        let directory = temp_dir("ring");
        let mut store = LogStore::with_limits(directory.clone(), 2, 1024 * 1024).unwrap();
        store
            .push(LogEvent::new(LogLevel::Info, "ADB", None, "one", "one"))
            .unwrap();
        store
            .push(LogEvent::new(LogLevel::Warn, "VPN", None, "two", "two"))
            .unwrap();
        store
            .push(LogEvent::new(
                LogLevel::Error,
                "ADB",
                None,
                "three",
                "three",
            ))
            .unwrap();
        let events = store.recent(10, Some(LogLevel::Warn), Some("ADB"), None, None, None);
        assert_eq!(1, events.len());
        assert_eq!("three", events[0].event_type);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn after_sequence_returns_only_newer_events() {
        let directory = temp_dir("after");
        let mut store = LogStore::with_limits(directory.clone(), 10, 1024 * 1024).unwrap();
        for event_type in ["one", "two", "three", "four"] {
            store
                .push(LogEvent::new(
                    LogLevel::Info,
                    "CORE",
                    None,
                    event_type,
                    event_type,
                ))
                .unwrap();
        }
        let before = store.recent(10, None, None, None, None, None);
        assert_eq!(4, before.len());
        let cutoff = before[1].sequence;

        let after = store.recent(10, None, None, None, Some(cutoff), None);
        assert_eq!(2, after.len());
        assert!(after.iter().all(|event| event.sequence > cutoff));
        assert_eq!("three", after[0].event_type);

        let empty = store.recent(10, None, None, None, Some(before[3].sequence), None);
        assert!(empty.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn before_sequence_returns_closest_older_events() {
        let directory = temp_dir("before");
        let mut store = LogStore::with_limits(directory.clone(), 10, 1024 * 1024).unwrap();
        for event_type in ["one", "two", "three", "four"] {
            store
                .push(LogEvent::new(
                    LogLevel::Info,
                    "CORE",
                    None,
                    event_type,
                    event_type,
                ))
                .unwrap();
        }
        let all = store.recent(10, None, None, None, None, None);
        let cutoff = all[3].sequence;

        let older = store.recent(10, None, None, None, None, Some(cutoff));
        assert_eq!(3, older.len());
        assert!(older.iter().all(|event| event.sequence < cutoff));
        assert_eq!("one", older[0].event_type);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn disk_log_rotates() {
        let directory = temp_dir("rotate");
        let mut store = LogStore::with_limits(directory.clone(), 10, 1).unwrap();
        store
            .push(LogEvent::new(LogLevel::Info, "CORE", None, "one", "one"))
            .unwrap();
        store
            .push(LogEvent::new(LogLevel::Info, "CORE", None, "two", "two"))
            .unwrap();
        assert!(directory.join("windows.log.1").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
