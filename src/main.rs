mod admin;
mod config;
mod handlers;
mod providers;
mod router;
mod thaw;
mod types;
mod usage;

use admin::apikeys::ApiKeyStore;
use admin::fallback::FallbackPolicyStore;
use admin::usage as admin_usage;
use axum::Router;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber;

use crate::config::load_config;
use crate::router::Router as GatewayRouter;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Mutex<types::GatewayConfig>>,
    pub router: Arc<GatewayRouter>,
    pub thaw_tracker: Option<Arc<thaw::ThawTracker>>,
    pub api_key_store: Arc<ApiKeyStore>,
    pub fallback_policy_store: Arc<FallbackPolicyStore>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    println!("╔══════════════════════════════════════╗");
    println!("║         Pstep Gateway v{}         ║", env!("CARGO_PKG_VERSION"));
    println!("╚══════════════════════════════════════╝");

    let config = load_config();

    // Initialize thaw tracker if configured
    let thaw_tracker = if let Some(thaw_config) = &config.thaw {
        Some(Arc::new(thaw::ThawTracker::new(thaw_config.clone())))
    } else {
        None
    };

    let gateway_router = GatewayRouter::new(config.clone(), thaw_tracker.clone());

    let state = AppState {
        config: Arc::new(Mutex::new(config.clone())),
        router: Arc::new(gateway_router),
        thaw_tracker,
        api_key_store: Arc::new(ApiKeyStore::new()),
        fallback_policy_store: Arc::new(FallbackPolicyStore::new()),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .nest("/v1", handlers::v1::v1_routes())
        .nest("/provider", handlers::v1::provider_routes())
        .route("/health", axum::routing::get(handlers::health))
        .route("/stats", axum::routing::get(handlers::stats))
        .route("/stats/recent", axum::routing::get(handlers::stats_recent))
        .route("/api/models", axum::routing::get(handlers::api_models))
        .route("/api/health", axum::routing::get(handlers::health_status))
        // Admin API routes
        .route("/api/admin/usage/stats", axum::routing::get(admin_usage::usage_stats))
        .route("/api/admin/usage/distribution", axum::routing::get(admin_usage::usage_distribution))
        .route("/api/admin/models", axum::routing::get(admin::models::list_models))
        .route("/api/admin/models/{id}", axum::routing::get(admin::models::get_model))
        .route("/api/admin/models/{id}", axum::routing::put(admin::models::update_model))
        .route("/api/admin/keys", axum::routing::get(admin::apikeys::list_keys))
        .route("/api/admin/keys", axum::routing::post(admin::apikeys::create_key))
        .route("/api/admin/keys/{id}", axum::routing::delete(admin::apikeys::delete_key))
        .route("/api/admin/fallback/policies", axum::routing::get(admin::fallback::list_policies))
        .route("/api/admin/fallback/policies", axum::routing::post(admin::fallback::create_policy))
        .route("/api/admin/fallback/policies/{id}", axum::routing::get(admin::fallback::get_policy))
        .route("/api/admin/fallback/policies/{id}", axum::routing::put(admin::fallback::update_policy))
        .route("/api/admin/fallback/policies/{id}", axum::routing::delete(admin::fallback::delete_policy))
        .layer(cors)
        .with_state(state);

    let port = config.port;
    let addr = format!("0.0.0.0:{}", port);

    println!("✅ 网关已启动: http://{}:{}", "0.0.0.0", port);
    let models: Vec<_> = config.models.keys().map(|s| s.as_str()).collect();
    println!("📋 已配置模型: {}", models.join(", "));
    println!("🔒 API Key 校验: 已启用");
    println!("📊 用量统计: {}", if config.usage_tracking.enabled { "已启用" } else { "已禁用" });

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}