use crate::types::{UsageRecord, UsageStats};
use crate::usage_db::UsageDb;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

const RECENT_CAP: usize = 10_000;

pub struct UsageTracker {
    /// Bounded ring of recent records. Seeded from DB on startup when DB is
    /// configured; otherwise just grows in memory up to RECENT_CAP.
    recent: RwLock<VecDeque<UsageRecord>>,
    /// Aggregated stats mirror; updated on every record().
    stats: RwLock<UsageStats>,
    retention_ms: u64,
    enabled: bool,
    /// None = in-memory only (legacy behavior, before usage_db was added).
    db: Option<Arc<UsageDb>>,
}

impl UsageTracker {
    /// In-memory only — same as pre-persistence behavior.
    pub fn new(enabled: bool, retention_hours: u32) -> Self {
        Self {
            recent: RwLock::new(VecDeque::with_capacity(RECENT_CAP)),
            stats: RwLock::new(UsageStats {
                total_requests: 0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_tokens: 0,
                by_model: HashMap::new(),
                by_upstream: HashMap::new(),
            }),
            retention_ms: retention_hours as u64 * 60 * 60 * 1000,
            enabled,
            db: None,
        }
    }

    /// Backed by a SQLite DB. Loads the last `retention_hours` of records from
    /// the DB on startup so admin views show historical data after a restart.
    pub fn with_db(enabled: bool, retention_hours: u32, db: Arc<UsageDb>) -> Self {
        let retention_ms = retention_hours as u64 * 60 * 60 * 1000;
        let cutoff_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            .saturating_sub(retention_ms);

        let loaded: Vec<UsageRecord> = db
            .load_recent(cutoff_ms, RECENT_CAP)
            .unwrap_or_else(|e| {
                tracing::warn!(target: "usage_db", error = %e, "load_recent 失败，使用空缓冲");
                Vec::new()
            });

        // Replay into in-memory state
        let mut recent: VecDeque<UsageRecord> = VecDeque::with_capacity(RECENT_CAP);
        let mut stats = UsageStats {
            total_requests: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            by_model: HashMap::new(),
            by_upstream: HashMap::new(),
        };
        for r in loaded {
            accumulate(&mut stats, &r);
            recent.push_back(r);
        }
        // Cap to RECENT_CAP (drop oldest)
        while recent.len() > RECENT_CAP {
            recent.pop_front();
        }

        Self {
            recent: RwLock::new(recent),
            stats: RwLock::new(stats),
            retention_ms,
            enabled,
            db: Some(db),
        }
    }

    pub fn record(&self, record: UsageRecord) {
        if !self.enabled {
            return;
        }

        // Update in-memory mirror first (cheap, lock-held)
        {
            let mut stats = self.stats.write().unwrap();
            accumulate(&mut stats, &record);
        }
        {
            let mut recent = self.recent.write().unwrap();
            recent.push_back(record.clone());
            while recent.len() > RECENT_CAP {
                recent.pop_front();
            }
        }

        // Persist to DB. Errors are logged but not propagated — a DB
        // hiccup must not fail a chat request.
        if let Some(db) = &self.db {
            if let Err(e) = db.insert_record(&record) {
                tracing::warn!(
                    target: "usage_db",
                    error = %e,
                    "usage_record 写库失败；内存已保留"
                );
            }
        }
    }

    pub fn get_stats(&self) -> UsageStats {
        self.cleanup();
        self.stats.read().unwrap().clone()
    }

    pub fn get_recent(&self, n: usize) -> Vec<UsageRecord> {
        self.cleanup();
        let recent = self.recent.read().unwrap();
        recent.iter().rev().take(n).cloned().collect()
    }

    /// Drop records older than `retention_ms` from the in-memory buffer.
    fn cleanup(&self) {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            .saturating_sub(self.retention_ms);

        let mut recent = self.recent.write().unwrap();
        recent.retain(|r| r.timestamp > cutoff);
    }
}

fn accumulate(stats: &mut UsageStats, r: &UsageRecord) {
    stats.total_requests += 1;
    stats.total_prompt_tokens += r.prompt_tokens;
    stats.total_completion_tokens += r.completion_tokens;
    stats.total_tokens += r.total_tokens;

    let m = stats
        .by_model
        .entry(r.model.clone())
        .or_default();
    m.requests += 1;
    m.tokens += r.total_tokens;

    let u = stats
        .by_upstream
        .entry(r.upstream.clone())
        .or_default();
    u.requests += 1;
    u.tokens += r.total_tokens;
}
