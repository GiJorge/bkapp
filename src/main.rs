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
//use tokio::signal::unix::{signal, SignalKind};
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
    

    let rules = Arc::new(config.compile_rules()?);
    let db = Arc::new(Mutex::new(Db::init(&config.db_path)?));

    // Global Log Store (keeps last 100 log entries)
    // let logger = LogStore::new(100);
    // logger.add("INFO", "Backup daemon initialized");

    // Pass config.timezone into LogStore
let logger = LogStore::new(100, config.timezone.clone());
logger.add("INFO", "Backup daemon initialized");

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

        tokio::spawn(async move {
            loop {
                let job = {
                    let mut lock = rx.lock().await;
                    lock.recv().await
                };

                match job {
                    Some(job) => {
                        let _ = sync_single_file(&job.path, &job.rule, &db).await;
                    }
                    None => break, // Channel closed on shutdown
                }
            }
        });
    }

    // Run Initial Crawl for all rules
    let mut crawl_handles = vec![];
    for rule in rules.iter().cloned() {
        let tx = job_tx.clone();

        crawl_handles.push(tokio::spawn(async move {
            let _ = run_initial_crawl(&rule, &tx).await;
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
            // Skip the immediate initial tick since initial crawl ran on startup
            ticker.tick().await;

            loop {
                ticker.tick().await;
                logger_clone.add("INFO", format!("⏰ Running scheduled scan for [{}]", rule.mapping.id));
                let _ = run_initial_crawl(&rule, &tx).await;
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

    // Only add real-time ("watch") folders to the file watcher
    for rule in rules.iter() {
        if rule.mapping.sync_mode != "interval" && rule.mapping.source_dir.exists() {
            watcher.watch(&rule.mapping.source_dir, RecursiveMode::Recursive)?;
            logger.add("INFO", format!("👀 Watching real-time [{}] at {:?}", rule.mapping.id, rule.mapping.source_dir));
        } else if rule.mapping.sync_mode == "interval" {
            logger.add("INFO", format!("⏰ Interval sync configured for [{}] (every {}s)", rule.mapping.id, rule.mapping.interval_seconds));
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
                            if path.is_file() {
                                if let Some(rule) = rules.iter().find(|r| path.starts_with(&r.mapping.source_dir)) {
                                    // Ignore events if folder is configured for interval sync mode
                                    if rule.mapping.sync_mode == "interval" {
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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut it = WalkDir::new(&rule.mapping.source_dir).into_iter();

    loop {
        let entry = match it.next() {
            Some(Ok(e)) => e,
            Some(Err(_)) => continue,
            None => break,
        };

        let path = entry.path();
        if let Ok(rel_path) = path.strip_prefix(&rule.mapping.source_dir) {
            if rel_path.as_os_str().is_empty() {
                continue;
            }

            if rule.is_excluded(rel_path) {
                if entry.file_type().is_dir() {
                    it.skip_current_dir();
                }
                continue;
            }

            if path.is_file() {
                let _ = job_tx
                    .send(SyncJob {
                        path: path.to_path_buf(),
                        rule: rule.clone(),
                    })
                    .await;
            }
        }
    }
    Ok(())
}

async fn sync_single_file(
    path: &Path,
    rule: &CompiledFolderRule,
    db: &Arc<Mutex<Db>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let relative_path = match path.strip_prefix(&rule.mapping.source_dir) {
        Ok(rel) => rel,
        Err(_) => return Ok(()),
    };

    if rule.is_excluded(relative_path) {
        return Ok(());
    }

    // 🛡️ UNMOUNT GUARD: Do not attempt sync if destination drive is unmounted
    // let mount_flag = rule.mapping.destination_dir.join(".mounted");
    // if !mount_flag.exists() {
    //     return Ok(());
    // }

    // 🛡️ UNMOUNT / DISCONNECT GUARD
    if !rule.mapping.destination_dir.exists() || !rule.mapping.destination_dir.join(".mounted").exists() {
        // Destination is disconnected or unmounted; skip quietly without crashing
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