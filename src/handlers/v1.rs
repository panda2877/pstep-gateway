use crate::providers::OutputFormat;
use crate::types::{ChatCompletionsRequest, AnthropicMessagesRequest, AnthropicMessagesMessage, AnthropicSystem, Message, ContentValue, ContentPart, Tool};
use crate::AppState;
use crate::handlers::FormatQuery;
use axum::{
    extract::Query,
    extract::State,
    response::{IntoResponse, Response},
    Json,
};

const VALID_API_KEY: &str = "pstep-gateway-key";

fn require_auth(headers: &axum::http::HeaderMap) -> Option<Response> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    if auth == format!("Bearer {}", VALID_API_KEY) {
        None
    } else {
        Some((axum::http::StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Unauthorized",
            "message": "Missing or invalid API key"
        }))).into_response())
    }
}

pub fn v1_routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/chat/completions", post(chat_completions))
        .route("/models", get(list_models))
}

pub fn anthropic_routes() -> axum::Router<AppState> {
    use axum::routing::post;

    axum::Router::new()
        .route("/messages", post(chat_completions_anthropic))
}

async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let models: Vec<_> = state.config.models.iter()
        .map(|(id, route)| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "created": now,
                "owned_by": route.upstream
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
    if let Some(resp) = require_auth(&headers) {
        return resp;
    }

    let format = match query.format.as_deref() {
        Some("anthropic") => OutputFormat::Anthropic,
        _ => OutputFormat::OpenAI,
    };

    let model_name = body.model.clone();
    let body_str = serde_json::to_string(&body).unwrap();

    let result = state.router.route(&model_name, &body_str, format).await;

    match result {
        Ok(stream) => {
            let mut resp = axum::response::Response::new(axum::body::Body::from(stream));
            resp.headers_mut().insert(
                "Content-Type",
                "text/event-stream".parse().unwrap(),
            );
            resp.headers_mut().insert("Cache-Control", "no-cache".parse().unwrap());
            resp.headers_mut().insert("X-Accel-Buffering", "no".parse().unwrap());
            if state.router.did_failover().await {
                resp.headers_mut().insert("X-Pstep-Failover", "true".parse().unwrap());
            }
            resp
        }
        Err(e) => {
            (axum::http::StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                "error": "bad_gateway",
                "message": e.to_string()
            }))).into_response()
        }
    }
}

async fn chat_completions_anthropic(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AnthropicMessagesRequest>,
) -> Response {
    if let Some(resp) = require_auth(&headers) {
        return resp;
    }

    let format = OutputFormat::Anthropic;
    let model_name = body.model.clone();
    let is_stream = body.stream == Some(true);

    // Convert Anthropic request to OpenAI format for internal processing
    let openai_messages = convert_anthropic_messages_to_openai(&body.messages);
    let openai_system = convert_anthropic_system_to_openai(&body.system);
    let openai_tools = body.tools.as_ref().and_then(|t| convert_anthropic_tools_to_openai(t));

    let mut messages = openai_system;
    messages.extend(openai_messages);

    let chat_request = ChatCompletionsRequest {
        model: model_name.clone(),
        messages,
        stream: false, // Always non-streaming internally
        max_tokens: body.max_tokens,
        temperature: None,
        top_p: None,
        tools: openai_tools,
        tool_choice: None,
        stop: None,
    };

    let body_str = serde_json::to_string(&chat_request).unwrap();

    if is_stream {
        let result = state.router.route(&model_name, &body_str, format).await;

        match result {
            Ok(stream) => {
                let mut resp = axum::response::Response::new(axum::body::Body::from(stream));
                resp.headers_mut().insert(
                    "Content-Type",
                    "text/event-stream".parse().unwrap(),
                );
                resp.headers_mut().insert("Cache-Control", "no-cache".parse().unwrap());
                resp.headers_mut().insert("X-Accel-Buffering", "no".parse().unwrap());
                if state.router.did_failover().await {
                    resp.headers_mut().insert("X-Pstep-Failover", "true".parse().unwrap());
                }
                resp
            }
            Err(e) => {
                (axum::http::StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                    "error": "bad_gateway",
                    "message": e.to_string()
                }))).into_response()
            }
        }
    } else {
        let result = state.router.route_non_stream(&model_name, &body_str, format).await;

        match result {
            Ok(response) => {
                let mut resp = axum::response::Response::new(axum::body::Body::from(response));
                resp.headers_mut().insert(
                    "Content-Type",
                    "application/json".parse().unwrap(),
                );
                if state.router.did_failover().await {
                    resp.headers_mut().insert("X-Pstep-Failover", "true".parse().unwrap());
                }
                resp
            }
            Err(e) => {
                (axum::http::StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                    "error": "bad_gateway",
                    "message": e.to_string()
                }))).into_response()
            }
        }
    }
}

fn convert_anthropic_system_to_openai(system: &Option<AnthropicSystem>) -> Vec<Message> {
    let mut msgs = Vec::new();
    if let Some(sys) = system {
        let sys_text = match sys {
            AnthropicSystem::String(s) => s.clone(),
            AnthropicSystem::Array(arr) => arr.iter()
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
            });
        }
    }
    msgs
}

fn convert_anthropic_messages_to_openai(messages: &[AnthropicMessagesMessage]) -> Vec<Message> {
    messages.iter().map(|m| {
        let role = if m.role == "assistant" { "assistant" } else { "user" };
        let content = match &m.content {
            serde_json::Value::String(s) => ContentValue::String(s.clone()),
            serde_json::Value::Array(arr) => {
                let parts: Vec<ContentPart> = arr.iter().filter_map(|v| {
                    let part_type = v.get("type")?.as_str()?;
                    let text = v.get("text").and_then(|t| t.as_str()).map(String::from);
                    Some(ContentPart {
                        part_type: part_type.to_string(),
                        text,
                        image_url: None,
                    })
                }).collect();
                ContentValue::Array(parts)
            }
            _ => ContentValue::String(m.content.to_string()),
        };
        Message {
            role: role.to_string(),
            content,
            name: None,
            tool_call_id: None,
        }
    }).collect()
}

fn convert_anthropic_tools_to_openai(tools: &[serde_json::Value]) -> Option<Vec<Tool>> {
    let result: Vec<Tool> = tools.iter().filter_map(|t| {
        let name = t.get("name")?.as_str()?;
        let description = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
        let input_schema = t.get("input_schema").cloned().unwrap_or(serde_json::json!({}));

        Some(Tool {
            tool_type: Some("function".to_string()),
            function: Some(crate::types::FunctionDef {
                name: name.to_string(),
                description: Some(description.to_string()),
                parameters: input_schema,
            }),
        })
    }).collect();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}