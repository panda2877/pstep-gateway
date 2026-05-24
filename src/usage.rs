use crate::types::{UsageRecord, UsageStats};
use std::collections::HashMap;
use std::sync::RwLock;

pub struct UsageTracker {
    records: RwLock<Vec<UsageRecord>>,
    retention_ms: u64,
    enabled: bool,
}

impl UsageTracker {
    pub fn new(enabled: bool, retention_hours: u32) -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            retention_ms: retention_hours as u64 * 60 * 60 * 1000,
            enabled,
        }
    }

    pub fn record(&self, record: UsageRecord) {
        if !self.enabled {
            return;
        }

        let mut records = self.records.write().unwrap();
        records.push(record);
        drop(records);
        self.cleanup();
    }

    fn cleanup(&self) {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            .saturating_sub(self.retention_ms);

        let mut records = self.records.write().unwrap();
        records.retain(|r| r.timestamp > cutoff);
    }

    pub fn get_stats(&self) -> UsageStats {
        self.cleanup();
        let records = self.records.read().unwrap();

        let total = records.len() as u32;
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        let mut by_model: HashMap<String, crate::types::ModelStats> = HashMap::new();
        let mut by_upstream: HashMap<String, crate::types::UpstreamStats> = HashMap::new();

        for r in records.iter() {
            prompt_tokens += r.prompt_tokens;
            completion_tokens += r.completion_tokens;

            by_model.entry(r.model.clone()).or_default().requests += 1;
            by_model.entry(r.model.clone()).or_default().tokens += r.total_tokens;

            by_upstream.entry(r.upstream.clone()).or_default().requests += 1;
            by_upstream.entry(r.upstream.clone()).or_default().tokens += r.total_tokens;
        }

        UsageStats {
            total_requests: total,
            total_prompt_tokens: prompt_tokens,
            total_completion_tokens: completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            by_model,
            by_upstream,
        }
    }

    pub fn get_recent(&self, n: usize) -> Vec<UsageRecord> {
        self.cleanup();
        let records = self.records.read().unwrap();
        records.iter().rev().take(n).cloned().collect()
    }
}