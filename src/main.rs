mod config;
mod db;
mod hash;
mod logger;
mod notification;
mod prune;
mod verify;
mod web;

use config::{AppConfig, CompiledFolderRule};
use db::Db;
use hash::compute_hash;
use logger::LogStore;
use notification::notify_error;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct SyncJob {
    pub path: PathBuf,
    pub rule: CompiledFolderRule,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load_or_create("config.toml")?;

    // 1. Initialize LogStore first
    let logger = LogStore::new(100, config.timezone.clone());
    logger.add("INFO", "Backup daemon initialized");

    // 2. Compile rules passing logger
    let compiled_rules = config.compile_rules(&logger);

    if compiled_rules.is_empty() {
        logger.add("ERROR", "No valid folder mappings loaded! Check config.toml.");
    } else {
        logger.add("INFO", format!("Loaded {} valid folder rule(s).", compiled_rules.len()));
    }

    let rules = Arc::new(compiled_rules);
    let db = Arc::new(Mutex::new(Db::init(&config.db_path)?));

    // Start Web Server
    let web_db = Arc::clone(&db);
    let web_logger = logger.clone();
    tokio::spawn(async move {
        web::start_server(web_db, web_logger, 3000).await;
    });

    // Work Queue Channel & Worker Pool Initialization
    let (job_tx, job_rx) = mpsc::channel::<SyncJob>(2000);
    let shared_rx = Arc::new(AsyncMutex::new(job_rx));

    for _ in 0..config.max_concurrent_copies {
        let rx = Arc::clone(&shared_rx);
        let db = Arc::clone(&db);
        let logger_clone = logger.clone();

        tokio::spawn(async move {
            loop {
                let job = {
                    let mut lock = rx.lock().await;
                    lock.recv().await
                };

                match job {
                    Some(job) => {
                        let _ = sync_single_file(&job.path, &job.rule, &db, &logger_clone).await;
                    }
                    None => break, // Channel closed on shutdown
                }
            }
        });
    }

    // Run Initial Crawl for all rules (Swallows errors, won't panic)
    let mut crawl_handles = vec![];
    for rule in rules.iter().cloned() {
        let tx = job_tx.clone();
        let logger_clone = logger.clone();

        crawl_handles.push(tokio::spawn(async move {
            run_initial_crawl(&rule, &tx, &logger_clone).await;
        }));
    }
    for handle in crawl_handles {
        let _ = handle.await;
    }

    // Scheduled Pruning Background Task
    let prune_rules = Arc::clone(&rules);
    let prune_db = Arc::clone(&db);
    tokio::spawn(async move {
        loop {
            for rule in prune_rules.iter() {
                let _ = prune::run_prune_pass(rule, &prune_db).await;
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });

    // Scheduled Verification & Self-Healing Audit Task
    let verify_rules = Arc::clone(&rules);
    let verify_db = Arc::clone(&db);
    tokio::spawn(async move {
        loop {
            for rule in verify_rules.iter() {
                let _ = verify::run_integrity_check(rule, &verify_db).await;
            }
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });

    // Scheduled Interval Backup Tasks (for folders with sync_mode = "interval")
    for rule in rules.iter().filter(|r| r.mapping.sync_mode == "interval") {
        let rule = rule.clone();
        let tx = job_tx.clone();
        let interval_secs = rule.mapping.interval_seconds;
        let logger_clone = logger.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
            ticker.tick().await;

            loop {
                ticker.tick().await;
                logger_clone.add("INFO", format!("⏰ Running scheduled scan for [{}]", rule.mapping.id));
                run_initial_crawl(&rule, &tx, &logger_clone).await;
            }
        });
    }

    // File Watcher Instance Setup
    let (watcher_tx, mut watcher_rx) = mpsc::channel::<notify::Result<Event>>(100);
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = watcher_tx.blocking_send(res);
        },
        Config::default(),
    )?;

    for rule in rules.iter() {
        if rule.mapping.sync_mode != "interval" && rule.mapping.source_dir.exists() {
            if let Err(e) = watcher.watch(&rule.mapping.source_dir, RecursiveMode::Recursive) {
                logger.add("ERROR", format!("Failed to watch source for [{}]: {}", rule.mapping.id, e));
            } else {
                logger.add("INFO", format!("👀 Watching real-time [{}] at {:?}", rule.mapping.id, rule.mapping.source_dir));
            }
        } else if rule.mapping.sync_mode == "interval" {
            logger.add("INFO", format!("⏰ Interval sync configured for [{}] (every {}s)", rule.mapping.id, rule.mapping.interval_seconds));
        } else {
            logger.add("INFO", format!("⏸️ Source folder missing/unmounted for [{}]. Watcher skipped.", rule.mapping.id));
        }
    }

    println!("🚀 Backup daemon operational. Press Ctrl+C or send SIGTERM to stop cleanly.");

    // Event Loop with Graceful Shutdown Signal Handling
    loop {
        tokio::select! {
            Some(res) = watcher_rx.recv() => {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                        tokio::time::sleep(Duration::from_secs(config.debounce_seconds)).await;

                        for path in event.paths {
                            let Some(rule) = rules.iter().find(|r| path.is_file() && path.starts_with(&r.mapping.source_dir)) else {
                                continue;
                            };

                            if rule.mapping.sync_mode == "interval" {
                                continue;
                            }

                            if let Ok(rel_path) = path.strip_prefix(&rule.mapping.source_dir) {
                                if !rule.is_allowed(rel_path) {
                                    continue;
                                }

                                let _ = job_tx
                                    .send(SyncJob {
                                        path: path.clone(),
                                        rule: rule.clone(),
                                    })
                                    .await;
                            }
                        }
                    }
                }
            }
            _ = shutdown_signal() => {
                println!("\n🛑 Shutdown signal received. Exiting daemon safely...");
                break;
            }
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn run_initial_crawl(
    rule: &CompiledFolderRule,
    job_tx: &mpsc::Sender<SyncJob>,
    logger: &LogStore,
) {
    let source = rule.mapping.source_dir.clone();
    let dest = rule.mapping.destination_dir.clone();
    let rule_id = rule.mapping.id.clone();

    // 🛡️ NON-BLOCKING PATH & MOUNT CHECK
    // Runs inside Tokio's blocking thread pool so network timeouts don't freeze the app
    let (source_exists, dest_exists, is_mounted) = tokio::task::spawn_blocking(move || {
        let s_exists = source.exists();
        let d_exists = dest.exists();

        let mut mounted = false;
        if d_exists {
            let mount_flag = dest.join(".mounted");
            // Auto-create .mounted file if destination exists but flag is missing
            if !mount_flag.exists() {
                let _ = std::fs::File::create(&mount_flag);
            }
            mounted = mount_flag.exists();
        }

        (s_exists, d_exists, mounted)
    })
    .await
    .unwrap_or((false, false, false));

    if !source_exists {
        logger.add(
            "INFO",
            format!("⏸️ Source directory for [{}] missing or unmounted.", rule_id),
        );
        return;
    }

    if !dest_exists {
        logger.add(
            "ERROR",
            format!("⚠️ Destination folder missing or timed out for [{}]: {:?}", rule_id, rule.mapping.destination_dir),
        );
        return;
    }

    if !is_mounted {
        logger.add(
            "INFO",
            format!("⏸️ Drive unmounted (.mounted missing) for [{}]. Skipping crawl.", rule_id),
        );
        return;
    }

    let source_dir = rule.mapping.source_dir.clone();
    let rule_clone = rule.clone();
    let tx_clone = job_tx.clone();
    let logger_clone = logger.clone();

    tokio::task::spawn_blocking(move || {
        let mut it = WalkDir::new(&source_dir).into_iter();

        loop {
            let entry = match it.next() {
                Some(Ok(e)) => e,
                Some(Err(err)) => {
                    logger_clone.add(
                        "ERROR",
                        format!("Skipped unreadable path in [{}]: {}", rule_id, err),
                    );
                    continue;
                }
                None => break,
            };

            let path = entry.path();

            if entry.file_type().is_dir() && path.starts_with(&rule_clone.mapping.destination_dir) {
                it.skip_current_dir();
                continue;
            }

            if let Ok(rel_path) = path.strip_prefix(&source_dir) {
                if rel_path.as_os_str().is_empty() {
                    continue;
                }

                if rule_clone.is_excluded(rel_path) && entry.file_type().is_dir() {
                    it.skip_current_dir();
                    continue;
                }

                if path.is_file() && rule_clone.is_allowed(rel_path) {
                    let _ = tx_clone.blocking_send(SyncJob {
                        path: path.to_path_buf(),
                        rule: rule_clone.clone(),
                    });
                }
            }
        }
    })
    .await
    .ok();
}

async fn sync_single_file(
    path: &Path,
    rule: &CompiledFolderRule,
    db: &Arc<Mutex<Db>>,
    logger: &LogStore,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.starts_with(&rule.mapping.destination_dir) {
        return Ok(());
    }

    let relative_path = match path.strip_prefix(&rule.mapping.source_dir) {
        Ok(rel) => rel,
        Err(_) => return Ok(()),
    };

    if !rule.is_allowed(relative_path) {
        return Ok(());
    }

    let dest = rule.mapping.destination_dir.clone();
    let rule_id = rule.mapping.id.clone();

    let (dest_exists, is_mounted) = tokio::task::spawn_blocking(move || {
        let d_exists = dest.exists();
        let mounted = d_exists && dest.join(".mounted").exists();
        (d_exists, mounted)
    })
    .await?;

    if !dest_exists {
        logger.add(
            "ERROR",
            format!("⚠️ Destination path unavailable or timed out for [{}]", rule_id),
        );
        return Ok(());
    }

    if !is_mounted {
        return Ok(());
    }

    let relative_str = relative_path.to_string_lossy().to_string();

    let metadata = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) => {
            notify_error(&rule.mapping.id, &relative_str, &e.to_string());
            return Err(e.into());
        }
    };
    let file_size = metadata.len();

    let path_buf = path.to_path_buf();
    let current_hash = match tokio::task::spawn_blocking(move || compute_hash(&path_buf)).await? {
        Ok(h) => h,
        Err(e) => {
            notify_error(&rule.mapping.id, &relative_str, &e.to_string());
            return Err(e.into());
        }
    };

    let needs_sync = {
        let db_lock = db.lock().unwrap();
        db_lock.needs_backup(&rule.mapping.id, &relative_str, &current_hash, file_size)?
    };

    if needs_sync {
        let dest_path = rule.mapping.destination_dir.join(relative_path);

        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if let Err(e) = tokio::fs::copy(path, &dest_path).await {
            notify_error(&rule.mapping.id, &relative_str, &e.to_string());
            return Err(e.into());
        }

        {
            let db_lock = db.lock().unwrap();
            db_lock.mark_synced(&rule.mapping.id, &relative_str, &current_hash, file_size)?;
        }

        println!("⚡ [{}] Synced: {}", rule.mapping.id, relative_str);
    }

    Ok(())
}