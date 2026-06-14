mod admin;
mod config;
mod handlers;
mod providers;
mod router;
mod thaw;
mod types;
mod usage;
mod usage_db;

use admin::apikeys::ApiKeyQuotaTracker;
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
    /// 运行期 quota tracker（不写盘）
    pub api_key_quota: Arc<Mutex<ApiKeyQuotaTracker>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    println!("╔══════════════════════════════════════╗");
    println!("║         Pstep Gateway v{}         ║", env!("CARGO_PKG_VERSION"));
    println!("╚══════════════════════════════════════╝");

    let config = load_config();

    // 打开 SQLite usage_db（如果配置了的话）。None = 走纯内存路径（向后兼容）。
    let usage_db: Option<Arc<usage_db::UsageDb>> = config
        .usage_db
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let db = Arc::new(
                usage_db::UsageDb::open(p).expect("无法打开 usage_db"),
            );
            db.migrate().expect("无法初始化 usage_db schema");
            println!("💾 usage_db 已就绪: {}", p);
            db
        });

    let thaw_tracker = if let Some(thaw_config) = &config.thaw {
        Some(Arc::new(thaw::ThawTracker::new(thaw_config.clone())))
    } else {
        None
    };

    let gateway_router = GatewayRouter::new(config.clone(), thaw_tracker.clone(), usage_db.clone());

    // ApiKeyQuotaTracker：若 DB 可用，挂上 DB 并从 DB 还原历史 quota
    let mut quota_tracker = ApiKeyQuotaTracker::default();
    if let Some(db) = &usage_db {
        quota_tracker.set_db(db.clone());
        quota_tracker.seed_from_db();
    }
    let api_key_quota = Arc::new(Mutex::new(quota_tracker));

    let state = AppState {
        config: Arc::new(Mutex::new(config.clone())),
        router: Arc::new(gateway_router),
        thaw_tracker,
        api_key_quota,
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
        .route(
            "/api/admin/usage/distribution",
            axum::routing::get(admin_usage::usage_distribution),
        )
        .route(
            "/api/admin/models",
            axum::routing::get(admin::models::list_models),
        )
        .route(
            "/api/admin/models/fallback-policies",
            axum::routing::get(admin::models::list_fallback_policies_mini),
        )
        .route(
            "/api/admin/models/{id}",
            axum::routing::get(admin::models::get_model),
        )
        .route(
            "/api/admin/models/{id}",
            axum::routing::put(admin::models::update_model),
        )
        .route(
            "/api/admin/keys",
            axum::routing::get(admin::apikeys::list_keys),
        )
        .route(
            "/api/admin/keys",
            axum::routing::post(admin::apikeys::create_key),
        )
        .route(
            "/api/admin/keys/{id}",
            axum::routing::put(admin::apikeys::update_key),
        )
        .route(
            "/api/admin/keys/{id}",
            axum::routing::delete(admin::apikeys::delete_key),
        )
        .route(
            "/api/admin/fallback/policies",
            axum::routing::get(admin::fallback::list_policies),
        )
        .route(
            "/api/admin/fallback/policies",
            axum::routing::post(admin::fallback::create_policy),
        )
        .route(
            "/api/admin/fallback/policies/{id}",
            axum::routing::get(admin::fallback::get_policy),
        )
        .route(
            "/api/admin/fallback/policies/{id}",
            axum::routing::put(admin::fallback::update_policy),
        )
        .route(
            "/api/admin/fallback/policies/{id}",
            axum::routing::delete(admin::fallback::delete_policy),
        )
        .layer(cors)
        .with_state(state);

    let port = config.port;
    let addr = format!("0.0.0.0:{}", port);

    println!("✅ 网关已启动: http://{}:{}", "0.0.0.0", port);
    let models: Vec<_> = config.models.keys().map(|s| s.as_str()).collect();
    println!("📋 已配置模型: {}", models.join(", "));
    println!("🔒 API Key 校验: 已启用（基于 config.client_api_keys）");
    println!(
        "📊 用量统计: {}",
        if config.usage_tracking.enabled {
            "已启用"
        } else {
            "已禁用"
        }
    );

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
