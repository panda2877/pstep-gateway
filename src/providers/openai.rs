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
        .timeout(Duration::from_secs(70))
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
    // Suppress content between <think> and </think> (thinking leaks into text field).
    let mut inside_think: bool = false;
    let mut think_tail: String = String::new();

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
                &mut inside_think,
                &mut think_tail,
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
    inside_think: &mut bool,
    think_tail: &mut String,
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
        // NB: do NOT return here. minimaxi M3 sends `role` and `content` (often
        // containing  THINK tags) in the same delta. Returning early would skip
        // the think-tag → thinking content block conversion below, leaving
        // thinking as raw text in the text block (which Claude Code can't
        // parse as thinking). Fall through so the content path can split it.
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
                let stop_event = json!({"type": "content_block_stop", "index": 0});
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                *thinking_started = false;
                *sent_content_block_start = false;
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

    // Upstream `reasoning_content` (e.g. DeepSeek-R1 / o1-style) — emit as
    // a real Anthropic `thinking` content block. Claude Code consumes these
    // and shows the chain-of-thought in its UI; dropping them hides it.
    if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            if !*thinking_started {
                // Close any open text block first.
                if *sent_content_block_start {
                    let stop_event = json!({"type": "content_block_stop", "index": 0});
                    output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                    *sent_content_block_start = false;
                }
                let start_event = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "thinking", "thinking": ""}
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                *thinking_started = true;
            }
            let delta_event = json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": reasoning}
            });
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
        }
        return Some(());
    }

    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            // Split on <think>...</think> boundaries. Each segment is emitted
            // as either thinking_delta (inside the tag) or text_delta (outside
            // the tag). Tag boundaries can split across chunks, so keep a tail
            // buffer (`think_tail`) of at most 7 bytes to detect the closing
            // tag split mid-token.
            let mut buf = std::mem::take(think_tail);
            buf.push_str(content);
            let mut rest = buf.as_str();
            while !rest.is_empty() {
                if *inside_think {
                    if let Some(end) = rest.find("</think>") {
                        // Emit everything up to </think> as thinking_delta.
                        let think_segment = &rest[..end];
                        if !think_segment.is_empty() {
                            if !*thinking_started {
                                // Close any open text block first.
                                if *sent_content_block_start {
                                    let stop_event = json!({"type": "content_block_stop", "index": 0});
                                    output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                                    *sent_content_block_start = false;
                                }
                                let start_event = json!({
                                    "type": "content_block_start",
                                    "index": 0,
                                    "content_block": {"type": "thinking", "thinking": ""}
                                });
                                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                                *thinking_started = true;
                            }
                            let delta_event = json!({
                                "type": "content_block_delta",
                                "index": 0,
                                "delta": {"type": "thinking_delta", "thinking": think_segment}
                            });
                            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                        }
                        // Close the thinking block.
                        let stop_event = json!({"type": "content_block_stop", "index": 0});
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                        *thinking_started = false;
                        rest = &rest[end + 8..];
                        *inside_think = false;
                    } else {
                        // Still inside a think block — emit everything except
                        // the last 7 bytes as thinking_delta, keep the trailing
                        // window in case "</think>" is split across chunks.
                        // Use floor_char_boundary to avoid slicing inside a
                        // UTF-8 multi-byte sequence (CJK, emoji). Break out
                        // when `cut == 0` (e.g. first char is 4-byte emoji
                        // and `rest.len() - 7` lands inside it) — otherwise
                        // `rest` doesn't shrink and we loop forever.
                        if rest.len() > 7 {
                            let cut = rest.floor_char_boundary(rest.len() - 7);
                            if cut == 0 { break; }
                            let think_segment = &rest[..cut];
                            rest = &rest[cut..];
                            if !think_segment.is_empty() {
                                if !*thinking_started {
                                    if *sent_content_block_start {
                                        let stop_event = json!({"type": "content_block_stop", "index": 0});
                                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                                        *sent_content_block_start = false;
                                    }
                                    let start_event = json!({
                                        "type": "content_block_start",
                                        "index": 0,
                                        "content_block": {"type": "thinking", "thinking": ""}
                                    });
                                    output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                                    *thinking_started = true;
                                }
                                let delta_event = json!({
                                    "type": "content_block_delta",
                                    "index": 0,
                                    "delta": {"type": "thinking_delta", "thinking": think_segment}
                                });
                                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                            }
                        } else {
                            break;
                        }
                    }
                } else if let Some(start) = rest.find("<think>") {
                    // Emit everything up to <think> as text_delta.
                    let text_segment = &rest[..start];
                    if !text_segment.is_empty() {
                        // Close any open thinking block first.
                        if *thinking_started {
                            let stop_event = json!({"type": "content_block_stop", "index": 0});
                            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                            *thinking_started = false;
                        }
                        if !*sent_content_block_start {
                            let start_event = json!({
                                "type": "content_block_start",
                                "index": 0,
                                "content_block": {"type": "text"}
                            });
                            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                            *sent_content_block_start = true;
                        }
                        let delta_event = json!({
                            "type": "content_block_delta",
                            "index": 0,
                            "delta": {"type": "text_delta", "text": text_segment}
                        });
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                    }
                    rest = &rest[start + 7..];
                    *inside_think = true;
                } else {
                    // Plain text (no think tag) — emit everything except the
                    // last 6 bytes as text_delta, keep trailing window in case
                    // "<think>" is split across chunks. floor_char_boundary to
                    // avoid slicing inside a UTF-8 multi-byte sequence. Break
                    // out when `cut == 0` (e.g. first char is 4-byte emoji and
                    // `rest.len() - 6` lands inside it) — otherwise `rest`
                    // doesn't shrink and we loop forever.
                    if rest.len() > 6 {
                        let cut = rest.floor_char_boundary(rest.len() - 6);
                        if cut == 0 { break; }
                        let text_segment = &rest[..cut];
                        rest = &rest[cut..];
                        if !text_segment.is_empty() {
                            if *thinking_started {
                                let stop_event = json!({"type": "content_block_stop", "index": 0});
                                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                                *thinking_started = false;
                            }
                            if !*sent_content_block_start {
                                let start_event = json!({
                                    "type": "content_block_start",
                                    "index": 0,
                                    "content_block": {"type": "text"}
                                });
                                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                                *sent_content_block_start = true;
                            }
                            let delta_event = json!({
                                "type": "content_block_delta",
                                "index": 0,
                                "delta": {"type": "text_delta", "text": text_segment}
                            });
                            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                        }
                    } else {
                        break;
                    }
                }
            }
            *think_tail = rest.to_string();
            return Some(());
        }
    }

    if let Some(reason) = finish_reason.and_then(|v| v.as_str()) {
        if reason == "stop" || reason == "length" {
            // Flush any buffered tail content. The state machine holds back
            // up to 6 (or 7) bytes to detect `<think>`/`</think>` splits across
            // chunks. At finish there are no more chunks, so emit whatever
            // was held back as the current block type, then close the block.
            if !think_tail.is_empty() {
                if *inside_think {
                    if !*thinking_started {
                        if *sent_content_block_start {
                            let stop_event = json!({"type": "content_block_stop", "index": 0});
                            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                            *sent_content_block_start = false;
                        }
                        let start_event = json!({
                            "type": "content_block_start",
                            "index": 0,
                            "content_block": {"type": "thinking", "thinking": ""}
                        });
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                        *thinking_started = true;
                    }
                    let delta_event = json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "thinking_delta", "thinking": think_tail.as_str()}
                    });
                    output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                } else {
                    if *thinking_started {
                        let stop_event = json!({"type": "content_block_stop", "index": 0});
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                        *thinking_started = false;
                    }
                    if !*sent_content_block_start {
                        let start_event = json!({
                            "type": "content_block_start",
                            "index": 0,
                            "content_block": {"type": "text"}
                        });
                        output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
                        *sent_content_block_start = true;
                    }
                    let delta_event = json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "text_delta", "text": think_tail.as_str()}
                    });
                    output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&delta_event).unwrap_or_default()));
                }
                think_tail.clear();
            }
            // Close any open thinking block (e.g. upstream ended mid-think
            // without sending </think>).
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

/// Split `text` into alternating (text, think) segments. `<think>` opens a
/// think segment, `</think>` closes it. The first tuple element is a slice
/// of the original text that should be emitted as a `text` block, the second
/// is a slice that should be emitted as a `thinking` block. Returns
/// (text_segments, think_segments) where each Vec contains the segments in
/// order, plus a leading newlines bool to drop think-tag whitespace.
fn split_think_tags(text: &str) -> (Vec<&str>, Vec<&str>) {
    let mut texts = Vec::new();
    let mut thinks = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("<think>") {
        if open > 0 {
            texts.push(&rest[..open]);
        }
        rest = &rest[open + 7..];
        match rest.find("</think>") {
            Some(close) => {
                thinks.push(&rest[..close]);
                rest = &rest[close + 8..];
            }
            None => {
                // Unclosed tag — keep the tail as thinking (best effort).
                thinks.push(rest);
                return (texts, thinks);
            }
        }
    }
    if !rest.is_empty() {
        texts.push(rest);
    }
    (texts, thinks)
}

pub fn convert_to_anthropic_response(openai_resp: &Value) -> Result<Value, String> {
    let id = openai_resp.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let model = openai_resp.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let usage = openai_resp.get("usage");
    let input_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let output_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let choice = openai_resp.get("choices").and_then(|v| v.as_array()).and_then(|arr| arr.first());
    let message = choice.and_then(|v| v.get("message"));
    let tool_calls = message.and_then(|m| m.get("tool_calls")).and_then(|v| v.as_array()).map(|arr| arr.clone()).unwrap_or_default();

    let mut content_blocks = Vec::new();

    // Helper: emit a single text segment, trimming the leading newlines that
    // often follow a `</think>` tag so the text block doesn't start with a
    // blank line.
    let push_text = |blocks: &mut Vec<Value>, segment: &str| {
        let trimmed = segment.trim_start_matches('\n');
        if !trimmed.is_empty() {
            blocks.push(json!({"type": "text", "text": trimmed}));
        }
    };

    let message_content = message.and_then(|m| m.get("content"));
    if let Some(content_val) = message_content {
        if content_val.is_string() {
            if let Some(text) = content_val.as_str() {
                let (texts, thinks) = split_think_tags(text);
                for t in thinks {
                    if !t.is_empty() {
                        content_blocks.push(json!({"type": "thinking", "thinking": t}));
                    }
                }
                for t in texts {
                    push_text(&mut content_blocks, t);
                }
            }
        } else if let Some(arr) = content_val.as_array() {
            for block in arr {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            let (texts, thinks) = split_think_tags(text);
                            for t in thinks {
                                if !t.is_empty() {
                                    content_blocks.push(json!({"type": "thinking", "thinking": t}));
                                }
                            }
                            for t in texts {
                                push_text(&mut content_blocks, t);
                            }
                        }
                    }
                    "thinking" | "reasoning" => {
                        if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                            if !t.is_empty() {
                                content_blocks.push(json!({"type": "thinking", "thinking": t}));
                            }
                        } else if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            if !t.is_empty() {
                                content_blocks.push(json!({"type": "thinking", "thinking": t}));
                            }
                        }
                    }
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

    // Upstream `reasoning_content` (OpenAI o1 / DeepSeek-R1 style) — emit as
    // a real thinking content block so Claude Code can show it in the UI.
    if let Some(reasoning) = message.and_then(|m| m.get("reasoning_content")).and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            content_blocks.push(json!({"type": "thinking", "thinking": reasoning}));
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

// ============================================================================
// Tests
// ============================================================================
//
// Regression coverage for the UTF-8 byte-slice panic in
// `convert_sse_to_anthropic_stream` that used to kill tokio workers mid-stream
// when upstream content contained multi-byte chars (CJK, emoji) at chunk
// boundaries. The tail-buffer used to slice by raw byte count (rest.len() - 6
// and -7), which could land in the middle of a 3/4-byte UTF-8 sequence and
// panic with "byte index N is not a char boundary". See journal:
//   thread 'tokio-rt-worker' panicked at src/providers/openai.rs:502
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal SSE delta chunk carrying the given content string.
    fn sse_delta(content: &str) -> String {
        // The minimal payload the converter needs: id, model, choices[0].delta.content
        let json = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "model": "MiniMax-M3",
            "choices": [{
                "index": 0,
                "delta": { "content": content },
                "finish_reason": null
            }]
        });
        format!("data: {}\n", json)
    }

    /// The bug case: content where the last 6 bytes land inside a CJK char.
    /// `rest = "abcdef中文"` (12 bytes; '中' is at 6..9, '文' at 9..12).
    /// Old code did `&rest[..rest.len()-6]` = `&rest[..6]` which slices inside
    /// '中' (byte 6 is mid-character) and panics.
    #[test]
    fn sse_to_anthropic_handles_cjk_tail() {
        let sse = sse_delta("abcdef中文");
        let result = convert_sse_to_anthropic_stream(&sse);
        assert!(result.is_ok(), "must not panic on CJK content: {:?}", result.err());
        let out = result.unwrap();
        // The first 5 ASCII bytes should be emitted; the 6th byte slot is
        // mid-CJK, so the converter trims back to the last char boundary and
        // keeps the trailing CJK as think_tail for the next delta.
        assert!(out.contains("abcde"), "expected emitted prefix 'abcde', got: {out}");
    }

    /// Emoji (4-byte UTF-8) is the same bug class — verified against the real
    /// panic observed at 22:12:21 with '👋' (bytes 11..15 of string).
    #[test]
    fn sse_to_anthropic_handles_emoji_tail() {
        // 8 ASCII + emoji '👋' + 'X' = 8 + 4 + 1 = 13 bytes.
        // Old code: &rest[..13-6] = &rest[..7] — byte 7 is inside '👋' (4..8).
        let sse = sse_delta("abcdefgh👋X");
        let result = convert_sse_to_anthropic_stream(&sse);
        assert!(result.is_ok(), "must not panic on emoji content: {:?}", result.err());
    }

    /// Baseline: pure ASCII still works and emits content. The last 6 bytes
    /// (" world") are kept as think_tail — they would be flushed by the next
    /// delta. We just verify the leading portion is emitted.
    #[test]
    fn sse_to_anthropic_ascii_baseline() {
        let sse = sse_delta("hello world");
        let result = convert_sse_to_anthropic_stream(&sse).unwrap();
        assert!(result.contains("hello"), "expected emitted prefix, got: {result}");
        // The trailing " world" is held in think_tail (verified by absence in
        // the first delta's output, not absence of the assistant text).
    }

    /// Tail shorter than the limit (≤ 6 bytes) is preserved whole — should
    /// emit nothing yet (kept as think_tail for the next delta) and not panic.
    #[test]
    fn sse_to_anthropic_short_cjk_tail() {
        // 5 bytes total, all CJK. ≤ 6 bytes → take the else branch (break).
        let sse = sse_delta("中文");
        let result = convert_sse_to_anthropic_stream(&sse);
        assert!(result.is_ok(), "must not panic on short CJK content: {:?}", result.err());
    }

    /// Inside-think path: when the buffer accumulates content up to and past
    /// the `<think>` open tag with a CJK tail, the 7-byte tail trim must also
    /// be char-boundary safe.
    #[test]
    fn sse_to_anthropic_inside_think_cjk_tail() {
        // Two deltas: first opens a <think> block, second accumulates tail.
        let sse1 = sse_delta("<think>");
        let sse2 = sse_delta("思考中中文");
        // Combined: think_tail collects "思考中中文" (12 bytes) and the 7-byte
        // tail trim lands inside '中' (bytes 6..9). Old code panicked.
        let combined = format!("{sse1}{sse2}");
        let result = convert_sse_to_anthropic_stream(&combined);
        assert!(result.is_ok(), "must not panic inside-think with CJK tail: {:?}", result.err());
    }

    // ----- thinking content block emission tests -----------------------------
    //
    // Regression coverage for the bug where M3's `<think>...</think>` text was
    // silently dropped on the /v1/messages and ?format=anthropic paths, hiding
    // chain-of-thought from Claude Code. After the fix, `<think>...</think>`
    // content is emitted as a real Anthropic `thinking` content block.

    /// Helper: parse an Anthropic SSE stream into a Vec of typed events.
    fn parse_anthropic_sse_events(out: &str) -> Vec<Value> {
        out.lines()
            .filter_map(|line| {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("data: ") {
                    serde_json::from_str::<Value>(rest).ok()
                } else if let Some(rest) = line.strip_prefix("event: ") {
                    // `event:` lines are not used by this converter (it always
                    // emits `data: <json>` regardless of the source prefix),
                    // so skip them.
                    let _ = rest;
                    None
                } else {
                    None
                }
            })
            .collect()
    }

    /// M3's full pattern: a chunk with both role + content (per c47afa6), then
    /// a <think> opening, then 3 chunks of think content, then </think>, then
    /// the answer, then finish. After the fix the response should contain both
    /// a `thinking` content block AND a `text` content block.
    #[test]
    fn sse_to_anthropic_emits_thinking_block() {
        let mut sse = String::new();
        // chunk 1: role + role-content (the c47afa6 case)
        sse.push_str(&sse_delta(""));
        sse.push_str(&sse_delta("<think>The user is asking about 1+1."));
        sse.push_str(&sse_delta(" It is a simple question."));
        sse.push_str(&sse_delta("</think>\n\n1+1=2。"));
        sse.push_str(&format!(
            "data: {}\n",
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion.chunk",
                "model": "MiniMax-M3",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 10}
            })
        ));

        let out = convert_sse_to_anthropic_stream(&sse).expect("convert ok");

        // We need to inject a message_start; the test chunk above has no
        // `role` field, so no message_start was emitted. The fix still works
        // for the content path; just verify the events we care about.
        let events = parse_anthropic_sse_events(&out);
        let types: Vec<&str> = events.iter()
            .filter_map(|e| e.get("type").and_then(|t| t.as_str()))
            .collect();

        // We expect: content_block_start (thinking), content_block_delta
        // (thinking_delta x N), content_block_stop (thinking),
        // content_block_start (text), content_block_delta (text_delta),
        // content_block_stop (text), message_delta, message_stop.
        assert!(types.contains(&"content_block_start"), "no content_block_start: {types:?}");
        assert!(types.contains(&"content_block_stop"), "no content_block_stop: {types:?}");
        assert!(types.contains(&"message_stop"), "no message_stop: {types:?}");

        // Find the thinking block and verify it carried the think content.
        let mut thinking_text = String::new();
        let mut text_text = String::new();
        let mut current_type: Option<String> = None;
        for ev in &events {
            if ev.get("type").and_then(|t| t.as_str()) == Some("content_block_start") {
                current_type = ev.get("content_block")
                    .and_then(|cb| cb.get("type"))
                    .and_then(|t| t.as_str())
                    .map(String::from);
            } else if ev.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                let delta = ev.get("delta").cloned().unwrap_or(json!({}));
                if delta.get("type").and_then(|t| t.as_str()) == Some("thinking_delta") {
                    if let Some(t) = delta.get("thinking").and_then(|t| t.as_str()) {
                        thinking_text.push_str(t);
                    }
                } else if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                    if let Some(t) = delta.get("text").and_then(|t| t.as_str()) {
                        text_text.push_str(t);
                    }
                }
            } else if ev.get("type").and_then(|t| t.as_str()) == Some("content_block_stop") {
                current_type = None;
            }
        }
        let _ = current_type;

        assert!(
            thinking_text.contains("simple question"),
            "expected thinking content to be present, got: {thinking_text:?}"
        );
        assert!(
            text_text.contains("1+1=2"),
            "expected answer text to be present, got: {text_text:?}"
        );
        // Think tags must NOT appear in the text content.
        assert!(
            !text_text.contains("<think>") && !text_text.contains("</think>"),
            "text content leaked think tags: {text_text:?}"
        );
    }

    /// Upstream `reasoning_content` field (DeepSeek-R1 / o1 style) — emitted
    /// as thinking_delta instead of being dropped.
    #[test]
    fn sse_to_anthropic_emits_reasoning_content() {
        // Build a chunk with reasoning_content in delta.
        let chunk = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "model": "deepseek-r1",
            "choices": [{
                "index": 0,
                "delta": { "reasoning_content": "Let me think..." },
                "finish_reason": null
            }]
        });
        let sse = format!("data: {chunk}\n");
        let out = convert_sse_to_anthropic_stream(&sse).expect("convert ok");
        let events = parse_anthropic_sse_events(&out);
        let has_thinking_start = events.iter().any(|e| {
            e.get("type").and_then(|t| t.as_str()) == Some("content_block_start")
                && e.get("content_block")
                    .and_then(|cb| cb.get("type"))
                    .and_then(|t| t.as_str()) == Some("thinking")
        });
        let has_thinking_delta = events.iter().any(|e| {
            e.get("type").and_then(|t| t.as_str()) == Some("content_block_delta")
                && e.get("delta")
                    .and_then(|d| d.get("type"))
                    .and_then(|t| t.as_str()) == Some("thinking_delta")
        });
        assert!(has_thinking_start, "expected thinking content_block_start in: {out}");
        assert!(has_thinking_delta, "expected thinking_delta event in: {out}");
    }

    /// Non-streaming: text content with embedded <think>...</think> gets
    /// split into a `thinking` block and a `text` block, preserving the
    /// actual think content (rather than dropping it).
    #[test]
    fn nonstream_extracts_thinking_block() {
        let resp = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "model": "MiniMax-M3",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<think>The user asks 1+1. The answer is 2.</think>\n\n1+1=2。"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10}
        });
        let v = convert_to_anthropic_response(&resp).expect("convert ok");
        let blocks = v.get("content").and_then(|c| c.as_array()).expect("content array");
        let types: Vec<&str> = blocks.iter()
            .filter_map(|b| b.get("type").and_then(|t| t.as_str()))
            .collect();
        assert!(types.contains(&"thinking"), "expected a thinking block, got: {types:?}");
        assert!(types.contains(&"text"), "expected a text block, got: {types:?}");

        // Thinking block should contain the actual think text.
        let thinking_text: String = blocks.iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking"))
            .filter_map(|b| b.get("thinking").and_then(|t| t.as_str()))
            .collect();
        assert!(thinking_text.contains("The answer is 2"), "expected think content, got: {thinking_text:?}");

        // Text block should contain only the answer, with no think tags.
        let text_text: String = blocks.iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect();
        assert!(text_text.contains("1+1=2"), "expected answer text, got: {text_text:?}");
        assert!(!text_text.contains("<think>"), "text leaked think tags: {text_text:?}");
    }

    /// Non-streaming: an explicit `reasoning_content` field on the message is
    /// emitted as a thinking content block (previously dropped).
    #[test]
    fn nonstream_emits_reasoning_content() {
        let resp = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "model": "deepseek-r1",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "The answer is 42.",
                    "reasoning_content": "Let me reason about this carefully."
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10}
        });
        let v = convert_to_anthropic_response(&resp).expect("convert ok");
        let blocks = v.get("content").and_then(|c| c.as_array()).expect("content array");
        let has_thinking = blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("thinking")
                && b.get("thinking").and_then(|t| t.as_str())
                    .map(|t| t.contains("reason about this"))
                    .unwrap_or(false)
        });
        assert!(has_thinking, "expected thinking block from reasoning_content, blocks: {blocks:?}");
    }
}
