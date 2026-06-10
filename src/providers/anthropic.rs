use crate::providers::OutputFormat;
use crate::types::{
    AnthropicRequest, AnthropicContent, AnthropicMessage, AnthropicContentBlock,
    AnthropicTool, AnthropicToolChoice, TokenUsage,
    UpstreamConfig, ChatCompletionsRequest, ContentValue, Message as OaiMessage,
    AnthropicMessagesRequest, AnthropicSystem,
    Tool as OaiTool, ToolChoice, ContentPart, FunctionDef,
};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_anthropic_to_openai() {
        let body = r#"{
          "model": "minimax",
          "max_tokens": 50,
          "tools": [{"name": "search", "input_schema": {"type":"object"}}],
          "messages": [
            {"role": "user", "content": "query"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_D", "name": "search", "input": {"q":"x"}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_D", "content": "data"}]}
          ]
        }"#;
        let result = anthropic_request_to_openai_json(body, "MiniMax-M2.7").unwrap();
        eprintln!("=== Case A: tool_result-only user ===\n{}\n=== end ===", result);

        let body2 = r#"{
          "model": "minimax",
          "messages": [
            {"role": "user", "content": [
              {"type": "text", "text": "Please call"},
              {"type": "tool_result", "tool_use_id": "toolu_D", "content": "data"}
            ]}
          ]
        }"#;
        let result2 = anthropic_request_to_openai_json(body2, "MiniMax-M2.7").unwrap();
        eprintln!("=== Case B: text + tool_result ===\n{}\n=== end ===", result2);
        panic!("intentional");
    }
}

// ============================================================================
// openai -> anthropic  (request translation, response based on downstream format)
// ============================================================================

pub async fn proxy_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
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
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    // Response translation: if downstream is openai, translate anthropic -> openai
    match downstream_format {
        OutputFormat::Anthropic => {
            let usage = if body_str.contains("event:") || body_str.contains("data:") {
                TokenUsage::from_anthropic_sse(&body_str)
            } else {
                TokenUsage::from_anthropic_response(&body_str)
            };
            Ok((body_str.to_string(), usage))
        }
        OutputFormat::OpenAI => {
            let usage = if body_str.contains("event:") || body_str.contains("data:") {
                TokenUsage::from_anthropic_sse(&body_str)
            } else {
                TokenUsage::from_anthropic_response(&body_str)
            };
            let converted = crate::providers::anthropic::anthropic_response_to_openai(&body_str)?;
            Ok((converted, usage))
        }
    }
}

pub async fn proxy_non_stream_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
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
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (non-stream) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    match downstream_format {
        OutputFormat::Anthropic => {
            let usage = TokenUsage::from_anthropic_response(&body_str);
            Ok((body_str.to_string(), usage))
        }
        OutputFormat::OpenAI => {
            let usage = TokenUsage::from_anthropic_response(&body_str);
            let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
            let resp = convert_anthropic_json_to_openai(&json)?;
            serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
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
) -> Result<(String, TokenUsage), String> {
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
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (passthrough) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }
    let usage = if body_str.contains("event:") || body_str.contains("data:") {
        TokenUsage::from_anthropic_sse(&body_str)
    } else {
        TokenUsage::from_anthropic_response(&body_str)
    };
    Ok((body_str.to_string(), usage))
}

pub async fn proxy_non_stream_anthropic_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<(String, TokenUsage), String> {
    proxy_anthropic_to_anthropic(upstream, target_model, body).await
}

// ============================================================================
// Conversions
// ============================================================================

/// Truncate `s` to at most `max` characters, appending a marker if cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{}…(truncated, total {} chars)", head, s.chars().count())
    }
}

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
            // OpenAI uses role="tool" with tool_call_id for tool results.
            // Anthropic expects a "user" message with content block type="tool_result".
            if m.role == "tool" {
                let tool_use_id = m.tool_call_id.clone().unwrap_or_default();
                let result_text = match &m.content {
                    ContentValue::String(s) => s.clone(),
                    ContentValue::Array(parts) => parts.iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                };
                let block = AnthropicContentBlock {
                    block_type: "tool_result".to_string(),
                    text: None,
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: Some(tool_use_id),
                    content: Some(json!(result_text)),
                };
                return AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Array(vec![block]),
                };
            }
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            let mut blocks: Vec<AnthropicContentBlock> = Vec::new();
            // assistant: prior tool_calls -> tool_use blocks
            if m.role == "assistant" {
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let name = tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        // arguments is a string per openai; parse to JSON for anthropic
                        let input = tc.get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or(json!({}));
                        blocks.push(AnthropicContentBlock {
                            block_type: "tool_use".to_string(),
                            text: None,
                            id: Some(id),
                            name: Some(name),
                            input: Some(input),
                            tool_use_id: None,
                            content: None,
                        });
                    }
                }
            }
            // text/other content
            match &m.content {
                ContentValue::String(s) if !s.is_empty() => {
                    if !blocks.is_empty() {
                        // mixed text + tool_use: emit as array
                        blocks.insert(0, AnthropicContentBlock {
                            block_type: "text".to_string(),
                            text: Some(s.clone()),
                            id: None, name: None, input: None,
                            tool_use_id: None, content: None,
                        });
                    } else {
                        // text-only assistant or user string
                    }
                }
                ContentValue::String(s) => {
                    // empty string; no-op
                    let _ = s;
                }
                ContentValue::Array(arr) => {
                    for part in arr {
                        let mut obj = serde_json::Map::new();
                        obj.insert("type".to_string(), json!(part.part_type));
                        if let Some(text) = &part.text {
                            obj.insert("text".to_string(), json!(text));
                        }
                        if let Ok(b) = serde_json::from_value::<AnthropicContentBlock>(Value::Object(obj)) {
                            blocks.push(b);
                        }
                    }
                }
            }
            let content = if blocks.is_empty() {
                // fall back to plain string content for empty text cases
                match &m.content {
                    ContentValue::String(s) => AnthropicContent::String(s.clone()),
                    _ => AnthropicContent::String(String::new()),
                }
            } else {
                AnthropicContent::Array(blocks)
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
                tool_calls: None,
            });
        }
    }

    for m in &req.messages {
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut tool_result_messages: Vec<OaiMessage> = Vec::new();
        let mut text_parts: Vec<ContentPart> = Vec::new();
        // Track whether the source message contained anything other than
        // tool_result blocks. If only tool_result blocks are present, the
        // source message itself was a synthetic carrier — we must not emit a
        // duplicate empty user message that confuses upstream tool-call
        // validators ("tool call result does not follow tool call").
        let mut has_non_tool_result = false;

        let content = match &m.content {
            Value::String(s) => {
                has_non_tool_result = true;
                ContentValue::String(s.clone())
            }
            Value::Array(arr) => {
                for v in arr {
                    let part_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match part_type {
                        "text" => {
                            has_non_tool_result = true;
                            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(ContentPart {
                                    part_type: "text".to_string(),
                                    text: Some(text.to_string()),
                                    image_url: None,
                                });
                            }
                        }
                        "tool_use" => {
                            has_non_tool_result = true;
                            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let input = v.get("input").cloned().unwrap_or(json!({}));
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                                }
                            }));
                        }
                        "tool_result" => {
                            // tool_result blocks are extracted into role="tool"
                            // messages; they do NOT contribute to the source
                            // message's content.
                            let tool_use_id = v.get("tool_use_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let result_text = v.get("content").map(|c| c.to_string()).unwrap_or_default();
                            tool_result_messages.push(OaiMessage {
                                role: "tool".to_string(),
                                content: ContentValue::String(result_text),
                                name: None,
                                tool_call_id: Some(tool_use_id),
                                tool_calls: None,
                            });
                        }
                        // skip thinking, image, etc.
                        _ => {}
                    }
                }
                if text_parts.is_empty() {
                    ContentValue::String(String::new())
                } else if text_parts.len() == 1 {
                    ContentValue::String(text_parts[0].text.clone().unwrap_or_default())
                } else {
                    ContentValue::Array(text_parts)
                }
            }
            _ => {
                has_non_tool_result = true;
                ContentValue::String(m.content.to_string())
            }
        };

        // Only emit the source message if it carried something other than
        // tool_result blocks. Otherwise the tool_result extraction below is
        // the only meaningful translation of this source message.
        if has_non_tool_result {
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            messages.push(OaiMessage {
                role: role.to_string(),
                content,
                name: None,
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
            });
        }
        // tool_result blocks become role="tool" messages
        for tr in tool_result_messages {
            messages.push(tr);
        }
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

    let blocks = anthropic_resp.get("content")
        .and_then(|v| v.as_array())
        .map(|v| v.clone())
        .unwrap_or_default();

    let mut content_text = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let block_type = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    content_text.push_str(t);
                }
            }
            "tool_use" => {
                let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let input = b.get("input").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "index": i,
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                    }
                }));
            }
            // ignore "thinking" and other block types
            _ => {}
        }
    }

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
    let finish_reason = if !tool_calls.is_empty() {
        "tool_calls"
    } else {
        match stop_reason {
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            _ => "stop",
        }
    };

    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), json!("assistant"));
    message.insert("content".to_string(), json!(content_text));
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), json!(tool_calls));
    }

    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
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
    // Tracks tool_use blocks by index so we can emit delta.tool_calls with
    // matching `index` values, matching OpenAI's streaming protocol.
    let mut out = String::new();
    let mut id = String::from("chatcmpl-unknown");
    let mut model = String::new();
    // tool_use block metadata keyed by anthropic content_block index
    let mut tool_blocks: std::collections::HashMap<u64, (String, String)> = std::collections::HashMap::new();
    // ordered list of tool call indices seen, to assign openai tool_calls index
    let mut tool_order: Vec<u64> = Vec::new();
    let mut final_stop_reason: Option<String> = None;

    let now = || std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for line in sse.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if line == "event: message_start" {
            continue;
        }
        if line == "event: content_block_stop" {
            continue;
        }
        if line == "event: message_stop" {
            let finish = if !tool_order.is_empty() {
                "tool_calls"
            } else {
                match final_stop_reason.as_deref() {
                    Some("max_tokens") => "length",
                    Some("tool_use") => "tool_calls",
                    _ => "stop",
                }
            };
            let chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": now(),
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish
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

                match v.get("type").and_then(|x| x.as_str()) {
                    Some("content_block_start") => {
                        let block = v.get("content_block");
                        let block_type = block.and_then(|b| b.get("type")).and_then(|x| x.as_str()).unwrap_or("");
                        if block_type == "tool_use" {
                            let block_index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0);
                            let tool_id = block.and_then(|b| b.get("id")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let tool_name = block.and_then(|b| b.get("name")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                            tool_blocks.insert(block_index, (tool_id, tool_name));
                            if !tool_order.contains(&block_index) {
                                tool_order.push(block_index);
                            }
                            let oa_index = tool_order.iter().position(|i| *i == block_index).unwrap_or(0);
                            let (tid, tname) = tool_blocks.get(&block_index).cloned().unwrap_or_default();
                            let chunk = json!({
                                "id": id,
                                "object": "chat.completion.chunk",
                                "created": now(),
                                "model": model,
                                "choices": [{
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [{
                                            "index": oa_index,
                                            "id": tid,
                                            "type": "function",
                                            "function": {"name": tname, "arguments": ""}
                                        }]
                                    },
                                    "finish_reason": null
                                }]
                            });
                            out.push_str(&format!("data: {}\n\n", chunk));
                        }
                    }
                    Some("content_block_delta") => {
                        let block_index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0);
                        if tool_blocks.contains_key(&block_index) {
                            // tool_use argument delta
                            if let Some(partial) = v.get("delta").and_then(|d| d.get("partial_json")).and_then(|x| x.as_str()) {
                                let oa_index = tool_order.iter().position(|i| *i == block_index).unwrap_or(0);
                                let chunk = json!({
                                    "id": id,
                                    "object": "chat.completion.chunk",
                                    "created": now(),
                                    "model": model,
                                    "choices": [{
                                        "index": 0,
                                        "delta": {
                                            "tool_calls": [{
                                                "index": oa_index,
                                                "function": {"arguments": partial}
                                            }]
                                        },
                                        "finish_reason": null
                                    }]
                                });
                                out.push_str(&format!("data: {}\n\n", chunk));
                            }
                        } else if let Some(text) = v.get("delta").and_then(|d| d.get("text")).and_then(|x| x.as_str()) {
                            let chunk = json!({
                                "id": id,
                                "object": "chat.completion.chunk",
                                "created": now(),
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
                    Some("message_delta") => {
                        if let Some(sr) = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(|x| x.as_str()) {
                            final_stop_reason = Some(sr.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(out)
}
