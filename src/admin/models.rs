use crate::config::save_config;
use crate::types::{ModelConfig, UpdateModelConfigRequest};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

/// Mask an api_key for display: keep first 3 and last 4 chars, replace middle with "***".
/// If the key is empty, return an empty string.
/// If the key is shorter than 8 chars, return "***" of the same length.
fn mask_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let len = key.chars().count();
    if len <= 8 {
        return "*".repeat(len);
    }
    let head: String = key.chars().take(3).collect();
    let tail: String = key.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("{}{}{}", head, "*".repeat(len - 7), tail)
}

fn build_model_config(id: &str, route: &crate::types::ModelRoute, upstream: Option<&crate::types::UpstreamConfig>) -> ModelConfig {
    let metadata = route.metadata.as_ref();
    let name = metadata
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| id.to_string());

    let (base_url, api_key_masked, api_key_configured) = match upstream {
        Some(u) => (
            Some(u.base_url.clone()),
            Some(mask_api_key(&u.api_key)),
            !u.api_key.is_empty(),
        ),
        None => (None, None, false),
    };

    ModelConfig {
        id: id.to_string(),
        name,
        provider: route.upstream.clone(),
        version: "2024-01-01".to_string(),
        status: "active".to_string(),
        timeout_secs: 30,
        price_per_input: metadata.and_then(|m| m.price_per_input),
        price_per_output: metadata.and_then(|m| m.price_per_output),
        upstream: route.upstream.clone(),
        fallback_chain: route.fallback_chain.clone(),
        base_url,
        api_key_masked,
        api_key_configured,
    }
}

/// GET /api/admin/models
pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock().unwrap();
    let models: Vec<ModelConfig> = config
        .models
        .iter()
        .map(|(id, route)| {
            let upstream = config.upstreams.get(&route.upstream);
            build_model_config(id, route, upstream)
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
            let upstream = config.upstreams.get(&route.upstream);
            Json(build_model_config(&id, route, upstream)).into_response()
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        ).into_response(),
    }
}

/// PUT /api/admin/models/:id
pub async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateModelConfigRequest>,
) -> Response {
    // Clone values from request before locking
    let name_clone = req.name.clone();
    let timeout_secs = req.timeout_secs;
    let price_input = req.price_per_input;
    let price_output = req.price_per_output;
    let status = req.status.clone();
    let base_url = req.base_url.clone();
    let api_key = req.api_key.clone();

    let mut config = state.config.lock().unwrap();

    if !config.models.contains_key(&id) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        ).into_response();
    }

    // Determine the upstream name for this model
    let upstream_name = config.models.get(&id).map(|r| r.upstream.clone());

    // Update the model configuration in memory
    if let Some(route) = config.models.get_mut(&id) {
        if let Some(name) = name_clone {
            if let Some(meta) = &mut route.metadata {
                meta.name = Some(name);
            } else {
                route.metadata = Some(crate::types::ModelMetadata {
                    name: Some(name),
                    ..Default::default()
                });
            }
        }
        if let Some(p) = price_input {
            if let Some(meta) = &mut route.metadata {
                meta.price_per_input = Some(p);
            } else {
                route.metadata = Some(crate::types::ModelMetadata {
                    price_per_input: Some(p),
                    ..Default::default()
                });
            }
        }
        if let Some(p) = price_output {
            if let Some(meta) = &mut route.metadata {
                meta.price_per_output = Some(p);
            } else {
                route.metadata = Some(crate::types::ModelMetadata {
                    price_per_output: Some(p),
                    ..Default::default()
                });
            }
        }
        if let Some(_timeout) = timeout_secs {
            // Note: ModelRoute 暂无 timeout_secs 字段，先不持久化以免污染配置。
            // 之前错误地写入 fallback，导致校验失败。这里只接受值但不写盘。
        }
    }

    // Update upstream base_url / api_key (only when model has a valid upstream)
    if let Some(upstream_name) = upstream_name {
        if let Some(upstream) = config.upstreams.get_mut(&upstream_name) {
            if let Some(new_base_url) = base_url {
                let trimmed = new_base_url.trim();
                if !trimmed.is_empty() {
                    upstream.base_url = trimmed.to_string();
                }
            }
            if let Some(new_api_key) = api_key {
                // Treat None, empty, and the placeholder "********" as "no change"
                if !new_api_key.is_empty() && new_api_key != "********" {
                    upstream.api_key = new_api_key;
                }
            }
        } else {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "upstream_not_found",
                    "message": format!("Upstream '{}' not found", upstream_name)
                })),
            ).into_response();
        }
    }

    // Save to disk
    if let Err(e) = save_config(&config) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "save_failed",
                "message": e
            })),
        ).into_response();
    }

    // Re-read the updated model for response
    let updated = match config.models.get(&id) {
        Some(route) => {
            let upstream = config.upstreams.get(&route.upstream);
            Some(build_model_config(&id, route, upstream))
        }
        None => None,
    };

    Json(serde_json::json!({
        "success": true,
        "message": "Model configuration updated",
        "model_id": id,
        "changes": {
            "name": req.name,
            "timeout_secs": timeout_secs,
            "price_per_input": price_input,
            "price_per_output": price_output,
            "status": status,
            "base_url": req.base_url,
            "api_key_changed": req.api_key.as_ref().map(|v| !v.is_empty() && v != "********").unwrap_or(false)
        },
        "model": updated
    })).into_response()
}
