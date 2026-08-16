use notify_rust::{Notification, Timeout, Urgency};

pub fn notify_heal(folder_id: &str, file_name: &str) {
    let _ = Notification::new()
        .summary("🩹 Backup Self-Healed")
        .body(&format!("Corrupted file healed in [{}]:\n{}", folder_id, file_name))
        .appname("Rust Backup Daemon")
        .timeout(Timeout::Milliseconds(5000))
        .show();
}

pub fn notify_error(folder_id: &str, file_name: &str, error_msg: &str) {
    let _ = Notification::new()
        .summary("🚨 Sync Error")
        .body(&format!("Failed to sync [{}] - {}\nError: {}", folder_id, file_name, error_msg))
        .appname("Rust Backup Daemon")
        .urgency(Urgency::Critical)
        .timeout(Timeout::Milliseconds(7000))
        .show();
}