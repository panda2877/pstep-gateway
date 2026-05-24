use crate::types::GatewayConfig;
use crate::providers::{self, OutputFormat};
use crate::usage::UsageTracker;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct Router {
    config: GatewayConfig,
    usage_tracker: Arc<UsageTracker>,
    last_failover: Arc<RwLock<bool>>,
}

impl Router {
    pub fn new(config: GatewayConfig) -> Self {
        let usage_tracking = config.usage_tracking.clone();
        Self {
            config,
            usage_tracker: Arc::new(UsageTracker::new(
                usage_tracking.enabled,
                usage_tracking.retention_hours,
            )),
            last_failover: Arc::new(RwLock::new(false)),
        }
    }

    pub fn get_usage_tracker(&self) -> Arc<UsageTracker> {
        self.usage_tracker.clone()
    }

    pub async fn did_failover(&self) -> bool {
        *self.last_failover.read().await
    }

    pub async fn route_non_stream(
        &self,
        model_name: &str,
        body: &str,
        format: OutputFormat,
    ) -> Result<String, String> {
        let route = self.config.models.get(model_name)
            .ok_or_else(|| format!("未知模型: {}。可用模型: {}",
                model_name,
                self.config.models.keys().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))?;

        let upstream = self.config.upstreams.get(&route.upstream)
            .ok_or_else(|| format!("upstream {} 不存在", route.upstream))?;

        let start = std::time::Instant::now();

        match providers::proxy_non_stream(upstream, &route.model, body, format).await {
            Ok(response) => {
                *self.last_failover.write().await = false;

                self.usage_tracker.record(crate::types::UsageRecord {
                    model: model_name.to_string(),
                    upstream: route.upstream.clone(),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    success: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                });

                Ok(response)
            }
            Err(e) => {
                if let Some(fallback) = &route.fallback {
                    let fallback_route = self.config.models.get(fallback)
                        .ok_or_else(|| format!("fallback {} 不存在", fallback))?;
                    let fallback_upstream = self.config.upstreams.get(&fallback_route.upstream)
                        .ok_or_else(|| format!("fallback upstream {} 不存在", fallback_route.upstream))?;

                    println!("⚠️  主模型 {} 失败，切换到 fallback: {}", route.model, fallback);

                    *self.last_failover.write().await = true;

                    match providers::proxy_non_stream(fallback_upstream, &fallback_route.model, body, format).await {
                        Ok(response) => {
                            self.usage_tracker.record(crate::types::UsageRecord {
                                model: model_name.to_string(),
                                upstream: fallback_route.upstream.clone(),
                                prompt_tokens: 0,
                                completion_tokens: 0,
                                total_tokens: 0,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                                success: true,
                                latency_ms: start.elapsed().as_millis() as u64,
                            });
                            Ok(response)
                        }
                        Err(e2) => Err(format!("所有上游都失败：{} → {}: {} → {}", route.model, fallback, e, e2))
                    }
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn route(
        &self,
        model_name: &str,
        body: &str,
        format: OutputFormat,
    ) -> Result<String, String> {
        let route = self.config.models.get(model_name)
            .ok_or_else(|| format!("未知模型: {}。可用模型: {}",
                model_name,
                self.config.models.keys().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))?;

        let upstream = self.config.upstreams.get(&route.upstream)
            .ok_or_else(|| format!("upstream {} 不存在", route.upstream))?;

        let start = std::time::Instant::now();

        match providers::proxy(upstream, &route.model, body, format).await {
            Ok(response) => {
                *self.last_failover.write().await = false;

                self.usage_tracker.record(crate::types::UsageRecord {
                    model: model_name.to_string(),
                    upstream: route.upstream.clone(),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    success: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                });

                Ok(response)
            }
            Err(e) => {
                if let Some(fallback) = &route.fallback {
                    let fallback_route = self.config.models.get(fallback)
                        .ok_or_else(|| format!("fallback {} 不存在", fallback))?;
                    let fallback_upstream = self.config.upstreams.get(&fallback_route.upstream)
                        .ok_or_else(|| format!("fallback upstream {} 不存在", fallback_route.upstream))?;

                    println!("⚠️  主模型 {} 失败，切换到 fallback: {}", route.model, fallback);

                    *self.last_failover.write().await = true;

                    match providers::proxy(fallback_upstream, &fallback_route.model, body, format).await {
                        Ok(response) => {
                            self.usage_tracker.record(crate::types::UsageRecord {
                                model: model_name.to_string(),
                                upstream: fallback_route.upstream.clone(),
                                prompt_tokens: 0,
                                completion_tokens: 0,
                                total_tokens: 0,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                                success: true,
                                latency_ms: start.elapsed().as_millis() as u64,
                            });
                            Ok(response)
                        }
                        Err(e2) => Err(format!("所有上游都失败：{} → {}: {} {}", route.model, fallback, e, e2))
                    }
                } else {
                    Err(e)
                }
            }
        }
    }
}