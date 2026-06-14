use crate::config::save_config;
use crate::types::{ModelConfig, ModelStatus, UpdateModelConfigRequest, UpstreamType};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

/// Mask an api_key for display: keep first 3 and last 4 chars, replace middle with "***".
fn mask_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let len = key.chars().count();
    if len <= 8 {
        return "*".repeat(len);
    }
    let head: String = key.chars().take(3).collect();
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}{}{}", head, "*".repeat(len - 7), tail)
}

/// 把 ModelRoute 转换为响应给前端的 ModelConfig（含展开的 fallback_chain）。
fn build_model_config(
    id: &str,
    route: &crate::types::ModelRoute,
    policy: Option<&crate::types::FallbackPolicyConfig>,
) -> ModelConfig {
    let metadata = route.metadata.as_ref();
    let name = metadata
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| id.to_string());

    let api_key_masked = if route.api_key.is_empty() {
        None
    } else {
        Some(mask_api_key(&route.api_key))
    };

    let fallback_chain = policy
        .filter(|p| p.enabled)
        .map(|p| p.chain.clone())
        .unwrap_or_default();

    ModelConfig {
        id: id.to_string(),
        name,
        provider: route.upstream_type.as_str().to_string(),
        version: "2024-01-01".to_string(),
        status: metadata
            .map(|m| m.status.as_str().to_string())
            .unwrap_or_else(|| ModelStatus::Active.as_str().to_string()),
        timeout_secs: 30,
        price_per_input: metadata.and_then(|m| m.price_per_input),
        price_per_output: metadata.and_then(|m| m.price_per_output),
        upstream: route.upstream_type.as_str().to_string(),
        upstream_type: route.upstream_type.as_str().to_string(),
        fallback_policy: route.fallback_policy.clone(),
        fallback_chain,
        base_url: Some(route.base_url.clone()),
        api_key_masked,
        api_key_configured: !route.api_key.is_empty(),
        upstream_model: route.model.clone(),
    }
}

/// GET /api/admin/models
pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock().unwrap();
    let models: Vec<ModelConfig> = config
        .models
        .iter()
        .map(|(id, route)| {
            let policy = route
                .fallback_policy
                .as_ref()
                .and_then(|pid| config.fallback_policies.get(pid));
            build_model_config(id, route, policy)
        })
        .collect();

    Json(serde_json::json!({ "models": models }))
}

/// GET /api/admin/models/:id
pub async fn get_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let config = state.config.lock().unwrap();
    match config.models.get(&id) {
        Some(route) => {
            let policy = route
                .fallback_policy
                .as_ref()
                .and_then(|pid| config.fallback_policies.get(pid));
            Json(build_model_config(&id, route, policy)).into_response()
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        )
            .into_response(),
    }
}

/// PUT /api/admin/models/:id
///
/// 字段分组（决策 #4）：
/// - 热更新：name, status, price_per_input, price_per_output, fallback_policy
/// - 需重启：type (upstream_type), base_url, api_key, model
pub async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateModelConfigRequest>,
) -> Response {
    // 提前克隆出所有需要持有的值，缩短锁区
    let name = req.name.clone();
    let status_str = req.status.clone();
    let price_input = req.price_per_input;
    let price_output = req.price_per_output;
    let upstream_type_str = req.upstream_type.clone();
    let base_url = req.base_url.clone();
    let model = req.model.clone();
    let api_key = req.api_key.clone();
    let fallback_policy = req.fallback_policy.clone();

    // 解析 status
    let new_status = match status_str.as_deref() {
        Some(s) => match s {
            "active" => Some(ModelStatus::Active),
            "rate_limited" => Some(ModelStatus::RateLimited),
            "disabled" => Some(ModelStatus::Disabled),
            _ => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "invalid_status",
                        "message": format!(
                            "status 必须是 active / rate_limited / disabled，收到: {}",
                            s
                        )
                    })),
                )
                    .into_response();
            }
        },
        None => None,
    };

    // 解析 upstream_type
    let new_upstream_type = match upstream_type_str.as_deref() {
        Some(s) => match s {
            "openai" => Some(UpstreamType::Openai),
            "anthropic" => Some(UpstreamType::Anthropic),
            _ => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "invalid_type",
                        "message": format!(
                            "upstream_type 必须是 openai / anthropic，收到: {}",
                            s
                        )
                    })),
                )
                    .into_response();
            }
        },
        None => None,
    };

    let mut config = state.config.lock().unwrap();

    if !config.models.contains_key(&id) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        )
            .into_response();
    }

    // 检测是否触发了「需重启」字段
    let restart_required = new_upstream_type.is_some()
        || base_url.is_some()
        || api_key
            .as_ref()
            .map(|v| !v.is_empty() && v != "********")
            .unwrap_or(false)
        || model.is_some();

    // 写盘前先验证 fallback_policy 引用（非空才校验）
    if let Some(ref fp) = fallback_policy {
        if !fp.is_empty() && !config.fallback_policies.contains_key(fp) {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_fallback_policy",
                    "message": format!("fallback_policy '{}' 不存在", fp)
                })),
            )
                .into_response();
        }
    }

    if let Some(route) = config.models.get_mut(&id) {
        // --- 热更新字段 ---
        if let Some(n) = name.clone() {
            match &mut route.metadata {
                Some(m) => m.name = Some(n),
                None => {
                    route.metadata = Some(crate::types::ModelMetadata {
                        name: Some(n),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(s) = new_status {
            match &mut route.metadata {
                Some(m) => m.status = s,
                None => {
                    route.metadata = Some(crate::types::ModelMetadata {
                        status: s,
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(p) = price_input {
            match &mut route.metadata {
                Some(m) => m.price_per_input = Some(p),
                None => {
                    route.metadata = Some(crate::types::ModelMetadata {
                        price_per_input: Some(p),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(p) = price_output {
            match &mut route.metadata {
                Some(m) => m.price_per_output = Some(p),
                None => {
                    route.metadata = Some(crate::types::ModelMetadata {
                        price_per_output: Some(p),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(ref fp) = fallback_policy {
            route.fallback_policy = if fp.is_empty() { None } else { Some(fp.clone()) };
        }

        // --- 需重启字段 ---
        if let Some(ut) = new_upstream_type {
            route.upstream_type = ut;
        }
        if let Some(ref b) = base_url {
            let trimmed = b.trim();
            if !trimmed.is_empty() {
                route.base_url = trimmed.to_string();
            }
        }
        if let Some(ref m) = model {
            if !m.is_empty() {
                route.model = m.clone();
            }
        }
        if let Some(ref k) = api_key {
            if !k.is_empty() && k != "********" {
                route.api_key = k.clone();
            }
        }
    }

    if let Err(e) = save_config(&config) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "save_failed",
                "message": e
            })),
        )
            .into_response();
    }

    // 读回最新值
    let updated = config.models.get(&id).map(|route| {
        let policy = route
            .fallback_policy
            .as_ref()
            .and_then(|pid| config.fallback_policies.get(pid));
        build_model_config(&id, route, policy)
    });

    let api_key_changed = api_key
        .as_ref()
        .map(|v| !v.is_empty() && v != "********")
        .unwrap_or(false);

    Json(serde_json::json!({
        "success": true,
        "message": if restart_required {
            "已保存。api_key/base_url/type/model 变更需重启服务生效"
        } else {
            "已保存"
        },
        "model_id": id,
        "restart_required": restart_required,
        "changes": {
            "name": name,
            "status": status_str,
            "price_per_input": price_input,
            "price_per_output": price_output,
            "upstream_type": upstream_type_str,
            "base_url": base_url,
            "model": model,
            "fallback_policy": fallback_policy,
            "api_key_changed": api_key_changed
        },
        "model": updated
    }))
    .into_response()
}

/// GET /api/admin/fallback/policies-mini（前端 model 编辑 modal 用）
pub async fn list_fallback_policies_mini(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let config = state.config.lock().unwrap();
    let policies: Vec<serde_json::Value> = config
        .fallback_policies
        .keys()
        .map(|id| serde_json::json!({ "id": id }))
        .collect();
    Json(serde_json::json!({ "policies": policies }))
}
