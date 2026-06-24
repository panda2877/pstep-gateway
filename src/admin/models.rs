use crate::types::{
    CreateModelRequest, ModelConfig, ModelMetadata, ModelRoute, ModelStatus,
    UpdateModelConfigRequest, UpstreamType,
};
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

/// 把 ModelRoute 转换为响应给前端的 ModelConfig。
fn build_model_config(
    id: &str,
    route: &ModelRoute,
    referenced_by_policies: Vec<String>,
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

    ModelConfig {
        id: id.to_string(),
        name,
        version: "2024-01-01".to_string(),
        status: metadata
            .map(|m| m.status.as_str().to_string())
            .unwrap_or_else(|| ModelStatus::Active.as_str().to_string()),
        timeout_secs: 30,
        price_per_input: metadata.and_then(|m| m.price_per_input),
        price_per_output: metadata.and_then(|m| m.price_per_output),
        referenced_by_policies,
        base_url: Some(route.base_url.clone()),
        api_key_masked,
        api_key_configured: !route.api_key.is_empty(),
        upstream_model: route.model.clone(),
    }
}

/// 找出引用了此 model 的所有 fallback policy id。
fn policies_referencing_model(
    config: &crate::types::GatewayConfig,
    model_id: &str,
) -> Vec<String> {
    let mut out: Vec<String> = config
        .fallback_policies
        .iter()
        .filter(|(_, p)| p.chain.iter().any(|n| n.model == model_id))
        .map(|(id, _)| id.clone())
        .collect();
    out.sort();
    out
}

/// GET /api/admin/models
pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let models: Vec<ModelConfig> = config
        .models
        .iter()
        .map(|(id, route)| {
            let refs = policies_referencing_model(&config, id);
            build_model_config(id, route, refs)
        })
        .collect();

    Json(serde_json::json!({ "models": models }))
}

/// GET /api/admin/models/:id
pub async fn get_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let config = state.config.read().await;
    match config.models.get(&id) {
        Some(route) => {
            let refs = policies_referencing_model(&config, &id);
            Json(build_model_config(&id, route, refs)).into_response()
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

/// POST /api/admin/models
pub async fn create_model(
    State(state): State<AppState>,
    Json(req): Json<CreateModelRequest>,
) -> Response {
    let mut config = state.config.write().await;

    if config.models.contains_key(&req.id) {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "already_exists",
                "message": format!("Model '{}' already exists", req.id)
            })),
        )
            .into_response();
    }

    let upstream_type = match req.upstream_type.as_str() {
        "openai" => UpstreamType::Openai,
        "anthropic" => UpstreamType::Anthropic,
        other => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_type",
                    "message": format!("upstream_type 必须是 openai / anthropic，收到: {}", other)
                })),
            )
                .into_response();
        }
    };

    let status = match req.status.as_deref() {
        Some("active") | None => ModelStatus::Active,
        Some("rate_limited") => ModelStatus::RateLimited,
        Some("disabled") => ModelStatus::Disabled,
        Some(other) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_status",
                    "message": format!("status 必须是 active / rate_limited / disabled，收到: {}", other)
                })),
            )
                .into_response();
        }
    };

    let route = ModelRoute {
        upstream_type,
        base_url: req.base_url,
        api_key: req.api_key,
        model: req.model,
        metadata: Some(ModelMetadata {
            name: req.name,
            status,
            price_per_input: req.price_per_input,
            price_per_output: req.price_per_output,
            ..Default::default()
        }),
        ..Default::default()
    };

    // 持久化到 SQLite
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.upsert_model(&req.id, &route) {
            eprintln!("⚠️  持久化 model 到数据库失败: {}", e);
        }
    }

    config.models.insert(req.id.clone(), route.clone());

    let resp = build_model_config(&req.id, &route, vec![]);
    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "model": resp
        })),
    )
        .into_response()
}

/// PUT /api/admin/models/:id
///
/// 所有字段（包括上游连接字段）都热生效，持久化到 SQLite。
pub async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateModelConfigRequest>,
) -> Response {
    // 解析 status
    let new_status = match req.status.as_deref() {
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
    let new_upstream_type = match req.upstream_type.as_deref() {
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

    let mut config = state.config.write().await;

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

    if let Some(route) = config.models.get_mut(&id) {
        // --- 上游连接字段 ---
        if let Some(ut) = new_upstream_type {
            route.upstream_type = ut;
        }
        if let Some(ref b) = req.base_url {
            let trimmed = b.trim();
            if !trimmed.is_empty() {
                route.base_url = trimmed.to_string();
            }
        }
        if let Some(ref m) = req.model {
            if !m.is_empty() {
                route.model = m.clone();
            }
        }
        if let Some(ref k) = req.api_key {
            if !k.is_empty() && k != "********" {
                route.api_key = k.clone();
            }
        }

        // --- 元数据字段 ---
        if let Some(n) = req.name.clone() {
            match &mut route.metadata {
                Some(m) => m.name = Some(n),
                None => {
                    route.metadata = Some(ModelMetadata {
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
                    route.metadata = Some(ModelMetadata {
                        status: s,
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(p) = req.price_per_input {
            match &mut route.metadata {
                Some(m) => m.price_per_input = Some(p),
                None => {
                    route.metadata = Some(ModelMetadata {
                        price_per_input: Some(p),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(p) = req.price_per_output {
            match &mut route.metadata {
                Some(m) => m.price_per_output = Some(p),
                None => {
                    route.metadata = Some(ModelMetadata {
                        price_per_output: Some(p),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // 持久化完整 model 到 SQLite
    let saved_route = config.models.get(&id).cloned().unwrap();
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.upsert_model(&id, &saved_route) {
            eprintln!("⚠️  持久化 model 到数据库失败: {}", e);
        }
    }

    let refs = policies_referencing_model(&config, &id);
    let updated = build_model_config(&id, &saved_route, refs);

    Json(serde_json::json!({
        "success": true,
        "message": "已保存",
        "model_id": id,
        "model": updated
    }))
    .into_response()
}

/// DELETE /api/admin/models/:id
pub async fn delete_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let mut config = state.config.write().await;

    // 检查是否被 fallback_policy 引用
    let referenced_by = policies_referencing_model(&config, &id);
    if !referenced_by.is_empty() {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "in_use",
                "message": format!(
                    "Model '{}' 仍被 fallback 策略引用: {:?}",
                    id, referenced_by
                )
            })),
        )
            .into_response();
    }

    if config.models.remove(&id).is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        )
            .into_response();
    }

    // 从 SQLite 删除
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.delete_model(&id) {
            eprintln!("⚠️  从数据库删除 model 失败: {}", e);
        }
    }

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": true, "message": "Model deleted" })),
    )
        .into_response()
}

/// GET /api/admin/fallback/policies-mini（前端 model 编辑 modal 用）
pub async fn list_fallback_policies_mini(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    let policies: Vec<serde_json::Value> = config
        .fallback_policies
        .keys()
        .map(|id| serde_json::json!({ "id": id }))
        .collect();
    Json(serde_json::json!({ "policies": policies }))
}
