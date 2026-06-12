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
        "version": env!("CARGO_PKG_VERSION"),
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
                "cost": {
                    "input": meta.and_then(|m| m.price_per_input).unwrap_or(0.0),
                    "output": meta.and_then(|m| m.price_per_output).unwrap_or(0.0),
                    "cacheRead": 0,
                    "cacheWrite": 0
                },
                "contextWindow": meta.and_then(|m| m.context_window).unwrap_or(128000),
            })
        })
        .collect();

    Json(serde_json::json!({
        "models": models,
        "apiKey": "pstep-gateway-key"
    }))
}

pub async fn health_status(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(ref tracker) = state.thaw_tracker {
        let health = tracker.get_all_health().await;
        Json(serde_json::json!({
            "thaw_enabled": true,
            "models": health
        }))
    } else {
        Json(serde_json::json!({
            "thaw_enabled": false,
            "models": {}
        }))
    }
}