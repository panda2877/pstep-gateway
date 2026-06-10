use crate::providers::{self, OutputFormat};
use crate::thaw::ThawTracker;
use crate::types::GatewayConfig;
use crate::usage::UsageTracker;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct Router {
    config: GatewayConfig,
    usage_tracker: Arc<UsageTracker>,
    thaw_tracker: Option<Arc<ThawTracker>>,
    last_failover: Arc<RwLock<bool>>,
}

impl Router {
    pub fn new(config: GatewayConfig, thaw_tracker: Option<Arc<ThawTracker>>) -> Self {
        let usage_tracking = config.usage_tracking.clone();
        Self {
            config,
            usage_tracker: Arc::new(UsageTracker::new(
                usage_tracking.enabled,
                usage_tracking.retention_hours,
            )),
            thaw_tracker,
            last_failover: Arc::new(RwLock::new(false)),
        }
    }

    pub fn get_usage_tracker(&self) -> Arc<UsageTracker> {
        self.usage_tracker.clone()
    }

    pub fn get_thaw_tracker(&self) -> Option<Arc<ThawTracker>> {
        self.thaw_tracker.clone()
    }

    pub async fn did_failover(&self) -> bool {
        *self.last_failover.read().await
    }

    /// Build the full fallback chain: primary model + fallback_chain
    fn build_chain(&self, model_name: &str) -> Result<Vec<String>, String> {
        let route = self.config.models.get(model_name)
            .ok_or_else(|| format!("未知模型: {}。可用模型: {}",
                model_name,
                self.config.models.keys().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))?;

        let mut chain = vec![model_name.to_string()];
        chain.extend(route.fallback_chain.clone());

        // Also include legacy fallback if no chain configured
        if chain.len() == 1 {
            if let Some(legacy_fallback) = &route.fallback {
                if !route.fallback_chain.contains(legacy_fallback) {
                    chain.push(legacy_fallback.clone());
                }
            }
        }

        Ok(chain)
    }

    /// Route a streaming request through the fallback chain
    pub async fn route(
        &self,
        model_name: &str,
        body: &str,
        format: OutputFormat,
    ) -> Result<String, String> {
        let chain = self.build_chain(model_name)?;
        let route = self.config.models.get(model_name).unwrap();
        let primary_upstream = route.upstream.clone();

        let start = std::time::Instant::now();
        let mut last_error = String::new();

        for (i, target_model_name) in chain.iter().enumerate() {
            let target_route = self.config.models.get(target_model_name)
                .ok_or_else(|| format!("fallback 模型 {} 不存在", target_model_name))?;

            let current_upstream = &target_route.upstream;

            // Check freeze status for primary model only
            if i == 0 {
                if let Some(ref tracker) = self.thaw_tracker {
                    if tracker.is_frozen(&primary_upstream, &route.model).await {
                        tracing::warn!(
                            target: "router",
                            upstream = %primary_upstream,
                            model = %route.model,
                            "primary model frozen, skipping to fallback"
                        );
                        continue;
                    }

                    // Check if we should try to thaw
                    if tracker.try_thaw(&primary_upstream, &route.model).await {
                        tracing::info!(
                            target: "router",
                            upstream = %primary_upstream,
                            model = %route.model,
                            "attempting to recover primary model"
                        );
                    }
                }
            }

            let upstream = self.config.upstreams.get(current_upstream)
                .ok_or_else(|| format!("upstream {} 不存在", current_upstream))?;

            match providers::proxy(upstream, &target_route.model, body, format).await {
                Ok((response, usage)) => {
                    *self.last_failover.write().await = i > 0;

                    // Record success in thaw tracker for primary model
                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_success(&primary_upstream, &route.model).await;
                        }
                    }

                    self.usage_tracker.record(crate::types::UsageRecord {
                        model: model_name.to_string(),
                        upstream: current_upstream.clone(),
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: usage.prompt_tokens + usage.completion_tokens,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0),
                        success: true,
                        latency_ms: start.elapsed().as_millis() as u64,
                    });

                    return Ok(response);
                }
                Err(e) => {
                    last_error = e;
                    tracing::error!(
                        target: "router",
                        model = %target_model_name,
                        upstream = %current_upstream,
                        error = %last_error,
                        "upstream request failed"
                    );

                    // Record failure for primary model
                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_failure(&primary_upstream, &route.model).await;
                            tracker.check_and_freeze(&primary_upstream, &route.model).await;
                        }
                    }

                    // Try next in chain
                    continue;
                }
            }
        }

        Err(format!("所有上游都失败: {}", last_error))
    }

    /// Route a non-streaming request through the fallback chain
    pub async fn route_non_stream(
        &self,
        model_name: &str,
        body: &str,
        format: OutputFormat,
    ) -> Result<String, String> {
        let chain = self.build_chain(model_name)?;
        let route = self.config.models.get(model_name).unwrap();
        let primary_upstream = route.upstream.clone();

        let start = std::time::Instant::now();
        let mut last_error = String::new();

        for (i, target_model_name) in chain.iter().enumerate() {
            let target_route = self.config.models.get(target_model_name)
                .ok_or_else(|| format!("fallback 模型 {} 不存在", target_model_name))?;

            let current_upstream = &target_route.upstream;

            // Check freeze status for primary model only
            if i == 0 {
                if let Some(ref tracker) = self.thaw_tracker {
                    if tracker.is_frozen(&primary_upstream, &route.model).await {
                        tracing::warn!(
                            target: "router",
                            upstream = %primary_upstream,
                            model = %route.model,
                            "primary model frozen, skipping to fallback"
                        );
                        continue;
                    }

                    if tracker.try_thaw(&primary_upstream, &route.model).await {
                        tracing::info!(
                            target: "router",
                            upstream = %primary_upstream,
                            model = %route.model,
                            "attempting to recover primary model"
                        );
                    }
                }
            }

            let upstream = self.config.upstreams.get(current_upstream)
                .ok_or_else(|| format!("upstream {} 不存在", current_upstream))?;

            match providers::proxy_non_stream(upstream, &target_route.model, body, format).await {
                Ok((response, usage)) => {
                    *self.last_failover.write().await = i > 0;

                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_success(&primary_upstream, &route.model).await;
                        }
                    }

                    self.usage_tracker.record(crate::types::UsageRecord {
                        model: model_name.to_string(),
                        upstream: current_upstream.clone(),
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: usage.prompt_tokens + usage.completion_tokens,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0),
                        success: true,
                        latency_ms: start.elapsed().as_millis() as u64,
                    });

                    return Ok(response);
                }
                Err(e) => {
                    last_error = e;
                    tracing::error!(
                        target: "router",
                        model = %target_model_name,
                        upstream = %current_upstream,
                        error = %last_error,
                        "upstream request failed"
                    );

                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_failure(&primary_upstream, &route.model).await;
                            tracker.check_and_freeze(&primary_upstream, &route.model).await;
                        }
                    }

                    continue;
                }
            }
        }

        Err(format!("所有上游都失败: {}", last_error))
    }
}