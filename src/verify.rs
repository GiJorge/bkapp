use crate::config::CompiledFolderRule;
use crate::db::Db;
use crate::hash::compute_hash;
use crate::notification::notify_heal;
use std::sync::{Arc, Mutex};

pub async fn run_integrity_check(
    rule: &CompiledFolderRule,
    db: &Arc<Mutex<Db>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Skip verification if destination drive/share is turned off or unmounted
    if !rule.mapping.destination_dir.exists() {
        return Ok(());
    }
    let records = {
        let db_lock = db.lock().unwrap();
        db_lock.get_folder_records(&rule.mapping.id)?
    };

    if records.is_empty() {
        return Ok(());
    }

    println!("🔍 [{}] Running integrity & self-healing audit ({} files)...", rule.mapping.id, records.len());

    for record in records {
        let source_path = rule.mapping.source_dir.join(&record.relative_path);
        let dest_path = rule.mapping.destination_dir.join(&record.relative_path);

        // 1. If source no longer exists, skip (pruning handles orphans)
        if !source_path.exists() {
            continue;
        }

        let needs_healing = if !dest_path.exists() {
            println!("⚠️ [{}] Destination missing: {}", rule.mapping.id, record.relative_path);
            true
        } else {
            // Verify destination hash in background thread
            let dest_buf = dest_path.clone();
            let dest_hash = tokio::task::spawn_blocking(move || compute_hash(&dest_buf)).await?;

            match dest_hash {
                Ok(hash) => {
                    if hash != record.blake3_hash {
                        println!("⚡ [{}] Bit rot/Corruption detected in destination: {}", rule.mapping.id, record.relative_path);
                        true
                    } else {
                        false
                    }
                }
                Err(_) => true,
            }
        };

        // 2. Self-Healing Action: Re-sync from source if missing or corrupted
        if needs_healing {
            if let Some(parent) = dest_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            if tokio::fs::copy(&source_path, &dest_path).await.is_ok() {
                let source_buf = source_path.clone();
                if let Ok(Ok(new_hash)) = tokio::task::spawn_blocking(move || compute_hash(&source_buf)).await {
                    if let Ok(metadata) = tokio::fs::metadata(&source_path).await {
                        let db_lock = db.lock().unwrap();
                        let _ = db_lock.mark_synced(
                            &rule.mapping.id,
                            &record.relative_path,
                            &new_hash,
                            metadata.len(),
                        );
                    }
                }

                println!("🩹 [{}] Self-healed destination file: {}", rule.mapping.id, record.relative_path);
                // Trigger Desktop Notification
                notify_heal(&rule.mapping.id, &record.relative_path);
            }
        }
    }

    Ok(())
}