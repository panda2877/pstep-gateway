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

    let models: Vec<_> = state.config.lock().unwrap().models.keys().cloned().collect();

    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "models": models,
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
    let config = state.config.lock().unwrap();
    let base_url = config.public_url.clone()
        .unwrap_or_else(|| format!("http://localhost:{}/v1", config.port));

    let models: Vec<serde_json::Value> = config.models.iter()
        .map(|(id, route)| {
            let meta = route.metadata.as_ref();
            serde_json::json!({
                "id": id,
                "name": meta.and_then(|m| m.name.clone()).unwrap_or_else(|| id.clone()),
                "api": "openai-completions",
                "provider": "pstep-gateway",
                "baseUrl": base_url,
                "status": meta.map(|m| m.status.as_str()).unwrap_or("active"),
                "cost": {
                    "input": meta.and_then(|m| m.price_per_input).unwrap_or(0.0),
                    "output": meta.and_then(|m| m.price_per_output).unwrap_or(0.0),
                    "cacheRead": 0,
                    "cacheWrite": 0
                },
                "contextWindow": 128000,
            })
        })
        .collect();

    // 选一把 client_api_key 作为对外 apiKey；空则用占位
    let first_key = config
        .client_api_keys
        .values()
        .next()
        .map(|k| k.key.clone())
        .unwrap_or_else(|| String::new());

    Json(serde_json::json!({
        "models": models,
        "apiKey": first_key
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