use crate::config::CompiledFolderRule;
use crate::db::Db;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

pub async fn run_prune_pass(
    rule: &CompiledFolderRule,
    db: &Arc<Mutex<Db>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest_dir = &rule.mapping.destination_dir;
    let source_dir = &rule.mapping.source_dir;

    
    if !dest_dir.exists() {
        return Ok(());
    }

    // 🛡️ UNMOUNT GUARD: Skip prune pass if destination is unmounted
    let mount_flag = dest_dir.join(".mounted");
    if !mount_flag.exists() {
        return Ok(());
    }

    let now = SystemTime::now();
    let max_age = if rule.mapping.retention_days > 0 {
        Some(Duration::from_secs(rule.mapping.retention_days * 86400))
    } else {
        None
    };

    for entry in WalkDir::new(dest_dir).into_iter().filter_map(|e| e.ok()) {
        let dest_path = entry.path();
        if !dest_path.is_file() {
            continue;
        }

        let relative_path = match dest_path.strip_prefix(dest_dir) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        // 🛡️ EXCLUSION & HARDGUARD: Skip if path is in config `exclude` list or is `.mounted`
        if relative_path.file_name().map_or(false, |name| name == ".mounted")
            || rule.is_excluded(relative_path)
        {
            continue;
        }

        let source_path = source_dir.join(relative_path);
        let relative_str = relative_path.to_string_lossy().to_string();

        // 1. Orphan Detection: Delete destination file if removed from source
        if rule.mapping.delete_orphans && !source_path.exists() {
            if let Ok(_) = tokio::fs::remove_file(dest_path).await {
                let db_lock = db.lock().unwrap();
                let _ = db_lock.remove_record(&rule.mapping.id, &relative_str);
                println!("🗑️ [{}] Pruned orphan: {}", rule.mapping.id, relative_str);
            }
            continue;
        }

        // 2. Retention Expiration: Delete destination file if older than allowed retention period
        if let Some(max_duration) = max_age {
            if let Ok(metadata) = fs::metadata(dest_path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > max_duration {
                            if let Ok(_) = tokio::fs::remove_file(dest_path).await {
                                let db_lock = db.lock().unwrap();
                                let _ = db_lock.remove_record(&rule.mapping.id, &relative_str);
                                println!(
                                    "⏳ [{}] Pruned expired file (>{} days): {}",
                                    rule.mapping.id, rule.mapping.retention_days, relative_str
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}