use crate::config::save_config;
use crate::types::{
    ApiKey, ClientApiKeyConfig, CreateApiKeyRequest, CreateApiKeyResponse, UpdateApiKeyRequest,
};
use crate::usage_db::UsageDb;
use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 运行期 quota_used：key_id → 累计 token。
/// 启动时若 `usage_db` 已配置则从 DB 还原；`record()` 时同步写库。
#[derive(Default)]
pub struct ApiKeyQuotaTracker {
    used: Mutex<std::collections::HashMap<String, u64>>,
    db: Option<Arc<UsageDb>>,
}

impl ApiKeyQuotaTracker {
    pub fn set_db(&mut self, db: Arc<UsageDb>) {
        self.db = Some(db);
    }

    /// 启动时调用：用 DB 中的累计值还原内存映射。
    pub fn seed_from_db(&self) {
        let Some(db) = &self.db else { return };
        match db.load_all_quotas() {
            Ok(map) => {
                let mut m = self.used.lock().unwrap();
                for (k, v) in map {
                    m.insert(k, v);
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "usage_db",
                    error = %e,
                    "quota_usage 启动还原失败；以空状态启动"
                );
            }
        }
    }

    pub fn record(&self, key_id: &str, tokens: u64) {
        let new_total = {
            let mut m = self.used.lock().unwrap();
            let entry = m.entry(key_id.to_string()).or_insert(0);
            *entry += tokens;
            *entry
        };
        if let Some(db) = &self.db {
            if let Err(e) = db.upsert_quota(key_id, new_total) {
                tracing::warn!(
                    target: "usage_db",
                    error = %e,
                    key_id,
                    "quota_usage 写库失败"
                );
            }
        }
    }

    pub fn get(&self, key_id: &str) -> u64 {
        self.used.lock().unwrap().get(key_id).copied().unwrap_or(0)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
            let idx = ((seed >> (i * 4)) as usize + i * 17) % chars.len();
            chars[idx]
        })
        .collect()
}

fn mask_key(key: &str) -> (String, String) {
    let prefix: String = key.chars().take(15).collect();
    let rest = "*".repeat(key.chars().count().saturating_sub(15).max(15));
    (prefix.clone(), format!("{} {}", prefix, rest))
}

/// 把持久化的 `ClientApiKeyConfig` 转换为响应给前端的 `ApiKey`。
fn build_api_key(
    id: &str,
    cfg: &ClientApiKeyConfig,
    used: u64,
) -> ApiKey {
    let (key_prefix, key_masked) = mask_key(&cfg.key);
    let quota_percent = if cfg.quota_limit > 0 {
        (used as f32 / cfg.quota_limit as f32) * 100.0
    } else {
        0.0
    };
    ApiKey {
        id: id.to_string(),
        name: cfg.name.clone(),
        key_prefix,
        key_masked,
        model_permissions: cfg.model_permissions.clone(),
        fallback_policy: cfg.fallback_policy.clone(),
        quota_limit: cfg.quota_limit,
        quota_used: used,
        quota_percent,
        created_at: cfg.created_at,
    }
}

/// GET /api/admin/keys
pub async fn list_keys(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock().unwrap();
    let quota = state.api_key_quota.lock().unwrap();
    let keys: Vec<ApiKey> = config
        .client_api_keys
        .iter()
        .map(|(id, cfg)| build_api_key(id, cfg, quota.get(id)))
        .collect();
    Json(serde_json::json!({ "keys": keys }))
}

/// POST /api/admin/keys
pub async fn create_key(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Response {
    let mut config = state.config.lock().unwrap();

    // 生成 id：用 name 简单 slug 化 + 后缀，避免冲突
    let base_id = req
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>();
    let id = if config.client_api_keys.contains_key(&base_id) {
        format!("{}_{}", base_id, random_suffix().chars().take(4).collect::<String>())
    } else {
        base_id
    };

    let raw_key = format!("sk-gw-{}-{}", random_suffix(), random_suffix());
    let cfg = ClientApiKeyConfig {
        name: req.name,
        key: raw_key.clone(),
        model_permissions: req.model_permissions,
        fallback_policy: req.fallback_policy,
        quota_limit: req.quota_limit,
        created_at: now_secs(),
    };

    config.client_api_keys.insert(id.clone(), cfg);
    if let Err(e) = save_config(&config) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "save_failed",
                "message": e
            })),
        )
            .into_response();
    }

    let resp = build_api_key(&id, config.client_api_keys.get(&id).unwrap(), 0);
    let response = CreateApiKeyResponse {
        key: resp,
        raw_key,
    };
    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "key": response.key,
            "raw_key": response.raw_key
        })),
    )
        .into_response()
}

/// PUT /api/admin/keys/:id
/// 注：不允许修改 `key`（明文），需要改 key 就 delete+recreate。
pub async fn update_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Response {
    let mut config = state.config.lock().unwrap();

    let Some(cfg) = config.client_api_keys.get_mut(&id) else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Key '{}' not found", id)
            })),
        )
            .into_response();
    };

    if let Some(name) = req.name {
        cfg.name = name;
    }
    if let Some(perms) = req.model_permissions {
        cfg.model_permissions = perms;
    }
    // fallback_policy: Option<Option<String>> —— 外层 Some 表示「要更新」；
    // 内层 None 表示「置空」；内层 Some 表示「设置新值」
    if let Some(fb) = req.fallback_policy {
        cfg.fallback_policy = fb;
    }
    if let Some(quota) = req.quota_limit {
        cfg.quota_limit = quota;
    }

    // 先复制出需要的字段（放下 cfg 借用）
    let saved_cfg = cfg.clone();
    if let Err(e) = save_config(&config) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "save_failed",
                "message": e
            })),
        )
            .into_response();
    }

    let quota = state.api_key_quota.lock().unwrap();
    let updated = build_api_key(&id, &saved_cfg, quota.get(&id));
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "key": updated
        })),
    )
        .into_response()
}

/// DELETE /api/admin/keys/:id
pub async fn delete_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let mut config = state.config.lock().unwrap();
    if config.client_api_keys.remove(&id).is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "message": format!("Key '{}' not found", id)
            })),
        )
            .into_response();
    }
    if let Err(e) = save_config(&config) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "save_failed",
                "message": e
            })),
        )
            .into_response();
    }
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": true, "message": "Key deleted" })),
    )
        .into_response()
}
