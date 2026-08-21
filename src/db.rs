use rusqlite::{params, Connection, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct BackupStats {
    pub total_files: u64,
    pub total_bytes: u64,
    pub last_synced_timestamp: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct StoredRecord {
    pub relative_path: String,
    pub blake3_hash: String,
    pub file_size: u64,
}

#[derive(Serialize)]
pub struct FolderStat {
    pub folder_id: String,
    pub total_files: u64,
    pub total_bytes: u64,
}

#[derive(Serialize)]
pub struct FileRecord {
    pub folder_id: String,
    pub relative_path: String,
    pub file_size: u64,
    pub last_synced: i64,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn init<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_records (
                folder_id TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                blake3_hash TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                last_synced INTEGER NOT NULL,
                PRIMARY KEY (folder_id, relative_path)
            )",
            [],
        )?;

        Ok(Self { conn })
    }



    /// Retrieve all stored file records for a given folder_id
    pub fn get_folder_records(&self, folder_id: &str) -> Result<Vec<StoredRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path, blake3_hash, file_size FROM file_records WHERE folder_id = ?1",
        )?;

        let rows = stmt.query_map(params![folder_id], |row| {
            Ok(StoredRecord {
                relative_path: row.get(0)?,
                blake3_hash: row.get(1)?,
                file_size: row.get(2)?,
            })
        })?;

        let mut records = Vec::new();
        for r in rows {
            records.push(r?);
        }
        Ok(records)
    }

    pub fn needs_backup(&self, folder_id: &str, relative_path: &str, current_hash: &str, size: u64) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT blake3_hash, file_size FROM file_records WHERE folder_id = ?1 AND relative_path = ?2",
        )?;

        let mut rows = stmt.query(params![folder_id, relative_path])?;

        if let Some(row) = rows.next()? {
            let stored_hash: String = row.get(0)?;
            let stored_size: u64 = row.get(1)?;
            Ok(stored_hash != current_hash || stored_size != size)
        } else {
            Ok(true)
        }
    }

    pub fn mark_synced(&self, folder_id: &str, relative_path: &str, hash: &str, size: u64) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT INTO file_records (folder_id, relative_path, blake3_hash, file_size, last_synced)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(folder_id, relative_path) DO UPDATE SET
                blake3_hash = excluded.blake3_hash,
                file_size = excluded.file_size,
                last_synced = excluded.last_synced",
            params![folder_id, relative_path, hash, size as i64, now],
        )?;

        Ok(())
    }

    pub fn get_stats(&self) -> Result<BackupStats> {
        let total_files: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM file_records", [], |r| r.get(0))
            .unwrap_or(0);

        let total_bytes: u64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(file_size), 0) FROM file_records", [], |r| r.get(0))
            .unwrap_or(0);

        let last_synced_timestamp: Option<i64> = self
            .conn
            .query_row("SELECT MAX(last_synced) FROM file_records", [], |r| r.get(0))
            .ok();

        Ok(BackupStats {
            total_files,
            total_bytes,
            last_synced_timestamp,
        })
    }

    /// Fetch breakdown per folder ID
    pub fn get_folder_stats(&self) -> Result<Vec<FolderStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, COUNT(*), COALESCE(SUM(file_size), 0)
             FROM file_records
             GROUP BY folder_id",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FolderStat {
                folder_id: row.get(0)?,
                total_files: row.get(1)?,
                total_bytes: row.get(2)?,
            })
        })?;

        let mut stats = Vec::new();
        for s in rows {
            stats.push(s?);
        }
        Ok(stats)
    }

    /// Fetch recent files with folder_id column included
    pub fn get_recent_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, relative_path, file_size, last_synced 
             FROM file_records 
             ORDER BY last_synced DESC LIMIT 10",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FileRecord {
                folder_id: row.get(0)?,
                relative_path: row.get(1)?,
                file_size: row.get(2)?,
                last_synced: row.get(3)?,
            })
        })?;

        let mut files = Vec::new();
        for file in rows {
            files.push(file?);
        }
        Ok(files)
    }

    /// Remove database record when a file is pruned or deleted from backup
  pub fn remove_record(&self, folder_id: &str, relative_path: &str) -> Result<()> {
    self.conn.execute(
        "DELETE FROM file_records WHERE folder_id = ?1 AND relative_path = ?2",
        params![folder_id, relative_path],
    )?;
    Ok(())
}

/// Delete all file records associated with a discarded folder ID
    pub fn delete_folder_records(&self, folder_id: &str) -> Result<usize, rusqlite::Error> {
        let count = self.conn.execute(
            "DELETE FROM file_records WHERE folder_id = ?1",
            [folder_id],
        )?;
        Ok(count)
    }

}