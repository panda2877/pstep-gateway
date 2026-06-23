use crate::types::{CreateFallbackPolicyRequest, FallbackPolicy, UpdateFallbackPolicyRequest};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// GET /api/admin/fallback/policies
pub async fn list_policies(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let policies: Vec<FallbackPolicy> = config
        .fallback_policies
        .iter()
        .map(|(id, p)| FallbackPolicy {
            id: id.clone(),
            name: id.clone(),
            description: p.description.clone().unwrap_or_default(),
            enabled: p.enabled,
            chain: p.chain.clone(),
            created_at: 0,
        })
        .collect();
    Json(serde_json::json!({ "policies": policies }))
}

/// POST /api/admin/fallback/policies
pub async fn create_policy(
    State(state): State<AppState>,
    Json(req): Json<CreateFallbackPolicyRequest>,
) -> Response {
    let mut config = state.config.write().await;

    if config.fallback_policies.contains_key(&req.id) {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "already_exists",
                "message": format!("Policy '{}' already exists", req.id)
            })),
        )
            .into_response();
    }

    let policy = crate::types::FallbackPolicyConfig {
        description: if req.description.is_empty() { None } else { Some(req.description) },
        enabled: req.enabled,
        chain: req.chain,
    };
    config.fallback_policies.insert(req.id.clone(), policy);

    // 持久化到数据库
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.upsert_fallback_policy(&req.id, config.fallback_policies.get(&req.id).unwrap()) {
            eprintln!("⚠️  持久化 fallback_policy 到数据库失败: {}", e);
            // 内存配置已更新，热重载仍有效；重启后此修改会丢失
        }
    }

    let resp_policy = {
        let id = req.id;
        FallbackPolicy {
            id: id.clone(),
            name: id.clone(),
            description: config
                .fallback_policies
                .get(&id)
                .and_then(|p| p.description.clone())
                .unwrap_or_default(),
            enabled: config
                .fallback_policies
                .get(&id)
                .map(|p| p.enabled)
                .unwrap_or(true),
            chain: config
                .fallback_policies
                .get(&id)
                .map(|p| p.chain.clone())
                .unwrap_or_default(),
            created_at: now_secs(),
        }
    };
    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "policy": resp_policy
        })),
    )
        .into_response()
}

/// GET /api/admin/fallback/policies/:id
pub async fn get_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let config = state.config.read().await;
    match config.fallback_policies.get(&id) {
        Some(p) => {
            let resp = FallbackPolicy {
                id: id.clone(),
                name: id,
                description: p.description.clone().unwrap_or_default(),
                enabled: p.enabled,
                chain: p.chain.clone(),
                created_at: 0,
            };
            (axum::http::StatusCode::OK, Json(serde_json::json!(resp))).into_response()
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        )
            .into_response(),
    }
}

/// PUT /api/admin/fallback/policies/:id
pub async fn update_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateFallbackPolicyRequest>,
) -> Response {
    let mut config = state.config.write().await;

    let Some(policy) = config.fallback_policies.get_mut(&id) else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        )
            .into_response();
    };

    if let Some(description) = req.description {
        policy.description = if description.is_empty() { None } else { Some(description) };
    }
    if let Some(enabled) = req.enabled {
        policy.enabled = enabled;
    }
    if let Some(chain) = req.chain {
        policy.chain = chain;
    }

    // 持久化到数据库
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.upsert_fallback_policy(&id, config.fallback_policies.get(&id).unwrap()) {
            eprintln!("⚠️  持久化 fallback_policy 到数据库失败: {}", e);
            // 内存配置已更新，热重载仍有效；重启后此修改会丢失
        }
    }

    let p = config.fallback_policies.get(&id).cloned().unwrap();
    let resp = FallbackPolicy {
        id: id.clone(),
        name: id,
        description: p.description.unwrap_or_default(),
        enabled: p.enabled,
        chain: p.chain,
        created_at: 0,
    };
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "policy": resp
        })),
    )
        .into_response()
}

/// DELETE /api/admin/fallback/policies/:id
pub async fn delete_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let mut config = state.config.write().await;

    // v0.3: model 不再引用 fallback_policy；引用关系由 policy.chain 反向表达。
    // 删除前只需检查：1) 有没有 client_api_key 引用；2) 这个 policy 自身。
    let referenced_by_keys: Vec<String> = config
        .client_api_keys
        .iter()
        .filter(|(_, k)| k.fallback_policy.as_deref() == Some(&id))
        .map(|(k, _)| k.clone())
        .collect();

    if !referenced_by_keys.is_empty() {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "in_use",
                "message": format!(
                    "Policy '{}' 仍被引用：keys={:?}",
                    id, referenced_by_keys
                )
            })),
        )
            .into_response();
    }

    if config.fallback_policies.remove(&id).is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        )
            .into_response();
    }

    // 从数据库删除
    if let Some(db) = &state.usage_db {
        if let Err(e) = db.delete_fallback_policy(&id) {
            eprintln!("⚠️  从数据库删除 fallback_policy 失败: {}", e);
            // 内存配置已更新，热重载仍有效；重启后此修改会丢失
        }
    }

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": true, "message": "Policy deleted" })),
    )
        .into_response()
}
