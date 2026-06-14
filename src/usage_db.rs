//! SQLite-backed persistence for usage tracking.
//!
//! Two tables:
//! - `usage_records` — append-only log of every successful upstream call.
//! - `quota_usage`   — cumulative tokens per client_api_key id.
//!
//! Concurrency model: single-process writer (the gateway runs as one process
//! per host). WAL mode is enabled so reads don't block the writer.
//!
//! All write errors are surfaced to the caller; the caller (UsageTracker /
//! ApiKeyQuotaTracker) logs and continues — a DB hiccup should not fail a
//! chat request.

use crate::types::UsageRecord;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub struct UsageDb {
    conn: Mutex<Connection>,
}

impl UsageDb {
    /// Open (or create) a SQLite database at `path`. Creates the parent
    /// directory if needed and chmods the file to 0600.
    pub fn open(path: &str) -> Result<Self, String> {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建 DB 目录失败: {}", e))?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("打开 usage_db 失败 ({}): {}", path, e))?;
        // Restrict permissions to owner only — keys/quota are sensitive.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create tables and indexes (idempotent). Sets WAL mode.
    pub fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS usage_records (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts_ms INTEGER NOT NULL,
                 model TEXT NOT NULL,
                 upstream TEXT NOT NULL,
                 prompt_tokens INTEGER NOT NULL,
                 completion_tokens INTEGER NOT NULL,
                 total_tokens INTEGER NOT NULL,
                 success INTEGER NOT NULL,
                 latency_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_usage_records_ts ON usage_records(ts_ms);
             CREATE TABLE IF NOT EXISTS quota_usage (
                 key_id TEXT PRIMARY KEY,
                 tokens INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );",
        )
        .map_err(|e| format!("初始化 usage_db schema 失败: {}", e))?;
        Ok(())
    }

    /// Append a usage record.
    pub fn insert_record(&self, r: &UsageRecord) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_records
                (ts_ms, model, upstream, prompt_tokens, completion_tokens,
                 total_tokens, success, latency_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                r.timestamp as i64,
                r.model,
                r.upstream,
                r.prompt_tokens as i64,
                r.completion_tokens as i64,
                r.total_tokens as i64,
                r.success as i64,
                r.latency_ms as i64,
            ],
        )
        .map_err(|e| format!("写入 usage_record 失败: {}", e))?;
        Ok(())
    }

    /// Set the cumulative quota for a key. Replaces the prior value.
    pub fn upsert_quota(&self, key_id: &str, tokens: u64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO quota_usage (key_id, tokens, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key_id) DO UPDATE SET
                tokens = excluded.tokens,
                updated_at_ms = excluded.updated_at_ms",
            params![key_id, tokens as i64, now_ms as i64],
        )
        .map_err(|e| format!("写入 quota_usage 失败: {}", e))?;
        Ok(())
    }

    /// Load records with `ts_ms > since_ms`, newest first, capped at `limit`.
    pub fn load_recent(&self, since_ms: u64, limit: usize) -> Result<Vec<UsageRecord>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT ts_ms, model, upstream, prompt_tokens, completion_tokens,
                        total_tokens, success, latency_ms
                 FROM usage_records
                 WHERE ts_ms > ?1
                 ORDER BY ts_ms DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("查询 usage_records 失败: {}", e))?;
        let rows = stmt
            .query_map(params![since_ms as i64, limit as i64], |row| {
                Ok(UsageRecord {
                    timestamp: row.get::<_, i64>(0)? as u64,
                    model: row.get(1)?,
                    upstream: row.get(2)?,
                    prompt_tokens: row.get::<_, i64>(3)? as u32,
                    completion_tokens: row.get::<_, i64>(4)? as u32,
                    total_tokens: row.get::<_, i64>(5)? as u32,
                    success: row.get::<_, i64>(6)? != 0,
                    latency_ms: row.get::<_, i64>(7)? as u64,
                })
            })
            .map_err(|e| format!("遍历 usage_records 失败: {}", e))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("读取 usage_record 行失败: {}", e))?);
        }
        Ok(out)
    }

    /// Load all persisted quota totals (key_id → tokens).
    pub fn load_all_quotas(&self) -> Result<HashMap<String, u64>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key_id, tokens FROM quota_usage")
            .map_err(|e| format!("查询 quota_usage 失败: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })
            .map_err(|e| format!("遍历 quota_usage 失败: {}", e))?;
        let mut out = HashMap::new();
        for r in rows {
            let (k, v) = r.map_err(|e| format!("读取 quota_usage 行失败: {}", e))?;
            out.insert(k, v);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_path(name: &str) -> String {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = dir.join(format!("pstep_test_{}_{}_{}.db", name, pid, nanos));
        p.to_string_lossy().to_string()
    }

    fn cleanup(path: &str) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path));
        let _ = std::fs::remove_file(format!("{}-shm", path));
    }

    fn make_record(model: &str, ts_ms: u64, total: u32) -> UsageRecord {
        UsageRecord {
            timestamp: ts_ms,
            model: model.to_string(),
            upstream: "openai".to_string(),
            prompt_tokens: total / 2,
            completion_tokens: total - total / 2,
            total_tokens: total,
            success: true,
            latency_ms: 42,
        }
    }

    #[test]
    fn open_creates_db_and_migrate_is_idempotent() {
        let path = tmp_path("open");
        let db = UsageDb::open(&path).expect("open");
        db.migrate().expect("migrate1");
        db.migrate().expect("migrate2 must be idempotent");
        assert!(std::path::Path::new(&path).exists());
        cleanup(&path);
    }

    #[test]
    fn insert_and_load_recent_roundtrip() {
        let path = tmp_path("rt");
        let db = UsageDb::open(&path).unwrap();
        db.migrate().unwrap();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        db.insert_record(&make_record("mimo", now_ms - 1000, 100))
            .unwrap();
        db.insert_record(&make_record("minimax", now_ms - 500, 200))
            .unwrap();
        db.insert_record(&make_record("old", now_ms - 10_000, 999))
            .unwrap();

        let recent = db.load_recent(now_ms - 5000, 10).unwrap();
        assert_eq!(recent.len(), 2, "old record must be filtered by ts_ms");
        // Newest first
        assert_eq!(recent[0].model, "minimax");
        assert_eq!(recent[1].model, "mimo");
        assert_eq!(recent[0].total_tokens, 200);
        cleanup(&path);
    }

    #[test]
    fn upsert_quota_replaces_value() {
        let path = tmp_path("quota");
        let db = UsageDb::open(&path).unwrap();
        db.migrate().unwrap();
        db.upsert_quota("k1", 100).unwrap();
        db.upsert_quota("k1", 250).unwrap();
        db.upsert_quota("k2", 50).unwrap();

        let map = db.load_all_quotas().unwrap();
        assert_eq!(map.get("k1"), Some(&250));
        assert_eq!(map.get("k2"), Some(&50));
        assert_eq!(map.len(), 2);
        cleanup(&path);
    }

    #[test]
    fn open_creates_missing_parent_dir() {
        let dir = std::env::temp_dir().join(format!(
            "pstep_test_nested_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("subdir/usage.db").to_string_lossy().to_string();
        let db = UsageDb::open(&path).expect("open with missing parent");
        db.migrate().unwrap();
        assert!(std::path::Path::new(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
