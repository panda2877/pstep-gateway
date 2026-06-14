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

    /// 构建完整 fallback 链：主模型 → 策略里的 chain 节点（按 model id 匹配）。
    ///
    /// 策略里的 `upstream` 字段是语义标签，仅用于 UI 展示；`model` 是 `config.models` 的 key。
    /// `key_fallback_policy` 来自请求附带的 API Key 元数据，可覆盖 model 默认策略。
    fn build_chain(
        &self,
        model_name: &str,
        key_fallback_policy: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let route = self.config.models.get(model_name).ok_or_else(|| {
            format!(
                "未知模型: {}。可用模型: {}",
                model_name,
                self.config.models
                    .keys()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        // 链：存的是 model id（即 config.models 的 key）
        let mut chain: Vec<String> = vec![model_name.to_string()];

        let policy_id = key_fallback_policy.or(route.fallback_policy.as_deref());

        if let Some(pid) = policy_id {
            if let Some(policy) = self.config.fallback_policies.get(pid) {
                if policy.enabled {
                    for node in &policy.chain {
                        if node.model == model_name {
                            continue;
                        }
                        if !self.config.models.contains_key(&node.model) {
                            tracing::warn!(
                                target: "router",
                                policy = %pid,
                                model = %node.model,
                                "fallback 策略节点 model id 不在 models 中，跳过"
                            );
                            continue;
                        }
                        chain.push(node.model.clone());
                    }
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
        key_fallback_policy: Option<&str>,
    ) -> Result<String, String> {
        let chain = self.build_chain(model_name, key_fallback_policy)?;
        let route = self.config.models.get(model_name).unwrap();
        let primary_upstream = route.upstream_type.as_str().to_string();

        let start = std::time::Instant::now();
        let mut last_error = String::new();

        for (i, target_model_id) in chain.iter().enumerate() {
            let target_route = self
                .config
                .models
                .get(target_model_id)
                .ok_or_else(|| format!("fallback 模型 {} 不存在", target_model_id))?;

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

            let upstream = target_route.as_upstream();
            match providers::proxy(&upstream, &target_route.model, body, format).await {
                Ok((response, usage)) => {
                    *self.last_failover.write().await = i > 0;

                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_success(&primary_upstream, &route.model).await;
                        }
                    }

                    self.usage_tracker.record(crate::types::UsageRecord {
                        model: target_route.model.clone(),
                        upstream: target_route.upstream_type.as_str().to_string(),
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
                        model = %target_model_id,
                        upstream = %target_route.upstream_type.as_str(),
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

    /// Route a non-streaming request through the fallback chain
    pub async fn route_non_stream(
        &self,
        model_name: &str,
        body: &str,
        format: OutputFormat,
        key_fallback_policy: Option<&str>,
    ) -> Result<String, String> {
        let chain = self.build_chain(model_name, key_fallback_policy)?;
        let route = self.config.models.get(model_name).unwrap();
        let primary_upstream = route.upstream_type.as_str().to_string();

        let start = std::time::Instant::now();
        let mut last_error = String::new();

        for (i, target_model_id) in chain.iter().enumerate() {
            let target_route = self
                .config
                .models
                .get(target_model_id)
                .ok_or_else(|| format!("fallback 模型 {} 不存在", target_model_id))?;

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

            let upstream = target_route.as_upstream();
            match providers::proxy_non_stream(&upstream, &target_route.model, body, format)
                .await
            {
                Ok((response, usage)) => {
                    *self.last_failover.write().await = i > 0;

                    if i == 0 {
                        if let Some(ref tracker) = self.thaw_tracker {
                            tracker.record_success(&primary_upstream, &route.model).await;
                        }
                    }

                    self.usage_tracker.record(crate::types::UsageRecord {
                        model: target_route.model.clone(),
                        upstream: target_route.upstream_type.as_str().to_string(),
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
                        model = %target_model_id,
                        upstream = %target_route.upstream_type.as_str(),
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
