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
}

impl LogStore {
    pub fn new(max_size: usize) -> Self {
        Self {
            logs: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
        }
    }

    pub fn add(&self, level: &str, message: impl Into<String>) {
        let entry = LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
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