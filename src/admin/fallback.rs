use crate::types::{CreateFallbackPolicyRequest, FallbackPolicy, UpdateFallbackPolicyRequest};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// In-memory fallback policy store
pub struct FallbackPolicyStore {
    policies: RwLock<HashMap<String, FallbackPolicy>>,
}

impl FallbackPolicyStore {
    pub fn new() -> Self {
        Self {
            policies: RwLock::new(HashMap::new()),
        }
    }

    pub fn list(&self) -> Vec<FallbackPolicy> {
        self.policies.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<FallbackPolicy> {
        self.policies.read().unwrap().get(id).cloned()
    }

    pub fn create(&self, req: CreateFallbackPolicyRequest) -> FallbackPolicy {
        let id = uuid_v4();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let policy = FallbackPolicy {
            id: id.clone(),
            name: req.name,
            description: req.description,
            enabled: req.enabled,
            chain: req.chain,
            created_at,
        };

        self.policies.write().unwrap().insert(id, policy.clone());
        policy
    }

    pub fn update(&self, id: &str, req: UpdateFallbackPolicyRequest) -> Option<FallbackPolicy> {
        let mut policies = self.policies.write().unwrap();
        if let Some(policy) = policies.get_mut(id) {
            if let Some(name) = req.name {
                policy.name = name;
            }
            if let Some(description) = req.description {
                policy.description = description;
            }
            if let Some(enabled) = req.enabled {
                policy.enabled = enabled;
            }
            if let Some(chain) = req.chain {
                policy.chain = chain;
            }
            return Some(policy.clone());
        }
        None
    }

    pub fn delete(&self, id: &str) -> bool {
        self.policies.write().unwrap().remove(id).is_some()
    }
}

impl Default for FallbackPolicyStore {
    fn default() -> Self {
        Self::new()
    }
}

fn uuid_v4() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let random: u64 = (now & 0xFFFFFFFFFFFFFFFF) as u64;
    format!(
        "{:016x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        now as u64,
        (random >> 48) as u16,
        (random >> 32) as u16 & 0x0FFF,
        ((random >> 16) as u16 & 0x3FFF) | 0x8000,
        random & 0xFFFFFFFFFFFF
    )
}

/// GET /api/admin/fallback/policies
pub async fn list_policies(State(state): State<AppState>) -> impl IntoResponse {
    let policies = state.fallback_policy_store.list();
    Json(serde_json::json!({ "policies": policies }))
}

/// POST /api/admin/fallback/policies
pub async fn create_policy(
    State(state): State<AppState>,
    Json(req): Json<CreateFallbackPolicyRequest>,
) -> impl IntoResponse {
    let policy = state.fallback_policy_store.create(req);
    Json(serde_json::json!({
        "success": true,
        "policy": policy
    }))
}

/// GET /api/admin/fallback/policies/:id
pub async fn get_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.fallback_policy_store.get(&id) {
        Some(policy) => (axum::http::StatusCode::OK, Json(serde_json::json!(policy))),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        ),
    }
}

/// PUT /api/admin/fallback/policies/:id
pub async fn update_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateFallbackPolicyRequest>,
) -> impl IntoResponse {
    match state.fallback_policy_store.update(&id, req) {
        Some(policy) => (axum::http::StatusCode::OK, Json(serde_json::json!({
            "success": true,
            "policy": policy
        }))),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        ),
    }
}

/// DELETE /api/admin/fallback/policies/:id
pub async fn delete_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.fallback_policy_store.delete(&id) {
        (axum::http::StatusCode::OK, Json(serde_json::json!({ "success": true, "message": "Policy deleted" })))
    } else {
        (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Policy '{}' not found", id)
            })),
        )
    }
}