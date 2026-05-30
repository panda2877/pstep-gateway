mod config;
mod handlers;
mod providers;
mod router;
mod types;
mod usage;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber;

use crate::config::load_config;
use crate::router::Router as GatewayRouter;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<types::GatewayConfig>,
    pub router: Arc<GatewayRouter>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    println!("╔══════════════════════════════════════╗");
    println!("║         Pstep Gateway v0.1.1          ║");
    println!("╚══════════════════════════════════════╝");

    let config = load_config();
    let gateway_router = GatewayRouter::new(config.clone());

    let state = AppState {
        config: Arc::new(config.clone()),
        router: Arc::new(gateway_router),
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