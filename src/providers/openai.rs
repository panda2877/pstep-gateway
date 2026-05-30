use crate::providers::OutputFormat;
use crate::types::UpstreamConfig;
use reqwest::Client;
use std::time::Duration;

pub async fn proxy(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    eprintln!("[DEBUG] proxy received body: {}", body);
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let body_json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let modified_body = if let Some(obj) = body_json.as_object() {
        let mut new_obj = obj.clone();
        new_obj.insert("model".to_string(), serde_json::json!(target_model));

        // Fix tools: ensure each tool has a "type" field (default to "function")
        if let Some(tools) = new_obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
            for tool in tools.iter_mut() {
                if let Some(tool_obj) = tool.as_object_mut() {
                    let type_val = tool_obj.get("type");
                    let needs_fix = match type_val {
                        Some(serde_json::Value::String(s)) if !s.is_empty() => false,
                        Some(serde_json::Value::Null) => true,
                        None => true,
                        _ => true,
                    };
                    if needs_fix {
                        tool_obj.insert("type".to_string(), serde_json::json!("function"));
                    }
                }
            }
        }

        // If there's a role:tool message but no tools field, reconstruct tools from history
        // This is needed because MiniMax and some other providers validate tool_call_id
        // against the original tool definitions in the session context
        let has_tools = new_obj.get("tools").is_some();
        let has_tool_role_message = new_obj.get("messages")
            .and_then(|m| m.as_array())
            .map(|msgs| msgs.iter().any(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool")))
            .unwrap_or(false);

        if !has_tools && has_tool_role_message {
            if let Some(messages) = new_obj.get("messages").and_then(|m| m.as_array()) {
                let mut tools_from_history: Vec<serde_json::Value> = Vec::new();
                let mut seen_functions: std::collections::HashSet<String> = std::collections::HashSet::new();

                for msg in messages {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                        for tc in tool_calls {
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                    if seen_functions.insert(name.to_string()) {
                                        let func_obj = func.as_object().cloned().unwrap_or_default();
                                        tools_from_history.push(serde_json::json!({
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
                    new_obj.insert("tools".to_string(), serde_json::json!(tools_from_history));
                }
            }
        }

        let result = serde_json::to_string(&new_obj).map_err(|e| e.to_string())?;
        result
    } else {
        body.to_string()
    };

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
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    if format == OutputFormat::OpenAI {
        Ok(body_str.to_string())
    } else if body_str.contains("event:") || body_str.contains("data:") {
        // Already in Anthropic SSE format, pass through
        if body_str.contains("message_start") || body_str.contains("content_block_start") {
            Ok(body_str.to_string())
        } else {
            // It's OpenAI SSE format, need to convert
            convert_sse_to_anthropic_stream(&body_str)
        }
    } else {
        let json: serde_json::Value = serde_json::from_str(&body_str)
            .map_err(|e| e.to_string())?;
        let anthropic_resp = convert_to_anthropic_response(&json)?;
        serde_json::to_string(&anthropic_resp).map_err(|e| e.to_string())
    }
}

fn convert_sse_to_anthropic_stream(sse_data: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut sent_message_start = false;
    let mut sent_content_block_start = false;
    let mut thinking_started = false;
    let mut current_prefix = "data:";

    for line in sse_data.lines() {
        let line = line.trim();
        if line.is_empty() || line == "[DONE]" {
            continue;
        }

        if line == "data: [DONE]" {
            continue;
        }

        if line.starts_with("event: ") {
            current_prefix = "event:";
            continue;
        }

        let json_str = if line.starts_with("data: ") {
            &line[6..]
        } else {
            continue;
        };

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            convert_openai_chunk_to_anthropic(&json, current_prefix, &mut sent_message_start, &mut sent_content_block_start, &mut thinking_started, &mut output);
        }
    }

    Ok(output)
}

fn convert_openai_chunk_to_anthropic(chunk: &serde_json::Value, prefix: &str, sent_message_start: &mut bool, sent_content_block_start: &mut bool, thinking_started: &mut bool, output: &mut String) -> Option<()> {
    let id = chunk.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let model = chunk.get("model").and_then(|v| v.as_str()).unwrap_or("");

    let choices = chunk.get("choices").and_then(|v| v.as_array())?;
    let choice = choices.first()?;

    let delta = choice.get("delta")?;
    let finish_reason = choice.get("finish_reason");

    if delta.get("role").is_some() && !*sent_message_start {
        *sent_message_start = true;
        let event = serde_json::json!({
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

    if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            if !*sent_content_block_start {
                *sent_content_block_start = true;
                *thinking_started = true;
                let start_event = serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "thinking",
                        "thinking": "",
                        "signature": ""
                    }
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
            }
            let event = serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "thinking_delta",
                    "thinking": reasoning
                }
            });
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&event).unwrap_or_default()));
            return Some(());
        }
    }

    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            if !*sent_content_block_start {
                *sent_content_block_start = true;
                let start_event = serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "text"
                    }
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
            } else if *thinking_started {
                *thinking_started = false;
                let stop_event = serde_json::json!({
                    "type": "content_block_stop",
                    "index": 0
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
                let start_event = serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "text"
                    }
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&start_event).unwrap_or_default()));
            }
            let event = serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "text_delta",
                    "text": content
                }
            });
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&event).unwrap_or_default()));
            return Some(());
        }
    }

    if let Some(reason) = finish_reason.and_then(|v| v.as_str()) {
        if reason == "stop" || reason == "length" {
            if *thinking_started {
                *thinking_started = false;
                let stop_event = serde_json::json!({
                    "type": "content_block_stop",
                    "index": 0
                });
                output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&stop_event).unwrap_or_default()));
            }
            let usage = chunk.get("usage");
            let output_tokens = usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let input_tokens = usage
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let message_delta = serde_json::json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": if reason == "length" { "max_tokens" } else { "end_turn" }
                },
                "usage": {
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens
                }
            });
            let message_stop = serde_json::json!({
                "type": "message_stop"
            });
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&message_delta).unwrap_or_default()));
            output.push_str(&format!("{} {}\n", prefix, serde_json::to_string(&message_stop).unwrap_or_default()));
            return Some(());
        }
    }
    None
}

fn extract_thinking_from_text(text: &str) -> (Option<String>, String) {
    let mut thinking_blocks: Vec<String> = Vec::new();
    let mut remaining_text = String::new();
    let mut in_thinking = false;
    let mut current_block = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check for opening tag (7 chars: <think>) - only when not already in thinking
        if !in_thinking && i + 7 <= chars.len() {
            let opening: String = chars[i..i+7].iter().collect();
            if opening == "<think>" {
                i += 7;
                in_thinking = true;
                continue;
            }
        }

        // Check for closing tag - only when in thinking
        if in_thinking && i + 8 <= chars.len() {
            let closing: String = chars[i..i+8].iter().collect();
            if closing.starts_with("</think>") {
                // Found closing tag
                let actual_closing_len = closing.find('>').map(|p| p + 1).unwrap_or(8);
                i += actual_closing_len;
                in_thinking = false;
                if !current_block.is_empty() {
                    thinking_blocks.push(current_block.trim().to_string());
                    current_block = String::new();
                }
                // Skip trailing newlines
                while i < chars.len() && (chars[i] == '\n' || chars[i] == '\r') {
                    i += 1;
                }
                continue;
            }
        }

        // Regular character
        if in_thinking {
            current_block.push(chars[i]);
        } else {
            remaining_text.push(chars[i]);
        }
        i += 1;
    }

    // Handle any remaining thinking content (unclosed tag)
    if in_thinking && !current_block.is_empty() {
        thinking_blocks.push(current_block.trim().to_string());
    }

    let thinking = if thinking_blocks.is_empty() {
        None
    } else {
        Some(thinking_blocks.join("\n"))
    };

    (thinking, remaining_text.trim().to_string())
}

fn convert_to_anthropic_response(openai_resp: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = openai_resp.get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let model = openai_resp.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let usage = openai_resp.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let output_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let choice = openai_resp.get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first());
    let message = choice
        .and_then(|v| v.get("message"));

    // Try to get reasoning_content from the message first
    let reasoning_from_field = message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let tool_calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.clone())
        .unwrap_or_default();

    // Build content blocks
    let mut content_blocks = Vec::new();
    let mut thinking_content: Option<String> = reasoning_from_field;

    // Handle content: can be plain string OR array with text/thinking blocks
    let message_content = message.and_then(|m| m.get("content"));
    if let Some(content_val) = message_content {
        if content_val.is_string() {
            // Plain text string - extract thinking if embedded
            if let Some(text) = content_val.as_str() {
                let (extracted_thinking, clean_text) = extract_thinking_from_text(text);
                // Merge thinking content
                if let Some(extracted) = extracted_thinking {
                    thinking_content = Some(
                        thinking_content.map(|h| h + "\n" + &extracted).unwrap_or(extracted)
                    );
                }
                if !clean_text.is_empty() {
                    content_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": clean_text
                    }));
                }
            }
        } else if let Some(arr) = content_val.as_array() {
            // Array content: extract text and thinking blocks
            for block in arr {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            let (extracted_thinking, clean_text) = extract_thinking_from_text(text);
                            if let Some(extracted) = extracted_thinking {
                                thinking_content = Some(
                                    thinking_content.map(|h| h + "\n" + &extracted).unwrap_or(extracted)
                                );
                            }
                            if !clean_text.is_empty() {
                                content_blocks.push(serde_json::json!({
                                    "type": "text",
                                    "text": clean_text
                                }));
                            }
                        }
                    }
                    "thinking" => {
                        if let Some(text) = block.get("thinking").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                thinking_content = Some(
                                    thinking_content.map(|h| h + "\n" + text).unwrap_or_else(|| text.to_string())
                                );
                            }
                        }
                    }
                    _ => {
                        // If it's a tool_use block from the API
                        if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                            if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                                let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                                content_blocks.push(serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // Add thinking block if we found any thinking content
    if let Some(ref reasoning) = thinking_content {
        if !reasoning.is_empty() {
            content_blocks.insert(0, serde_json::json!({
                "type": "thinking",
                "thinking": reasoning,
                "signature": ""
            }));
        }
    }

    // Add tool_use blocks for each tool_call
    for tc in &tool_calls {
        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let func = tc.get("function").ok_or("missing function in tool_call")?;
        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
        let input: serde_json::Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));

        content_blocks.push(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input
        }));
    }

    // Determine stop_reason
    let finish_reason = choice
        .and_then(|v| v.get("finish_reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn");
    let stop_reason = if !tool_calls.is_empty() {
        "tool_use"
    } else if finish_reason == "length" {
        "max_tokens"
    } else {
        "end_turn"
    };

    Ok(serde_json::json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": serde_json::Value::Null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    }))
}

pub async fn proxy_non_stream(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let body_json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let modified_body = if let Some(obj) = body_json.as_object() {
        let mut new_obj = obj.clone();
        new_obj.insert("model".to_string(), serde_json::json!(target_model));

        // Fix tools: ensure each tool has a "type" field (default to "function")
        if let Some(tools) = new_obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
            for tool in tools.iter_mut() {
                if let Some(tool_obj) = tool.as_object_mut() {
                    let type_val = tool_obj.get("type");
                    let needs_fix = match type_val {
                        Some(serde_json::Value::String(s)) if !s.is_empty() => false,
                        Some(serde_json::Value::Null) => true,
                        None => true,
                        _ => true,
                    };
                    if needs_fix {
                        tool_obj.insert("type".to_string(), serde_json::json!("function"));
                    }
                }
            }
        }

        // If there's a role:tool message but no tools field, reconstruct tools from history
        // This is needed because MiniMax and some other providers validate tool_call_id
        // against the original tool definitions in the session context
        let has_tools = new_obj.get("tools").is_some();
        let has_tool_role_message = new_obj.get("messages")
            .and_then(|m| m.as_array())
            .map(|msgs| msgs.iter().any(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool")))
            .unwrap_or(false);

        if !has_tools && has_tool_role_message {
            if let Some(messages) = new_obj.get("messages").and_then(|m| m.as_array()) {
                let mut tools_from_history: Vec<serde_json::Value> = Vec::new();
                let mut seen_functions: std::collections::HashSet<String> = std::collections::HashSet::new();

                for msg in messages {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                        for tc in tool_calls {
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                    if seen_functions.insert(name.to_string()) {
                                        let func_obj = func.as_object().cloned().unwrap_or_default();
                                        tools_from_history.push(serde_json::json!({
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
                    new_obj.insert("tools".to_string(), serde_json::json!(tools_from_history));
                }
            }
        }

        let result = serde_json::to_string(&new_obj).map_err(|e| e.to_string())?;
        result
    } else {
        body.to_string()
    };

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
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    if format == OutputFormat::OpenAI {
        Ok(body_str.to_string())
    } else {
        // Non-streaming: convert JSON response to Anthropic format
        let json: serde_json::Value = serde_json::from_str(&body_str)
            .map_err(|e| e.to_string())?;
        let anthropic_resp = convert_to_anthropic_response(&json)?;
        serde_json::to_string(&anthropic_resp).map_err(|e| e.to_string())
    }
}