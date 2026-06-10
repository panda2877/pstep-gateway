use crate::types::{ModelHealthStatus, ThawConfig};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Health state for a specific (upstream, model) combination
#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    Healthy,
    Recovering,
    Frozen,
}

impl HealthState {
    fn as_str(&self) -> &'static str {
        match self {
            HealthState::Healthy => "healthy",
            HealthState::Recovering => "recovering",
            HealthState::Frozen => "frozen",
        }
    }
}

#[derive(Debug, Clone)]
struct ModelHealth {
    total_requests: u64,
    failed_requests: u64,
    frozen_until: Option<u64>,
    state: HealthState,
    recovering_attempts: u8,
}

impl ModelHealth {
    fn new() -> Self {
        Self {
            total_requests: 0,
            failed_requests: 0,
            frozen_until: None,
            state: HealthState::Healthy,
            recovering_attempts: 0,
        }
    }

    fn success_rate(&self) -> f32 {
        if self.total_requests == 0 {
            return 1.0;
        }
        (self.total_requests - self.failed_requests) as f32 / self.total_requests as f32
    }

    fn to_status(&self) -> ModelHealthStatus {
        let mut status = ModelHealthStatus {
            state: self.state.as_str().to_string(),
            success_rate: self.success_rate(),
            total_requests: self.total_requests,
            failed_requests: self.failed_requests,
            frozen_until: None,
        };

        if let Some(ts) = self.frozen_until {
            status.frozen_until = Some(format_timestamp(ts));
        }

        status
    }
}

fn format_timestamp(ts: u64) -> String {
    SystemTime::UNIX_EPOCH
        .checked_add(std::time::Duration::from_millis(ts))
        .map(|t| {
            let datetime: chrono::DateTime<chrono::Utc> = t.into();
            datetime.to_rfc3339()
        })
        .unwrap_or_else(|| ts.to_string())
}

pub struct ThawTracker {
    stats: RwLock<HashMap<String, ModelHealth>>,
    config: ThawConfig,
}

impl ThawTracker {
    pub fn new(config: ThawConfig) -> Self {
        Self {
            stats: RwLock::new(HashMap::new()),
            config,
        }
    }

    fn make_key(upstream: &str, model: &str) -> String {
        format!("{}/{}", upstream, model)
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Check if a model is currently frozen
    pub async fn is_frozen(&self, upstream: &str, model: &str) -> bool {
        let stats = self.stats.read().await;
        let Some(health) = stats.get(&Self::make_key(upstream, model)) else {
            return false;
        };

        if health.state != HealthState::Frozen {
            return false;
        }

        // Check if freeze period has expired
        if let Some(frozen_until) = health.frozen_until {
            // Still frozen if current time is before the freeze expiration
            Self::now_ms() < frozen_until
        } else {
            // No frozen_until set, consider it not frozen
            false
        }
    }

    /// Check if a model is in recovering state and ready for a test request
    pub async fn should_test_recovery(&self, upstream: &str, model: &str) -> bool {
        let stats = self.stats.read().await;
        let Some(health) = stats.get(&Self::make_key(upstream, model)) else {
            return false;
        };

        health.state == HealthState::Recovering
    }

    /// Record a successful request
    pub async fn record_success(&self, upstream: &str, model: &str) {
        let mut stats = self.stats.write().await;
        let health = stats.entry(Self::make_key(upstream, model)).or_insert_with(ModelHealth::new);

        health.total_requests += 1;

        // If recovering, increment attempt counter
        if health.state == HealthState::Recovering {
            health.recovering_attempts += 1;

            // If we have enough consecutive successes, recover permanently
            if health.recovering_attempts >= self.config.recovering_attempts {
                tracing::info!(
                    upstream = %upstream,
                    model = %model,
                    success_rate = health.success_rate(),
                    "model recovered, permanently enabled"
                );
                health.state = HealthState::Healthy;
                health.recovering_attempts = 0;
            }
        }
    }

    /// Record a failed request
    pub async fn record_failure(&self, upstream: &str, model: &str) {
        let mut stats = self.stats.write().await;
        let health = stats.entry(Self::make_key(upstream, model)).or_insert_with(ModelHealth::new);

        health.total_requests += 1;
        health.failed_requests += 1;

        // If recovering and got a failure, freeze again
        if health.state == HealthState::Recovering {
            tracing::warn!(
                upstream = %upstream,
                model = %model,
                success_rate = health.success_rate(),
                "recovery attempt failed, re-freezing"
            );
            Self::freeze(health, &self.config);
        }
    }

    /// Check if primary model should be frozen based on failure rate
    pub async fn check_and_freeze(&self, upstream: &str, model: &str) {
        let mut stats = self.stats.write().await;
        let Some(health) = stats.get_mut(&Self::make_key(upstream, model)) else {
            return;
        };

        // Don't freeze if already frozen or recovering
        if health.state != HealthState::Healthy {
            return;
        }

        // Need minimum requests to make a decision
        if health.total_requests < self.config.min_requests_to_freeze {
            return;
        }

        // Freeze if failure rate > 50%
        if health.success_rate() < 0.5 {
            tracing::warn!(
                upstream = %upstream,
                model = %model,
                success_rate = health.success_rate(),
                failures = health.failed_requests,
                total = health.total_requests,
                freeze_duration_minutes = self.config.freeze_duration_minutes,
                "model frozen due to high failure rate"
            );
            Self::freeze(health, &self.config);
        }
    }

    fn freeze(health: &mut ModelHealth, config: &ThawConfig) {
        let freeze_duration_ms = config.freeze_duration_minutes as u64 * 60 * 1000;
        health.frozen_until = Some(Self::now_ms().saturating_add(freeze_duration_ms));
        health.state = HealthState::Frozen;
        health.recovering_attempts = 0;
    }

    /// Transition from Frozen to Recovering (called when frozen period expires)
    pub async fn try_thaw(&self, upstream: &str, model: &str) -> bool {
        let mut stats = self.stats.write().await;
        let Some(health) = stats.get_mut(&Self::make_key(upstream, model)) else {
            return false;
        };

        if health.state != HealthState::Frozen {
            return false;
        }

        // Check if freeze period has expired
        if let Some(frozen_until) = health.frozen_until {
            if Self::now_ms() < frozen_until {
                return false;
            }
        }

        tracing::info!(
            upstream = %upstream,
            model = %model,
            "model entering recovery phase"
        );
        health.state = HealthState::Recovering;
        health.recovering_attempts = 0;
        true
    }

    /// Get health status for all models
    pub async fn get_all_health(&self) -> HashMap<String, ModelHealthStatus> {
        let stats = self.stats.read().await;
        stats
            .iter()
            .map(|(k, v)| (k.clone(), v.to_status()))
            .collect()
    }

    /// Get health status for a specific model
    pub async fn get_health(&self, upstream: &str, model: &str) -> Option<ModelHealthStatus> {
        let stats = self.stats.read().await;
        stats.get(&Self::make_key(upstream, model)).map(|h| h.to_status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ThawConfig {
        ThawConfig {
            freeze_duration_minutes: 15,
            recovery_threshold: 0.8,
            min_requests_to_freeze: 5,
            recovering_attempts: 3,
        }
    }

    #[tokio::test]
    async fn test_initial_state_is_healthy() {
        let tracker = ThawTracker::new(test_config());
        assert!(!tracker.is_frozen("openai", "gpt-4o").await);
    }

    #[tokio::test]
    async fn test_freeze_on_high_failure_rate() {
        let tracker = ThawTracker::new(test_config());

        // Record 10 requests with 6 failures (40% success rate)
        for _ in 0..6 {
            tracker.record_failure("openai", "gpt-4o").await;
        }
        for _ in 0..4 {
            tracker.record_success("openai", "gpt-4o").await;
        }

        // Check and freeze
        tracker.check_and_freeze("openai", "gpt-4o").await;

        // Should be frozen now
        assert!(tracker.is_frozen("openai", "gpt-4o").await);
    }

    #[tokio::test]
    async fn test_recovery_flow() {
        let tracker = ThawTracker::new(test_config());

        // Freeze the model with an already expired freeze time
        {
            let mut stats = tracker.stats.write().await;
            let health = stats.entry("openai/gpt-4o".to_string()).or_insert_with(ModelHealth::new);
            health.state = HealthState::Frozen;
            // Set frozen_until to a time in the past (now - 1ms)
            health.frozen_until = Some(ThawTracker::now_ms() - 1);
        }

        // Try to thaw - should succeed since freeze expired
        let thawed = tracker.try_thaw("openai", "gpt-4o").await;
        assert!(thawed);

        // Record 3 successful attempts
        for _ in 0..3 {
            tracker.record_success("openai", "gpt-4o").await;
        }

        // Should be healthy now
        let health = tracker.get_health("openai", "gpt-4o").await.unwrap();
        assert_eq!(health.state, "healthy");
    }

    #[tokio::test]
    async fn test_freeze_expiration() {
        let tracker = ThawTracker::new(test_config());

        // Manually set a frozen state
        {
            let mut stats = tracker.stats.write().await;
            let health = stats.entry("openai/gpt-4o".to_string()).or_insert_with(ModelHealth::new);
            health.state = HealthState::Frozen;
            // Set frozen_until to 1 minute in the future
            health.frozen_until = Some(ThawTracker::now_ms() + 60000);
        }

        // Should be frozen (freeze period not expired)
        assert!(tracker.is_frozen("openai", "gpt-4o").await);

        // Manually set a frozen state with expired time
        {
            let mut stats = tracker.stats.write().await;
            let health = stats.entry("openai/gpt-4o".to_string()).or_insert_with(ModelHealth::new);
            health.state = HealthState::Frozen;
            // Set frozen_until to the past
            health.frozen_until = Some(ThawTracker::now_ms() - 1000);
        }

        // Should NOT be frozen (freeze period expired)
        assert!(!tracker.is_frozen("openai", "gpt-4o").await);
    }

    #[tokio::test]
    async fn test_recovering_state() {
        let tracker = ThawTracker::new(test_config());

        // Set to recovering state
        {
            let mut stats = tracker.stats.write().await;
            let health = stats.entry("openai/gpt-4o".to_string()).or_insert_with(ModelHealth::new);
            health.state = HealthState::Recovering;
            health.recovering_attempts = 1;
        }

        // Should not be frozen (in recovering state)
        assert!(!tracker.is_frozen("openai", "gpt-4o").await);

        // Record 2 more successes to reach recovering_attempts threshold
        for _ in 0..2 {
            tracker.record_success("openai", "gpt-4o").await;
        }

        // Should be healthy now
        let health = tracker.get_health("openai", "gpt-4o").await.unwrap();
        assert_eq!(health.state, "healthy");
    }

    #[tokio::test]
    async fn test_recovering_failure_resets() {
        let tracker = ThawTracker::new(test_config());

        // Set to recovering state with 2 attempts
        {
            let mut stats = tracker.stats.write().await;
            let health = stats.entry("openai/gpt-4o".to_string()).or_insert_with(ModelHealth::new);
            health.state = HealthState::Recovering;
            health.recovering_attempts = 2;
            health.total_requests = 10;
        }

        // Record a failure during recovery
        tracker.record_failure("openai", "gpt-4o").await;

        // Should be frozen again
        let health = tracker.get_health("openai", "gpt-4o").await.unwrap();
        assert_eq!(health.state, "frozen");
    }
}