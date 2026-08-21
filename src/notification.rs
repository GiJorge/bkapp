use notify_rust::{Notification, Timeout, Urgency};
use std::process::Command;

pub fn notify_heal(folder_id: &str, file_name: &str) {
    let summary = "🩹 Backup Self-Healed";
    let body = format!("Corrupted file healed in [{}]:\n{}", folder_id, file_name);

    // 1. Try Linux Desktop Notification
    let desktop_res = Notification::new()
        .summary(summary)
        .body(&body)
        .appname("Rust Backup Daemon")
        .timeout(Timeout::Milliseconds(5000))
        .show();

    // 2. Fallback to Termux Notification
    if let Err(e) = desktop_res {
        eprintln!("Desktop notification failed: {}. Trying termux-notification...", e);
        
        let termux_res = Command::new("termux-notification")
            .arg("--title")
            .arg(summary)
            .arg("--content")
            .arg(format!("[{}] {}", folder_id, file_name))
            .status();

        if let Err(err) = termux_res {
            eprintln!("Failed to execute termux-notification: {}", err);
        }
    }
}

pub fn notify_error(folder_id: &str, file_name: &str, error_msg: &str) {
    let summary = "🚨 Sync Error";
    let body = format!("Failed to sync [{}] - {}\nError: {}", folder_id, file_name, error_msg);

    // 1. Try Linux Desktop Notification
    let desktop_res = Notification::new()
        .summary(summary)
        .body(&body)
        .appname("Rust Backup Daemon")
        .urgency(Urgency::Critical)
        .timeout(Timeout::Milliseconds(7000))
        .show();

    // 2. Fallback to Termux Notification
    if let Err(e) = desktop_res {
        eprintln!("Desktop notification failed: {}. Trying termux-notification...", e);

        let termux_res = Command::new("termux-notification")
            .arg("--title")
            .arg(summary)
            .arg("--content")
            .arg(format!("[{}] {}\n{}", folder_id, file_name, error_msg))
            .status();

        if let Err(err) = termux_res {
            eprintln!("Failed to execute termux-notification: {}", err);
        }
    }
}