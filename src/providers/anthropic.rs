use crate::providers::OutputFormat;
use crate::types::{
    AnthropicRequest, AnthropicContent, AnthropicMessage, AnthropicContentBlock,
    AnthropicTool, AnthropicToolChoice,
    UpstreamConfig, ChatCompletionsRequest, ContentValue, Message as OaiMessage,
    AnthropicMessagesRequest, AnthropicSystem,
    Tool as OaiTool, ToolChoice, ContentPart, FunctionDef,
};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

// ============================================================================
// openai -> anthropic  (request translation, response based on downstream format)
// ============================================================================

pub async fn proxy_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<String, String> {
    let client = build_client()?;
    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = openai_to_anthropic_request(&openai_req, target_model)?;
    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&anthropic_req).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    // Response translation: if downstream is openai, translate anthropic -> openai
    match downstream_format {
        OutputFormat::Anthropic => Ok(body_str.to_string()),
        OutputFormat::OpenAI => crate::providers::anthropic::anthropic_response_to_openai(&body_str),
    }
}

pub async fn proxy_non_stream_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<String, String> {
    let client = build_client()?;
    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = openai_to_anthropic_request(&openai_req, target_model)?;
    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&anthropic_req).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    match downstream_format {
        OutputFormat::Anthropic => Ok(body_str.to_string()),
        OutputFormat::OpenAI => {
            let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
            let resp = convert_anthropic_json_to_openai(&json)?;
            serde_json::to_string(&resp).map_err(|e| e.to_string())
        }
    }
}

// ============================================================================
// anthropic -> anthropic  (passthrough with model rename)
// ============================================================================

pub async fn proxy_anthropic_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<String, String> {
    let client = build_client()?;
    let mut req_json: Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    // Replace model name with the configured upstream model
    if let Some(obj) = req_json.as_object_mut() {
        obj.insert("model".to_string(), json!(target_model));
    }

    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&req_json).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }
    Ok(body_str.to_string())
}

pub async fn proxy_non_stream_anthropic_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<String, String> {
    proxy_anthropic_to_anthropic(upstream, target_model, body).await
}

// ============================================================================
// Conversions
// ============================================================================

fn build_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

fn openai_to_anthropic_request(
    openai_req: &ChatCompletionsRequest,
    target_model: &str,
) -> Result<AnthropicRequest, String> {
    let system_msg = openai_req.messages.iter().find(|m| m.role == "system");
    let system = system_msg.and_then(|m| {
        if let ContentValue::String(s) = &m.content {
            Some(s.clone())
        } else {
            None
        }
    });

    let messages: Vec<AnthropicMessage> = openai_req.messages.iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            let content = match &m.content {
                ContentValue::String(s) => AnthropicContent::String(s.clone()),
                ContentValue::Array(arr) => {
                    let converted: Vec<AnthropicContentBlock> = arr.iter().filter_map(|part| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("type".to_string(), json!(part.part_type));
                        if let Some(text) = &part.text {
                            obj.insert("text".to_string(), json!(text));
                        }
                        serde_json::from_value(Value::Object(obj)).ok()
                    }).collect();
                    AnthropicContent::Array(converted)
                }
            };
            AnthropicMessage { role: role.to_string(), content }
        })
        .collect();

    let max_tokens = openai_req.max_tokens.unwrap_or(4096);

    let tools = openai_req.tools.as_ref().map(|tools| {
        tools.iter().filter_map(|t| {
            let func = t.function.as_ref()?;
            Some(AnthropicTool {
                tool_type: "function".to_string(),
                name: func.name.clone(),
                description: func.description.clone(),
                input_schema: func.parameters.clone(),
            })
        }).collect()
    });

    let tool_choice = openai_req.tool_choice.as_ref().and_then(|tc| {
        match tc {
            ToolChoice::String(s) => {
                match s.as_str() {
                    "auto" => Some(AnthropicToolChoice { choice_type: "auto".to_string(), name: None }),
                    "none" | "required" => Some(AnthropicToolChoice { choice_type: "any".to_string(), name: None }),
                    _ => None,
                }
            }
            ToolChoice::Object(obj) => {
                Some(AnthropicToolChoice {
                    choice_type: "tool".to_string(),
                    name: obj.function.as_ref().map(|f| f.name.clone()),
                })
            }
        }
    });

    Ok(AnthropicRequest {
        model: target_model.to_string(),
        messages,
        system,
        max_tokens,
        stream: openai_req.stream,
        temperature: openai_req.temperature,
        top_p: openai_req.top_p,
        tools,
        tool_choice,
        stop_sequences: None,
    })
}

// ============================================================================
// anthropic -> openai  (used by openai::proxy_anthropic_to_openai)
// ============================================================================

/// Convert an anthropic-format request body to an openai-format request body (JSON).
pub fn anthropic_request_to_openai_json(
    body: &str,
    target_model: &str,
) -> Result<String, String> {
    let req: AnthropicMessagesRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let mut messages: Vec<OaiMessage> = Vec::new();

    if let Some(sys) = req.system {
        let sys_text = match sys {
            AnthropicSystem::String(s) => s,
            AnthropicSystem::Array(arr) => arr.iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        if !sys_text.is_empty() {
            messages.push(OaiMessage {
                role: "system".to_string(),
                content: ContentValue::String(sys_text),
                name: None,
                tool_call_id: None,
            });
        }
    }

    for m in &req.messages {
        let role = if m.role == "assistant" { "assistant" } else { "user" };
        let content = match &m.content {
            Value::String(s) => ContentValue::String(s.clone()),
            Value::Array(arr) => {
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
        messages.push(OaiMessage {
            role: role.to_string(),
            content,
            name: None,
            tool_call_id: None,
        });
    }

    let openai_tools = req.tools.as_ref().map(|tools| {
        tools.iter().filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let description = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let input_schema = t.get("input_schema").cloned().unwrap_or(json!({}));
            Some(OaiTool {
                tool_type: Some("function".to_string()),
                function: Some(FunctionDef {
                    name: name.to_string(),
                    description: Some(description.to_string()),
                    parameters: input_schema,
                }),
            })
        }).collect()
    });

    let chat_req = ChatCompletionsRequest {
        model: target_model.to_string(),
        messages,
        stream: req.stream.unwrap_or(false),
        max_tokens: req.max_tokens,
        temperature: None,
        top_p: None,
        tools: openai_tools,
        tool_choice: None,
        stop: None,
    };

    serde_json::to_string(&chat_req).map_err(|e| e.to_string())
}

/// Convert an anthropic-format response (JSON or SSE chunk) to openai format.
pub fn anthropic_response_to_openai(anthropic_body: &str) -> Result<String, String> {
    // SSE path
    if anthropic_body.contains("event:") || anthropic_body.contains("data:") {
        return convert_anthropic_sse_to_openai(anthropic_body);
    }
    // JSON path
    let json: Value = serde_json::from_str(anthropic_body)
        .map_err(|e| format!("解析上游响应失败: {}", e))?;
    let openai_resp = convert_anthropic_json_to_openai(&json)?;
    serde_json::to_string(&openai_resp).map_err(|e| e.to_string())
}

fn convert_anthropic_json_to_openai(anthropic_resp: &Value) -> Result<Value, String> {
    let id = anthropic_resp.get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let model = anthropic_resp.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let content = anthropic_resp.get("content")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let input_tokens = anthropic_resp.get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = anthropic_resp.get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let stop_reason = anthropic_resp.get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };

    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    }))
}

fn convert_anthropic_sse_to_openai(sse: &str) -> Result<String, String> {
    // Translate each anthropic SSE event to the equivalent openai chunk.
    // We do a best-effort translation; the goal is to keep clients working
    // when calling /v1/chat/completions against an anthropic upstream.
    let mut out = String::new();
    let mut id = String::from("chatcmpl-unknown");
    let mut model = String::new();

    for line in sse.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if line == "event: message_start" {
            // next data line will carry the message
            continue;
        }
        if line == "event: content_block_start" {
            continue;
        }
        if line == "event: content_block_stop" {
            continue;
        }
        if line == "event: message_stop" {
            let chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]
            });
            out.push_str(&format!("data: {}\n\n", chunk));
            out.push_str("data: [DONE]\n\n");
            continue;
        }
        if line.starts_with("data: ") {
            let data = &line[6..];
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(mid) = v.get("message").and_then(|m| m.get("id")).and_then(|x| x.as_str()) {
                    id = mid.to_string();
                }
                if let Some(m) = v.get("message").and_then(|m| m.get("model")).and_then(|x| x.as_str()) {
                    model = m.to_string();
                }
                if v.get("type").and_then(|x| x.as_str()) == Some("content_block_delta") {
                    if let Some(text) = v.get("delta").and_then(|d| d.get("text")).and_then(|x| x.as_str()) {
                        let chunk = json!({
                            "id": id,
                            "object": "chat.completion.chunk",
                            "created": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "delta": {"content": text},
                                "finish_reason": null
                            }]
                        });
                        out.push_str(&format!("data: {}\n\n", chunk));
                    }
                }
                if v.get("type").and_then(|x| x.as_str()) == Some("message_delta") {
                    // stop_reason may be present
                }
            }
        }
    }
    Ok(out)
}
