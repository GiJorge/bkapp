use chrono::Utc;
use chrono_tz::Tz;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String, // "INFO", "HEAL", "ERROR"
    pub message: String,
}

#[derive(Clone)]
pub struct LogStore {
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
    max_size: usize,
    timezone: String,
}

impl LogStore {
    pub fn new(max_size: usize, timezone: String) -> Self {
        Self {
            logs: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
            timezone,
        }
    }

    pub fn add(&self, level: &str, message: impl Into<String>) {
        // Parse IANA timezone string or fallback to UTC
        let tz: Tz = self.timezone.parse().unwrap_or(chrono_tz::UTC);
        
        // 🕒 Updated format string to 12-hour AM/PM time (%l for non-padded hour, %p for AM/PM)
        let timestamp = Utc::now().with_timezone(&tz).format("%l:%M:%S %p").to_string();

        let entry = LogEntry {
            timestamp,
            level: level.to_string(),
            message: message.into(),
        };

        println!("[{}] [{}] {}", entry.timestamp, entry.level, entry.message);

        let mut lock = self.logs.lock().unwrap();
        if lock.len() >= self.max_size {
            lock.pop_front();
        }
        lock.push_back(entry);
    }

    pub fn get_all(&self) -> Vec<LogEntry> {
        let lock = self.logs.lock().unwrap();
        lock.iter().cloned().collect()
    }
}