use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FolderMapping {
    pub id: String,
    pub source_dir: PathBuf,
    pub destination_dir: PathBuf,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_true")]
    pub delete_orphans: bool,
    #[serde(default)]
    pub retention_days: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub folders: Vec<FolderMapping>,
    pub db_path: PathBuf,
    pub debounce_seconds: u64,
    pub max_concurrent_copies: usize,
}

#[derive(Debug, Clone)]
pub struct CompiledFolderRule {
    pub mapping: FolderMapping,
    pub glob_set: GlobSet,
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
        exclude: vec!["*.tmp".to_string(), "*.log".to_string(), "target/*".to_string()],
        delete_orphans: true,
        retention_days: 0,
    }],
    db_path: PathBuf::from("backup_index.db"),
    debounce_seconds: 2,
    max_concurrent_copies: 4,
};

            let toml_str = toml::to_string_pretty(&default_config)?;
            fs::write(path, toml_str)?;

            println!("Generated default config at '{}'.", path.display());
            Ok(default_config)
        }
    }

    /// Compile string glob patterns into optimized GlobSets
    pub fn compile_rules(&self) -> Result<Vec<CompiledFolderRule>, globset::Error> {
        let mut rules = Vec::new();

        for folder in &self.folders {
            let mut builder = GlobSetBuilder::new();
            for pattern in &folder.exclude {
                // Support matching subdirectories or root-relative files
                let glob_str = if pattern.starts_with('*') || pattern.contains('/') {
                    pattern.clone()
                } else {
                    format!("**/{}", pattern)
                };
                builder.add(Glob::new(&glob_str)?);
            }
            let glob_set = builder.build()?;
            rules.push(CompiledFolderRule {
                mapping: folder.clone(),
                glob_set,
            });
        }

        Ok(rules)
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

            if !folder.destination_dir.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Destination directory for [{}] does not exist: {:?}", folder.id, folder.destination_dir),
                ));
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
    pub fn is_excluded<P: AsRef<Path>>(&self, relative_path: P) -> bool {
        self.glob_set.is_match(relative_path.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_exclusions() {
        let folder = FolderMapping {
            id: "test".to_string(),
            source_dir: PathBuf::from("/tmp/src"),
            destination_dir: PathBuf::from("/tmp/dst"),
            exclude: vec!["*.tmp".to_string(), "*.log".to_string(), "target/*".to_string(), "node_modules/*".to_string()],
        };

        let config = AppConfig {
            folders: vec![folder],
            db_path: PathBuf::from("test.db"),
            debounce_seconds: 2,
            max_concurrent_copies: 4,
        };

        let rules = config.compile_rules().expect("Failed to compile rules");
        let rule = &rules[0];

        // Should be excluded
        assert!(rule.is_excluded("cache.tmp"));
        assert!(rule.is_excluded("app.log"));
        assert!(rule.is_excluded("target/debug/app"));
        assert!(rule.is_excluded("node_modules/express/index.js"));

        // Should NOT be excluded
        assert!(!rule.is_excluded("index.rs"));
        assert!(!rule.is_excluded("data/memos.db"));
        assert!(!rule.is_excluded("images/photo.png"));
    }
}