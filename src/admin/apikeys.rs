use crate::types::{ApiKey, CreateApiKeyRequest, CreateApiKeyResponse};
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// In-memory API key store (in production, use a database)
pub struct ApiKeyStore {
    keys: RwLock<HashMap<String, ApiKey>>,
}

impl ApiKeyStore {
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
        }
    }

    pub fn list(&self) -> Vec<ApiKey> {
        self.keys.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<ApiKey> {
        self.keys.read().unwrap().get(id).cloned()
    }

    pub fn create(&self, req: CreateApiKeyRequest) -> CreateApiKeyResponse {
        let id = uuid_v4();
        let raw_key = format!("sk-gw-{}-{}", random_suffix(), random_suffix());
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let key = ApiKey {
            id: id.clone(),
            name: req.name,
            key_prefix: raw_key.chars().take(15).collect(),
            key_masked: format!("{} ************************************", &raw_key[..15]),
            model_permissions: req.model_permissions,
            quota_limit: req.quota_limit,
            quota_used: 0,
            quota_percent: 0.0,
            created_at,
        };

        self.keys.write().unwrap().insert(id, key.clone());

        CreateApiKeyResponse { key, raw_key }
    }

    pub fn delete(&self, id: &str) -> bool {
        self.keys.write().unwrap().remove(id).is_some()
    }

    pub fn update_quota(&self, id: &str, used: u64) {
        if let Some(key) = self.keys.write().unwrap().get_mut(id) {
            key.quota_used = used;
            key.quota_percent = if key.quota_limit > 0 {
                (used as f32 / key.quota_limit as f32) * 100.0
            } else {
                0.0
            };
        }
    }
}

impl Default for ApiKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let random: u64 = (now & 0xFFFFFFFFFFFFFFFF) as u64;
    format!("{:016x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        now as u64,
        (random >> 48) as u16,
        (random >> 32) as u16 & 0x0FFF,
        ((random >> 16) as u16 & 0x3FFF) | 0x8000,
        random & 0xFFFFFFFFFFFF
    )
}

fn random_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
    (0..8)
        .map(|i| {
            let idx = ((seed >> (i * 4)) as usize) % chars.len();
            chars[idx]
        })
        .collect()
}

/// GET /api/admin/keys
pub async fn list_keys(State(state): State<AppState>) -> impl IntoResponse {
    let keys = state.api_key_store.list();
    Json(serde_json::json!({ "keys": keys }))
}

/// POST /api/admin/keys
pub async fn create_key(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    let result = state.api_key_store.create(req);
    Json(serde_json::json!({
        "success": true,
        "key": result.key,
        "raw_key": result.raw_key
    }))
}

/// DELETE /api/admin/keys/:id
pub async fn delete_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.api_key_store.delete(&id) {
        (axum::http::StatusCode::OK, Json(serde_json::json!({ "success": true, "message": "Key deleted" })))
    } else {
        (axum::http::StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": "not_found",
            "message": format!("Key '{}' not found", id)
        })))
    }
}