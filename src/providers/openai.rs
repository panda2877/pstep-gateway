use crate::providers::OutputFormat;
use crate::types::{TokenUsage, UpstreamConfig};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

// ============================================================================
// openai -> openai  (passthrough with model rename, format conversion on response)
// ============================================================================

/// Downstream is openai format; upstream is openai.
/// We replace the model name with `target_model`, and if downstream requested
/// anthropic format (e.g. via ?format=anthropic on /v1/chat/completions) we
/// convert the response.
pub async fn proxy_openai_to_openai(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
    let client = build_client()?;
    let body_json: Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;
    let modified_body = prepare_openai_body(body_json, target_model);

    let resp = client
        .post(format!("{}/chat/completions", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", upstream.api_key))
        .body(modified_body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "openai",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "openai upstream returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    if downstream_format == OutputFormat::OpenAI {
        // Streaming upstream -> downstream: SSE passthrough
        let usage = TokenUsage::from_openai_sse(&body_str);
        Ok((body_str.to_string(), usage))
    } else {
        // Client asked for anthropic format on the openai endpoint
        if body_str.contains("event:") || body_str.contains("data:") {
            let usage = TokenUsage::from_openai_sse(&body_str);
            crate::providers::openai::convert_sse_to_anthropic_stream(&body_str)
                .map(|s| (s, usage))
        } else {
            let usage = TokenUsage::from_openai_response(&body_str);
            let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
            let resp = convert_to_anthropic_response(&json)?;
            serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
        }
    }
}

pub async fn proxy_non_stream_openai_to_openai(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
    let client = build_client()?;
    let body_json: Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;
    let modified_body = prepare_openai_body(body_json, target_model);

    let resp = client
        .post(format!("{}/chat/completions", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", upstream.api_key))
        .body(modified_body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "openai",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "openai upstream (non-stream) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    if downstream_format == OutputFormat::OpenAI {
        Ok((body_str.to_string(), TokenUsage::from_openai_response(&body_str)))
    } else {
        let usage = TokenUsage::from_openai_response(&body_str);
        let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
        let resp = convert_to_anthropic_response(&json)?;
        serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
    }
}

// ============================================================================
// anthropic -> openai
// ============================================================================

/// Downstream is anthropic format (e.g. POST /v1/messages); upstream is openai.
/// We convert the request body to openai chat-completions, send it, then convert
/// the response back to anthropic format.
pub async fn proxy_anthropic_to_openai(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<(String, TokenUsage), String> {
    let openai_body_raw = crate::providers::anthropic::anthropic_request_to_openai_json(body, target_model)?;
    let openai_body = prepare_openai_body(serde_json::from_str(&openai_body_raw).unwrap_or(json!({})), target_model);
    let client = build_client()?;

    let resp = client
        .post(format!("{}/chat/completions", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", upstream.api_key))
        .body(openai_body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "openai",
            target_model,
            base_url = %upstream.base_url,
            downstream = "anthropic",
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "openai upstream (anthropic->openai) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    // Translate openai response back to anthropic format
    if body_str.contains("event:") || body_str.contains("data:") {
        let usage = TokenUsage::from_openai_sse(&body_str);
        crate::providers::openai::convert_sse_to_anthropic_stream(&body_str)
            .map(|s| (s, usage))
    } else {
        let usage = TokenUsage::from_openai_response(&body_str);
        let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
        let resp = convert_to_anthropic_response(&json)?;
        serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
    }
}

pub async fn proxy_non_stream_anthropic_to_openai(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<(String, TokenUsage), String> {
    let openai_body = crate::providers::anthropic::anthropic_request_to_openai_json(body, target_model)?;
    let client = build_client()?;

    let resp = client
        .post(format!("{}/chat/completions", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", upstream.api_key))
        .body(openai_body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "openai",
            target_model,
            base_url = %upstream.base_url,
            downstream = "anthropic",
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "openai upstream (anthropic->openai, non-stream) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    let usage = TokenUsage::from_openai_response(&body_str);
    let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
    let resp = convert_to_anthropic_response(&json)?;
    serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
}

// ============================================================================
// Helpers
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

fn prepare_openai_body(body_json: Value, target_model: &str) -> String {
    let Some(obj) = body_json.as_object() else {
        return body_json.to_string();
    };
    let mut new_obj = obj.clone();
    new_obj.insert("model".to_string(), json!(target_model));

    // For streaming requests, set stream_options.include_usage so the upstream
    // sends usage data in the final SSE chunk. This enables token tracking for
    // all streaming calls (minimax, mimo, etc.).
    let is_streaming = new_obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    if is_streaming {
        new_obj.insert("stream_options".to_string(), json!({"include_usage": true}));
    }

    // Ensure each tool has a "type" field
    if let Some(tools) = new_obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for tool in tools.iter_mut() {
            if let Some(tool_obj) = tool.as_object_mut() {
                let type_val = tool_obj.get("type");
                let needs_fix = match type_val {
                    Some(Value::String(s)) if !s.is_empty() => false,
                    Some(Value::Null) => true,
                    None => true,
                    _ => true,
                };
                if needs_fix {
                    tool_obj.insert("type".to_string(), json!("function"));
                }
            }
        }
    }

    // Reconstruct tools from history if missing
    let has_tools = new_obj.get("tools").is_some();
    let has_tool_role_message = new_obj.get("messages")
        .and_then(|m| m.as_array())
        .map(|msgs| msgs.iter().any(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool")))
        .unwrap_or(false);

    if !has_tools && has_tool_role_message {
        if let Some(messages) = new_obj.get("messages").and_then(|m| m.as_array()) {
            let mut tools_from_history: Vec<Value> = Vec::new();
            let mut seen_functions: std::collections::HashSet<String> = std::collections::HashSet::new();

            for msg in messages {
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        if let Some(func) = tc.get("function") {
                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                if seen_functions.insert(name.to_string()) {
                                    let func_obj = func.as_object().cloned().unwrap_or_default();
                                    tools_from_history.push(json!({
                                        "type": "function",
                                        "function": func_obj
                                    }));
                                }
                            }
                        }
                    }
                }
            }

            if !tools_from_history.is_empty() {
                new_obj.insert("tools".to_string(), json!(tools_from_history));
            }
        }
    }

    serde_json::to_string(&new_obj).unwrap_or_else(|_| body_json.to_string())
}

// ============================================================================
// openai -> anthropic response conversion (kept for ?format=anthropic on /v1/chat/completions)
// ============================================================================

pub fn convert_sse_to_anthropic_stream(sse_data: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut sent_message_start = false;
    let mut sent_content_block_start = false;
    let mut thinking_started = false;
    let mut current_prefix = "data:";
    // Track which tool_use block indices we've emitted content_block_start for.
    let mut started_tool_blocks: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut tool_block_counter: usize = 0;

    for line in sse_data.lines() {
        let line = line.trim();
        if line.is_empty() || line == "[DONE]" { continue; }
        if line == "data: [DONE]" { continue; }
        if line.starts_with("event: ") {
            current_prefix = "event:";
            continue;
        }

        let json_str = if line.starts_with("data: ") {
            &line[6..]
        } else {
            continue;
        };

        if let Ok(json) = serde_json::from_str::<Value>(json_str) {
            convert_openai_chunk_to_anthropic(
                &json,
                current_prefix,
                &mut sent_message_start,
                &mut sent_content_block_start,
                &mut thinking_started,
                &mut started_tool_blocks,
                &mut tool_block_counter,
                &mut output,
            );
        }
    }
    Ok(output)
}

fn convert_openai_chunk_to_anthropic(
    chunk: &Value,
    prefix: &str,
    sent_message_start: &mut bool,
    sent_content_block_start: &mut bool,
    thinking_started: &mut bool,
    started_tool_blocks: &mut std::collections::HashSet<usize>,
    tool_block_counter: &mut usize,
    output: &mut String,
) -> Option<()> {
    let id = chunk.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let model = chunk.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let choices = chunk.get("choices").and_then(|v| v.as_array())?;
    let choice = choices.first()?;
    let delta = choice.get("delta")?;
    let finish_reason = choice.get("finish_reason");

    if delta.get("role").is_some() && !*sent_message_start {
        *sent_message_start = true;
        let event = json!({
            "type": "message_start",
            "message": {
                "id": id,
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [],
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        });
        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&event).unwrap_or_default()));
        return Some(());
    }

    // Handle tool_calls streaming: emit content_block_start (tool_use) and
    // input_json_delta events so that downstream Anthropic clients see proper
    // tool_use blocks. Without this, the next request's tool_result blocks
    // would fail with "tool call result does not follow tool call".
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let oa_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

            // Close any open thinking/text block before opening tool_use.
            if *sent_content_block_start && (*thinking_started || oa_index == 0) {
                let stop_event = json!({"type": "content_block_stop", "index": oa_index});
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                *thinking_started = false;
            }

            // Emit content_block_start once per tool_use index.
            if !started_tool_blocks.contains(&oa_index) {
                started_tool_blocks.insert(oa_index);
                let tool_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let tool_name = tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let block_index = *tool_block_counter;
                *tool_block_counter += 1;
                let start_event = json!({
                    "type": "content_block_start",
                    "index": block_index,
                    "content_block": {
                        "type": "tool_use",
                        "id": tool_id,
                        "name": tool_name,
                        "input": {}
                    }
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                *sent_content_block_start = true;

                // Emit any initial arguments if present.
                if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()) {
                    if !args.is_empty() {
                        let delta_event = json!({
                            "type": "content_block_delta",
                            "index": block_index,
                            "delta": {"type": "input_json_delta", "partial_json": args}
                        });
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                    }
                }
            } else {
                // Subsequent chunk for an existing tool block: emit argument delta.
                if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()) {
                    if !args.is_empty() {
                        // Map openai index back to our sequential block index.
                        let block_index = started_tool_blocks.iter().position(|&i| i == oa_index).unwrap_or(oa_index);
                        let delta_event = json!({
                            "type": "content_block_delta",
                            "index": block_index,
                            "delta": {"type": "input_json_delta", "partial_json": args}
                        });
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                    }
                }
            }
        }
        return Some(());
    }

    // Skip reasoning_content from upstream — Claude Code client doesn't recognize
    // Anthropic-style "thinking" content blocks, so emitting them breaks response
    // parsing (e.g. compact fails after a few rounds). Just drop the field entirely.
    if delta.get("reasoning_content").is_some() {
        return Some(());
    }

    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            if !*sent_content_block_start {
                *sent_content_block_start = true;
                let start_event = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text"}
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
            } else if *thinking_started {
                *thinking_started = false;
                let stop_event = json!({"type": "content_block_stop", "index": 0});
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                let start_event = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text"}
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
            }
            let event = json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": content}
            });
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&event).unwrap_or_default()));
            return Some(());
        }
    }

    if let Some(reason) = finish_reason.and_then(|v| v.as_str()) {
        if reason == "stop" || reason == "length" {
            if *thinking_started {
                *thinking_started = false;
                let stop_event = json!({"type": "content_block_stop", "index": 0});
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
            }
            let usage = chunk.get("usage");
            let output_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            let input_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);

            let message_delta = json!({
                "type": "message_delta",
                "delta": {"stop_reason": if reason == "length" { "max_tokens" } else { "end_turn" }},
                "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}
            });
            let message_stop = json!({"type": "message_stop"});
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&message_delta).unwrap_or_default()));
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&message_stop).unwrap_or_default()));
            return Some(());
        }
    }
    None
}

pub fn convert_to_anthropic_response(openai_resp: &Value) -> Result<Value, String> {
    let id = openai_resp.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let model = openai_resp.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let usage = openai_resp.get("usage");
    let input_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let output_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let choice = openai_resp.get("choices").and_then(|v| v.as_array()).and_then(|arr| arr.first());
    let message = choice.and_then(|v| v.get("message"));
    // Skip reasoning_content / thinking blocks from upstream — Claude Code client
    // doesn't recognize Anthropic-style "thinking" content blocks, so emitting
    // them breaks response parsing (e.g. compact fails after a few rounds).
    let tool_calls = message.and_then(|m| m.get("tool_calls")).and_then(|v| v.as_array()).map(|arr| arr.clone()).unwrap_or_default();

    let mut content_blocks = Vec::new();

    let message_content = message.and_then(|m| m.get("content"));
    if let Some(content_val) = message_content {
        if content_val.is_string() {
            if let Some(text) = content_val.as_str() {
                if !text.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": text}));
                }
            }
        } else if let Some(arr) = content_val.as_array() {
            for block in arr {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                content_blocks.push(json!({"type": "text", "text": text}));
                            }
                        }
                    }
                    // "thinking" / "reasoning" blocks from upstream are dropped
                    // entirely (see comment above).
                    "thinking" | "reasoning" => {}
                    _ => {
                        if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                            if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                                let input = block.get("input").cloned().unwrap_or(json!({}));
                                content_blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
                            }
                        }
                    }
                }
            }
        }
    }

    for tc in &tool_calls {
        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let func = tc.get("function").ok_or("missing function in tool_call")?;
        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
        let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
        content_blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
    }

    let finish_reason = choice.and_then(|v| v.get("finish_reason")).and_then(|v| v.as_str()).unwrap_or("end_turn");
    let stop_reason = if !tool_calls.is_empty() {
        "tool_use"
    } else if finish_reason == "length" {
        "max_tokens"
    } else {
        "end_turn"
    };

    Ok(json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}
    }))
}
