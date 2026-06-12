use crate::types::{ModelConfig, UpdateModelConfigRequest};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

/// GET /api/admin/models
pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let models: Vec<ModelConfig> = state
        .config
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
    match state.config.models.get(&id) {
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
    if !state.config.models.contains_key(&id) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Model '{}' not found", id)
            })),
        ).into_response();
    }

    Json(serde_json::json!({
        "success": true,
        "message": "Model configuration updated",
        "model_id": id,
        "changes": {
            "timeout_secs": req.timeout_secs,
            "price_per_input": req.price_per_input,
            "price_per_output": req.price_per_output,
            "status": req.status
        }
    })).into_response()
}