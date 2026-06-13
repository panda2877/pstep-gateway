use crate::config::save_config;
use crate::types::{ModelConfig, UpdateModelConfigRequest};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

/// GET /api/admin/models
pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock().unwrap();
    let models: Vec<ModelConfig> = config
        .models
        .iter()
        .map(|(id, route)| {
            let metadata = route.metadata.as_ref();
            let name = metadata
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| id.clone());

            ModelConfig {
                id: id.clone(),
                name,
                provider: route.upstream.clone(),
                version: "2024-01-01".to_string(),
                status: "active".to_string(),
                timeout_secs: 30,
                price_per_input: metadata.and_then(|m| m.price_per_input),
                price_per_output: metadata.and_then(|m| m.price_per_output),
                upstream: route.upstream.clone(),
                fallback_chain: route.fallback_chain.clone(),
            }
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
            let metadata = route.metadata.as_ref();
            let name = metadata
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| id.clone());

            Json(ModelConfig {
                id: id.clone(),
                name,
                provider: route.upstream.clone(),
                version: "2024-01-01".to_string(),
                status: "active".to_string(),
                timeout_secs: 30,
                price_per_input: metadata.and_then(|m| m.price_per_input),
                price_per_output: metadata.and_then(|m| m.price_per_output),
                upstream: route.upstream.clone(),
                fallback_chain: route.fallback_chain.clone(),
            }).into_response()
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
        if let Some(timeout) = timeout_secs {
            route.fallback = Some(timeout.to_string());
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

    Json(serde_json::json!({
        "success": true,
        "message": "Model configuration updated",
        "model_id": id,
        "changes": {
            "name": req.name,
            "timeout_secs": timeout_secs,
            "price_per_input": price_input,
            "price_per_output": price_output,
            "status": status
        }
    })).into_response()
}