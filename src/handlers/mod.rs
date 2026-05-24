pub mod v1;

use crate::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, serde::Deserialize)]
pub struct FormatQuery {
    pub format: Option<String>,
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Json(serde_json::json!({
        "status": "ok",
        "version": "0.1.1",
        "models": state.config.models.keys().collect::<Vec<_>>(),
        "uptime": uptime
    }))
}

pub async fn stats(State(state): State<AppState>) -> impl IntoResponse {
    let tracker = state.router.get_usage_tracker();
    Json(tracker.get_stats())
}

pub async fn stats_recent(State(state): State<AppState>) -> impl IntoResponse {
    let tracker = state.router.get_usage_tracker();
    Json(tracker.get_recent(50))
}

pub async fn api_models(State(state): State<AppState>) -> impl IntoResponse {
    let base_url = state.config.public_url.clone()
        .unwrap_or_else(|| format!("http://localhost:{}/v1", state.config.port));

    let models: Vec<serde_json::Value> = state.config.models.iter()
        .map(|(id, route)| {
            let meta = route.metadata.as_ref();
            serde_json::json!({
                "id": id,
                "name": meta.and_then(|m| m.name.clone()).unwrap_or_else(|| id.clone()),
                "api": "openai-completions",
                "provider": "pstep-gateway",
                "baseUrl": base_url,
                "reasoning": meta.map(|m| m.reasoning).unwrap_or(false),
                "input": meta.map(|m| m.input.clone()).unwrap_or_else(|| vec!["text".to_string()]),
                "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
                "contextWindow": meta.and_then(|m| m.context_window).unwrap_or(128000),
                "maxTokens": meta.and_then(|m| m.max_tokens).unwrap_or(4096),
            })
        })
        .collect();

    Json(serde_json::json!({
        "models": models,
        "apiKey": "pstep-gateway-key"
    }))
}