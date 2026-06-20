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
use axum::{
    extract::Request,
    response::Response,
    Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{Mutex, RwLock};
use tower::{Layer, Service};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber;

use crate::config::load_config;
use crate::router::Router as GatewayRouter;

// ============================================================================
// InFlight 计数 — 用于优雅排空（graceful drain）
//
// tower middleware 包裹每个 HTTP 请求：进入时 +1，处理后 -1。
// SIGTERM 触发的 drainer 轮询此计数器归 0 后才允许 axum::serve 的 future
// resolve，使得 systemd 重启容器时不会掐断正在跑的 LLM 调用。
//
// 注意：axum/hyper 的"连接"和"逻辑请求"不等价（HTTP/1.1 keep-alive
// 上一连接可能跑多个 request）。把计数加在 middleware 层是按"逻辑请求"
// 计的，对应 Router::route() 的一次调用 —— 也就是真正需要等待的工作。
// ============================================================================

#[derive(Clone)]
pub struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    pub fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
    pub fn load(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct InFlightLayer {
    counter: Arc<AtomicUsize>,
}

impl InFlightLayer {
    pub fn new(counter: Arc<AtomicUsize>) -> Self {
        Self { counter }
    }
}

impl<S> Layer<S> for InFlightLayer {
    type Service = InFlightService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        InFlightService {
            inner,
            counter: self.counter.clone(),
        }
    }
}

pub struct InFlightService<S> {
    inner: S,
    counter: Arc<AtomicUsize>,
}

// axum 0.8 的 `Router::layer` 要求 `Service<Request>: Clone`。
// `inner: S` 已经是 `Clone + Send + 'static`（见 trait bound），加 derive 即可。
impl<S: Clone> Clone for InFlightService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            counter: self.counter.clone(),
        }
    }
}

impl<S> Service<Request> for InFlightService<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send,
{
    type Response = Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        // 计数 +1。健康检查的 GET 不计入（避免 nginx / 负载均衡的预检
        // 把计数器卡住）。这跟 with_graceful_shutdown 的 stop-accepting
        // 互补：后者拒绝新 TCP，前者只关心逻辑请求。
        let is_health = req
            .uri()
            .path()
            .chars()
            .eq("/health".chars())
            && req.method() == axum::http::Method::GET;
        if !is_health {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let counter = self.counter.clone();

        Box::pin(async move {
            let result = inner.call(req).await;
            // 成功/失败都 -1：失败也代表"这个请求结束了"。
            if !is_health {
                counter.fetch_sub(1, Ordering::SeqCst);
            }
            result
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    /// 全局 config（tokio RwLock：handler 读、admin 写；写不频繁但要求不阻塞读）
    pub config: Arc<RwLock<types::GatewayConfig>>,
    pub router: Arc<GatewayRouter>,
    pub thaw_tracker: Option<Arc<thaw::ThawTracker>>,
    /// 运行期 quota tracker（不写盘）。tokio Mutex：hot-path 频繁增减
    pub api_key_quota: Arc<Mutex<ApiKeyQuotaTracker>>,
    /// 在飞请求计数（tower middleware 写入），供 SIGTERM drainer 轮询。
    pub in_flight: InFlight,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // 抓 panic 到 stderr + backtrace + 文件。否则 deadlock 复发时只能看到
    // "futex_wait" 看不到是哪个 .unwrap() 触发的（参见 memory
    // `tokio-worker-futex-wedge`）。distroless 容器 journald 不一定抓到 stderr
    // （v7 已经踩过），写文件 SIGKILL 也还在，事后能 `cat panic.log` 看到。
    let panic_log_path = std::path::PathBuf::from("/var/lib/pstep-gateway/panic.log");
    std::panic::set_hook(Box::new(move |info| {
        let line = format!(
            "\n🔥 PANIC at {}\n   payload: {:?}\n   backtrace:\n{:?}\n   ts_ms: {}\n",
            info.location()
                .map(|l| l.to_string())
                .unwrap_or_else(|| "unknown".into()),
            info.payload(),
            std::backtrace::Backtrace::force_capture(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        );
        eprintln!("{}", line);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log_path)
        {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
    }));

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
        quota_tracker.seed_from_db().await;
    }
    let api_key_quota = Arc::new(Mutex::new(quota_tracker));

    let in_flight = InFlight::new();

    let state = AppState {
        config: Arc::new(RwLock::new(config.clone())),
        router: Arc::new(gateway_router),
        thaw_tracker,
        api_key_quota,
        in_flight: in_flight.clone(),
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
            "/api/admin/keys/{id}/reveal",
            axum::routing::post(admin::apikeys::reveal_key),
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
        .layer(InFlightLayer::new(in_flight.0.clone()))
        .layer(cors)
        .with_state(state);

    // GATEWAY_PORT 环境变量覆盖 config.port，便于 blue/green 双实例各
    // 监听不同端口而共用同一份 config.yaml。
    let port: u16 = std::env::var("GATEWAY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(config.port);
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

    // Graceful shutdown：等 SIGTERM / SIGINT 后，drain 至 in-flight=0 再退。
    //
    // 时序：
    //  1. systemd 发送 SIGTERM（systemctl stop / restart）
    //  2. with_graceful_shutdown 触发，axum 停止 accept 新 TCP 连接
    //  3. drainer 轮询 in_flight 计数器：归 0 → 让 future resolve
    //  4. axum::serve 的 future 退出，主进程返回
    //
    // 必须满足 TimeoutStopSec >= DRAIN_DEADLINE，否则 systemd 会 SIGKILL。
    // 经验值：上游超时 70s + fallback 70s = 140s，handler 硬超时 150s 兜底。
    // quadlet 配 TimeoutStopSec=160s 留 10s 余量给 SQLite 等。
    const DRAIN_DEADLINE_SECS: u64 = 150;
    let counter_for_drain = in_flight.clone();
    let shutdown = async move {
        // 等 SIGTERM（或 SIGINT，方便本地 Ctrl-C 测试）。
        let mut sigterm = signal(SignalKind::terminate())
            .expect("install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt())
            .expect("install SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => tracing::info!("收到 SIGTERM，开始 graceful drain..."),
            _ = sigint.recv()  => tracing::info!("收到 SIGINT，开始 graceful drain..."),
        }

        let deadline = Instant::now() + Duration::from_secs(DRAIN_DEADLINE_SECS);
        loop {
            let n = counter_for_drain.load();
            if n == 0 {
                tracing::info!("drain 完成（in-flight=0），进程退出");
                break;
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    in_flight = n,
                    "drain 超时 {}s，强制退出（systemd 即将 SIGKILL）",
                    DRAIN_DEADLINE_SECS
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };

    // 心跳日志：每 30s 写一条 in_flight / RSS / FD count 到 stderr。
    // 下次 wedge 起来时，journald 里能看到"上一次心跳是 X 秒前"，精确知道
    // wedge 起点，而不只是看 systemd SIGKILL 时间。
    let started_at = std::time::Instant::now();
    let hb_counter = in_flight.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.tick().await; // skip immediate first tick
        loop {
            ticker.tick().await;
            let rss_mb = read_rss_mb().unwrap_or(0);
            let fd_count = read_fd_count().unwrap_or(0);
            tracing::info!(
                target: "heartbeat",
                in_flight = hb_counter.load(),
                rss_mb,
                fd_count,
                uptime_s = started_at.elapsed().as_secs(),
                "💓 heartbeat"
            );
        }
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .unwrap();
}

/// 读 /proc/self/statm 第二列（resident pages） × page_size，返回 MB。
/// 失败返回 None，让调用方记 0。
fn read_rss_mb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    if page_size == 0 {
        return None;
    }
    Some((pages * page_size) / (1024 * 1024))
}

/// 读 /proc/self/fd 目录项数，作为 fd_count 近似值。
fn read_fd_count() -> Option<u64> {
    let c = std::fs::read_dir("/proc/self/fd").ok()?.count();
    Some(c as u64)
}
