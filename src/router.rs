use crate::providers::{self, OutputFormat};
use crate::thaw::ThawTracker;
use crate::types::GatewayConfig;
use crate::usage::UsageTracker;
use crate::usage_db::UsageDb;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct Router {
    config: GatewayConfig,
    usage_tracker: Arc<UsageTracker>,
    thaw_tracker: Option<Arc<ThawTracker>>,
    last_failover: Arc<RwLock<bool>>,
}

impl Router {
    pub fn new(
        config: GatewayConfig,
        thaw_tracker: Option<Arc<ThawTracker>>,
        usage_db: Option<Arc<UsageDb>>,
    ) -> Self {
        let usage_tracking = config.usage_tracking.clone();
        let usage_tracker = match usage_db {
            Some(db) => Arc::new(UsageTracker::with_db(
                usage_tracking.enabled,
                usage_tracking.retention_hours,
                db,
            )),
            None => Arc::new(UsageTracker::new(
                usage_tracking.enabled,
                usage_tracking.retention_hours,
            )),
        };
        Self {
            config,
            usage_tracker,
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
    /// 决策（v0.3）：model 不再自带 fallback_policy。fallback 关系由请求侧
    /// `key_fallback_policy` 决定：API Key 可指定一个策略覆盖默认（无 key
    /// 时无 fallback）。
    fn build_chain(
        &self,
        model_name: &str,
        key_fallback_policy: Option<&str>,
    ) -> Result<Vec<String>, String> {
        if !self.config.models.contains_key(model_name) {
            return Err(format!(
                "未知模型: {}。可用模型: {}",
                model_name,
                self.config.models
                    .keys()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // 链：存的是 model id（即 config.models 的 key）
        let mut chain: Vec<String> = vec![model_name.to_string()];

        if let Some(pid) = key_fallback_policy {
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
        key_id: Option<&str>,
        quota_tracker: &Arc<tokio::sync::Mutex<crate::admin::apikeys::ApiKeyQuotaTracker>>,
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
            // SSE 转换（convert_sse_*_stream）是纯 CPU 用户态循环，跑多久占 worker 多久。
            // v7 之前 4 worker 同时撞上 → 全进 R/wchan=0 → timer wheel 不转 → 永久 wedge。
            // v8：把转换塞 spawn_blocking，释放 tokio worker 让 reqwest timeout 等定时器跑。
            let proxy_upstream = upstream.clone();
            let proxy_target_model = target_route.model.clone();
            let proxy_body = body.to_string();
            let proxy_format = format;
            let proxy_result = tokio::task::spawn_blocking(move || {
                // providers::proxy 内部已经是阻塞 + .await 混合，但 SSE 转换这一段
                // 是纯 CPU，跑在 blocking pool 里不会占 tokio worker 槽。
                tokio::runtime::Handle::current().block_on(providers::proxy(
                    &proxy_upstream,
                    &proxy_target_model,
                    &proxy_body,
                    proxy_format,
                ))
            })
            .await
            .map_err(|e| format!("blocking task join 失败: {}", e))?;
            match proxy_result {
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

                    // 累计到 client_api_key 的 quota_used（持久化由 tracker 内部处理）
                    if let Some(kid) = key_id {
                        quota_tracker
                            .lock()
                            .await
                            .record(kid, (usage.prompt_tokens + usage.completion_tokens) as u64)
                            .await;
                    }

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
        key_id: Option<&str>,
        quota_tracker: &Arc<tokio::sync::Mutex<crate::admin::apikeys::ApiKeyQuotaTracker>>,
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
            // 同 stream 路径：转换 SSE 这段 CPU work 跑在 blocking pool，详见上注释。
            let proxy_upstream = upstream.clone();
            let proxy_target_model = target_route.model.clone();
            let proxy_body = body.to_string();
            let proxy_format = format;
            let proxy_result = tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(providers::proxy_non_stream(
                    &proxy_upstream,
                    &proxy_target_model,
                    &proxy_body,
                    proxy_format,
                ))
            })
            .await
            .map_err(|e| format!("blocking task join 失败: {}", e))?;
            match proxy_result {
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

                    // 累计到 client_api_key 的 quota_used（持久化由 tracker 内部处理）
                    if let Some(kid) = key_id {
                        quota_tracker
                            .lock()
                            .await
                            .record(kid, (usage.prompt_tokens + usage.completion_tokens) as u64)
                            .await;
                    }

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
