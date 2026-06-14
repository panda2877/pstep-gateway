use crate::providers::OutputFormat;
use crate::types::{
    AnthropicMessagesMessage, AnthropicMessagesRequest, AnthropicSystem, ChatCompletionsRequest,
    ContentPart, ContentValue, Message, Tool,
};
use crate::AppState;
use crate::handlers::FormatQuery;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
    Json,
};
use std::time::Duration;

/// 鉴权后的 API Key 元数据，注入到请求上下文供 router 使用。
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub key_id: String,
    pub fallback_policy: Option<String>,
}

/// 鉴权：从 `config.client_api_keys` 查询 bearer key。
fn require_auth(
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> Result<AuthContext, Response> {
    let auth = match headers.get("authorization").and_then(|v| v.to_str().ok()) {
        Some(a) => a,
        None => return Err(unauthorized("Missing Authorization header")),
    };

    let token = auth.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() {
        return Err(unauthorized("Missing bearer token"));
    }

    let config = state.config.lock().unwrap();
    for (id, key) in config.client_api_keys.iter() {
        if key.key == token {
            return Ok(AuthContext {
                key_id: id.clone(),
                fallback_policy: key.fallback_policy.clone(),
            });
        }
    }
    Err(unauthorized("Invalid API key"))
}

fn unauthorized(msg: &str) -> Response {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "Unauthorized",
            "message": msg
        })),
    )
        .into_response()
}

/// Truncate `s` to at most `max` characters, appending a marker if cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{}…(truncated, total {} chars)", head, s.chars().count())
    }
}

// Provider base URL mapping - 仅作为「无对应 model」时的兜底
const PROVIDER_BASE_URLS: &[(&str, &str, &str)] = &[
    ("openai", "https://api.openai.com/v1", "bearer"),
    ("anthropic", "https://api.anthropic.com/v1", "x-api-key"),
    ("deepseek", "https://api.deepseek.com/v1", "bearer"),
];

fn get_provider_default(provider: &str) -> Option<(&'static str, &'static str)> {
    PROVIDER_BASE_URLS
        .iter()
        .find(|(p, _, _)| *p == provider)
        .map(|(_, url, auth)| (*url, *auth))
}

async fn provider_proxy(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Response {
    if let Err(resp) = require_auth(&headers, &state) {
        return resp;
    }

    let (provider, actual_path) = match path.split_once('/') {
        Some((p, rest)) => (p.to_string(), rest.to_string()),
        None => (path.clone(), String::new()),
    };

    // 兼容 v0.1：/provider/{name}/... 仍走 upstreams。新结构里没有顶层 upstreams，
    // 这里改为：在 models 里查同 id 的 model，用其 4 字段；如果没有就走硬编码默认。
    let (base_url, api_key, auth) = {
        let config = state.config.lock().unwrap();
        if let Some(route) = config.models.get(&provider) {
            let auth = route.upstream_type.auth_header();
            (
                route.base_url.clone(),
                route.api_key.clone(),
                auth.to_string(),
            )
        } else if let Some((url, a)) = get_provider_default(&provider) {
            (url.to_string(), String::new(), a.to_string())
        } else {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "bad_request",
                    "message": format!("Unknown provider: {}", provider)
                })),
            )
                .into_response();
        }
    };

    let target_url = if actual_path.is_empty() {
        if provider == "anthropic" {
            format!("{}/v1/messages", base_url.trim_end_matches('/'))
        } else {
            base_url.clone()
        }
    } else {
        let path = actual_path.trim_start_matches('/');
        if base_url.ends_with("/v1") && (path == "v1" || path.starts_with("v1/")) {
            let stripped = path.strip_prefix("v1").unwrap_or(path).trim_start_matches('/');
            if stripped.is_empty() {
                base_url.clone()
            } else {
                format!("{}/{}", base_url.trim_end_matches('/'), stripped)
            }
        } else {
            format!("{}/{}", base_url.trim_end_matches('/'), path)
        }
    };

    let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "bad_request",
                    "message": format!("Failed to read request body: {}", e)
                })),
            )
                .into_response()
        }
    };

    println!("🔄 [{}] -> {}", provider, target_url);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    let mut request = client.post(&target_url);
    request = request.header("Content-Type", "application/json");

    match auth.as_str() {
        "x-api-key" => {
            request = request.header("x-api-key", api_key);
            if provider == "anthropic" {
                request = request.header("anthropic-version", "2023-06-01");
            }
        }
        _ => {
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {}", api_key));
            }
        }
    }

    if let Some(req_id) = headers.get("x-request-id") {
        request = request.header("x-request-id", req_id);
    }

    let result = request.body(body_bytes).send().await;

    match result {
        Ok(resp) => {
            let status = resp.status();
            let body_str = match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    return (
                        axum::http::StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": "bad_gateway",
                            "message": format!("Failed to read response: {}", e)
                        })),
                    )
                        .into_response();
                }
            };

            if !status.is_success() {
                tracing::error!(
                    target: "upstream",
                    upstream = "provider_proxy",
                    provider = %provider,
                    target_url = %target_url,
                    status = status.as_u16(),
                    body = %truncate(&body_str, 2048),
                    "provider_proxy upstream returned non-success"
                );
            }

            let mut response =
                axum::response::Response::new(axum::body::Body::from(body_str.clone()));
            response.headers_mut().insert(
                "Content-Type",
                if body_str.contains("event:") || body_str.contains("data:") {
                    "text/event-stream".parse().unwrap()
                } else {
                    "application/json".parse().unwrap()
                },
            );
            response
                .headers_mut()
                .insert("Cache-Control", "no-cache".parse().unwrap());
            *response.status_mut() = status;
            response
        }
        Err(e) => {
            tracing::error!(
                target: "upstream",
                upstream = "provider_proxy",
                provider = %provider,
                target_url = %target_url,
                error = %e,
                "provider_proxy request failed"
            );
            (
                axum::http::StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "bad_gateway",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

pub fn v1_routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/chat/completions", post(chat_completions))
        .route("/messages", post(chat_completions_anthropic))
        .route("/models", get(list_models))
}

pub fn provider_routes() -> axum::Router<AppState> {
    use axum::routing::post;
    axum::Router::new().route("/{*path}", post(provider_proxy))
}

async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let models: Vec<_> = state
        .config
        .lock()
        .unwrap()
        .models
        .iter()
        .map(|(id, route)| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "created": now,
                "owned_by": route.upstream_type.as_str()
            })
        })
        .collect();

    Json(serde_json::json!({
        "object": "list",
        "data": models
    }))
}

async fn chat_completions(
    State(state): State<AppState>,
    Query(query): Query<FormatQuery>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ChatCompletionsRequest>,
) -> Response {
    let auth = match require_auth(&headers, &state) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let format = match query.format.as_deref() {
        Some("anthropic") => OutputFormat::Anthropic,
        _ => OutputFormat::OpenAI,
    };

    let model_name = body.model.clone();
    let body_str = serde_json::to_string(&body).unwrap();

    let result = state
        .router
        .route(&model_name, &body_str, format, auth.fallback_policy.as_deref())
        .await;

    match result {
        Ok(stream) => {
            let mut resp = axum::response::Response::new(axum::body::Body::from(stream));
            resp.headers_mut()
                .insert("Content-Type", "text/event-stream".parse().unwrap());
            resp.headers_mut()
                .insert("Cache-Control", "no-cache".parse().unwrap());
            resp.headers_mut()
                .insert("X-Accel-Buffering", "no".parse().unwrap());
            if state.router.did_failover().await {
                resp.headers_mut()
                    .insert("X-Pstep-Failover", "true".parse().unwrap());
            }
            resp
        }
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "bad_gateway",
                "message": e.to_string()
            })),
        )
            .into_response(),
    }
}

async fn chat_completions_anthropic(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AnthropicMessagesRequest>,
) -> Response {
    let auth = match require_auth(&headers, &state) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let model_name = body.model.clone();
    let is_stream = body.stream == Some(true);
    let body_str = serde_json::to_string(&body).unwrap();

    if is_stream {
        let result = state
            .router
            .route(
                &model_name,
                &body_str,
                OutputFormat::Anthropic,
                auth.fallback_policy.as_deref(),
            )
            .await;

        match result {
            Ok(stream) => {
                let mut resp = axum::response::Response::new(axum::body::Body::from(stream));
                resp.headers_mut()
                    .insert("Content-Type", "text/event-stream".parse().unwrap());
                resp.headers_mut()
                    .insert("Cache-Control", "no-cache".parse().unwrap());
                resp.headers_mut()
                    .insert("X-Accel-Buffering", "no".parse().unwrap());
                if state.router.did_failover().await {
                    resp.headers_mut()
                        .insert("X-Pstep-Failover", "true".parse().unwrap());
                }
                resp
            }
            Err(e) => (
                axum::http::StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "bad_gateway",
                    "message": e.to_string()
                })),
            )
                .into_response(),
        }
    } else {
        let result = state
            .router
            .route_non_stream(
                &model_name,
                &body_str,
                OutputFormat::Anthropic,
                auth.fallback_policy.as_deref(),
            )
            .await;

        match result {
            Ok(response) => {
                let mut resp = axum::response::Response::new(axum::body::Body::from(response));
                resp.headers_mut()
                    .insert("Content-Type", "application/json".parse().unwrap());
                if state.router.did_failover().await {
                    resp.headers_mut()
                        .insert("X-Pstep-Failover", "true".parse().unwrap());
                }
                resp
            }
            Err(e) => (
                axum::http::StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "bad_gateway",
                    "message": e.to_string()
                })),
            )
                .into_response(),
        }
    }
}

fn convert_anthropic_system_to_openai(system: &Option<AnthropicSystem>) -> Vec<Message> {
    let mut msgs = Vec::new();
    if let Some(sys) = system {
        let sys_text = match sys {
            AnthropicSystem::String(s) => s.clone(),
            AnthropicSystem::Array(arr) => arr
                .iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        if !sys_text.is_empty() {
            msgs.push(Message {
                role: "system".to_string(),
                content: ContentValue::String(sys_text),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }
    msgs
}

fn convert_anthropic_messages_to_openai(messages: &[AnthropicMessagesMessage]) -> Vec<Message> {
    messages
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            let content = match &m.content {
                serde_json::Value::String(s) => ContentValue::String(s.clone()),
                serde_json::Value::Array(arr) => {
                    let parts: Vec<ContentPart> = arr
                        .iter()
                        .filter_map(|v| {
                            let part_type = v.get("type")?.as_str()?;
                            let text = v
                                .get("text")
                                .and_then(|t| t.as_str())
                                .map(String::from);
                            Some(ContentPart {
                                part_type: part_type.to_string(),
                                text,
                                image_url: None,
                            })
                        })
                        .collect();
                    ContentValue::Array(parts)
                }
                _ => ContentValue::String(m.content.to_string()),
            };
            Message {
                role: role.to_string(),
                content,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }
        })
        .collect()
}

fn convert_anthropic_tools_to_openai(tools: &[serde_json::Value]) -> Option<Vec<Tool>> {
    let result: Vec<Tool> = tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let description = t
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let input_schema = t
                .get("input_schema")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            Some(Tool {
                tool_type: Some("function".to_string()),
                function: Some(crate::types::FunctionDef {
                    name: name.to_string(),
                    description: Some(description.to_string()),
                    parameters: input_schema,
                }),
            })
        })
        .collect();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
