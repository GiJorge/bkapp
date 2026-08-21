use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use crate::logger::LogStore;

fn default_sync_mode() -> String {
    "watch".to_string()
}

fn default_interval() -> u64 {
    3600
}

fn default_true() -> bool {
    true
}

fn default_timezone() -> String {
    "UTC".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FolderMapping {
    pub id: String,
    pub source_dir: PathBuf,
    pub destination_dir: PathBuf,
    #[serde(default = "default_sync_mode")]
    pub sync_mode: String, // "watch" or "interval"
    #[serde(default = "default_interval")]
    pub interval_seconds: u64, // Default 3600 (1 hr)
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default = "default_true")]
    pub delete_orphans: bool,
    #[serde(default)]
    pub retention_days: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub folders: Vec<FolderMapping>,
    pub db_path: PathBuf,
    pub debounce_seconds: u64,
    pub max_concurrent_copies: usize,
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

#[derive(Debug, Clone)]
pub struct CompiledFolderRule {
    pub mapping: FolderMapping,
    pub exclude_glob_set: GlobSet,
    pub include_glob_set: Option<GlobSet>,
}

impl AppConfig {
    pub fn load_or_create<P: AsRef<Path>>(config_path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = config_path.as_ref();

        if path.exists() {
            let content = fs::read_to_string(path)?;
            let mut config: AppConfig = toml::from_str(&content)?;
            config.clean_and_expand_paths();
            config.validate_paths()?;
            Ok(config)
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/data/data/com.termux/files/home".to_string());
            let default_config = AppConfig {
                folders: vec![FolderMapping {
                    id: "memos".to_string(),
                    source_dir: PathBuf::from(format!("{}/memos-data", home)),
                    destination_dir: PathBuf::from(format!("{}/backups/memos", home)),
                    sync_mode: "watch".to_string(),
                    interval_seconds: 3600,
                    exclude: vec![".mounted".to_string(), "*.tmp".to_string(), "*.log".to_string(), "target/*".to_string()],
                    include: vec![],
                    delete_orphans: true,
                    retention_days: 0,
                }],
                db_path: PathBuf::from("backup_index.db"),
                debounce_seconds: 2,
                max_concurrent_copies: 4,
                timezone: "UTC".to_string(),
            };

            let toml_str = toml::to_string_pretty(&default_config)?;
            fs::write(path, toml_str)?;

            println!("Generated default config at '{}'.", path.display());
            Ok(default_config)
        }
    }

    /// Compile string glob patterns into optimized GlobSets
 pub fn compile_rules(&self, logger: &LogStore) -> Vec<CompiledFolderRule> {
        let mut rules = Vec::new();

        for folder in &self.folders {
            // 🛡️ RECURSIVE LOOP GUARD: Log warning to Web UI and skip dangerous mappings
            if let Err(err_msg) = validate_path_boundaries(&folder.source_dir, &folder.destination_dir) {
                eprintln!("⚠️ [CONFIG WARNING] Folder '{}' skipped: {}", folder.id, err_msg);
                logger.add("ERROR", format!("Skipped folder ['{}']: {}", folder.id, err_msg));
                continue;
            }

            // Build Exclude GlobSet
            let mut exclude_builder = GlobSetBuilder::new();
            for pattern in &folder.exclude {
                let glob_str = if pattern.starts_with('*') || pattern.contains('/') {
                    pattern.clone()
                } else {
                    format!("**/{}", pattern)
                };
                if let Ok(glob) = Glob::new(&glob_str) {
                    exclude_builder.add(glob);
                }
            }
            let exclude_glob_set = exclude_builder.build().unwrap_or_default();

            // Build Include GlobSet
            let include_glob_set = if !folder.include.is_empty() {
                let mut include_builder = GlobSetBuilder::new();
                for pattern in &folder.include {
                    let glob_str = if pattern.starts_with('*') || pattern.contains('/') {
                        pattern.clone()
                    } else {
                        format!("**/{}", pattern)
                    };
                    if let Ok(glob) = Glob::new(&glob_str) {
                        include_builder.add(glob);
                    }
                }
                include_builder.build().ok()
            } else {
                None
            };

            rules.push(CompiledFolderRule {
                mapping: folder.clone(),
                exclude_glob_set,
                include_glob_set,
            });
        }

        rules
    }




    fn clean_and_expand_paths(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/data/data/com.termux/files/home".to_string());
        for folder in &mut self.folders {
            folder.source_dir = clean_path(&folder.source_dir, &home);
            folder.destination_dir = clean_path(&folder.destination_dir, &home);
        }
        self.db_path = clean_path(&self.db_path, &home);
    }

    fn validate_paths(&self) -> Result<(), io::Error> {
        for folder in &self.folders {
            if !folder.source_dir.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Source directory for [{}] does not exist: {:?}", folder.id, folder.source_dir),
                ));
            }

            // Check if destination exists
            if folder.destination_dir.exists() {
                // 📌 Auto-create .mounted file if destination directory is accessible
                let mount_flag = folder.destination_dir.join(".mounted");
                if !mount_flag.exists() {
                    let _ = fs::File::create(&mount_flag);
                }
            } else {
                println!(
                    "⚠️ Destination directory for [{}] is currently unavailable or unmounted: {:?}",
                    folder.id, folder.destination_dir
                );
            }
        }
        Ok(())
    }
}





fn clean_path(path: &Path, home: &str) -> PathBuf {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim().trim_matches(|c| c == '\r' || c == '"' || c == '\'');

    if trimmed.starts_with("~/") {
        PathBuf::from(format!("{}/{}", home, &trimmed[2..]))
    } else if trimmed == "~" {
        PathBuf::from(home)
    } else {
        PathBuf::from(trimmed)
    }
}

impl CompiledFolderRule {
    /// Helper to maintain backward compatibility with old `is_excluded` call sites
    pub fn is_excluded<P: AsRef<Path>>(&self, relative_path: P) -> bool {
        self.exclude_glob_set.is_match(relative_path.as_ref())
    }

    /// Evaluates both exclude rules and include whitelists
    pub fn is_allowed<P: AsRef<Path>>(&self, relative_path: P) -> bool {
        let path = relative_path.as_ref();

        // 1. If exclude rule matches, reject
        if self.exclude_glob_set.is_match(path) {
            return false;
        }

        // 2. If include set exists, reject files that do NOT match the whitelist
        if let Some(ref include_set) = self.include_glob_set {
            return include_set.is_match(path);
        }

        true
    }
}


/// Helper function to check overlapping or nested paths
fn validate_path_boundaries(source: &Path, destination: &Path) -> Result<(), String> {
    let source_canon = source.canonicalize().unwrap_or_else(|_| source.to_path_buf());
    let dest_canon = destination.canonicalize().unwrap_or_else(|_| destination.to_path_buf());

    if source_canon == dest_canon {
        return Err(format!(
            "Source and Destination cannot be identical! ({:?})",
            source_canon
        ));
    }

    if dest_canon.starts_with(&source_canon) {
        return Err(format!(
            "Destination ({:?}) is inside Source ({:?}). Loop prevented!",
            dest_canon, source_canon
        ));
    }

    if source_canon.starts_with(&dest_canon) {
        return Err(format!(
            "Source ({:?}) is inside Destination ({:?}). Loop prevented!",
            source_canon, dest_canon
        ));
    }

    Ok(())
}