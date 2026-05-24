use crate::providers::OutputFormat;
use crate::types::{AnthropicRequest, AnthropicContent, AnthropicMessage, UpstreamConfig, ChatCompletionsRequest, ContentValue};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

pub async fn proxy(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = convert_to_anthropic(&openai_req, target_model)?;
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

    if format == OutputFormat::Anthropic {
        Ok(body_str.to_string())
    } else {
        let json: Value = serde_json::from_str(&body_str)
            .map_err(|e| e.to_string())?;
        let openai_resp = convert_to_openai(&json)?;
        serde_json::to_string(&openai_resp).map_err(|e| e.to_string())
    }
}

fn convert_to_anthropic(openai_req: &ChatCompletionsRequest, target_model: &str) -> Result<AnthropicRequest, String> {
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
                    let json_values: Vec<serde_json::Value> = arr.iter().map(|part| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("type".to_string(), serde_json::json!(part.part_type));
                        if let Some(text) = &part.text {
                            obj.insert("text".to_string(), serde_json::json!(text));
                        }
                        serde_json::Value::Object(obj)
                    }).collect();
                    let converted: Vec<crate::types::AnthropicContentBlock> = json_values.into_iter().filter_map(|v| {
                        serde_json::from_value(v).ok()
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
            Some(crate::types::AnthropicTool {
                tool_type: "function".to_string(),
                name: func.name.clone(),
                description: func.description.clone(),
                input_schema: func.parameters.clone(),
            })
        }).collect()
    });

    let tool_choice = openai_req.tool_choice.as_ref().map(|tc| {
        match tc {
            crate::types::ToolChoice::String(s) => {
                match s.as_str() {
                    "auto" => Some(crate::types::AnthropicToolChoice { choice_type: "auto".to_string(), name: None }),
                    "none" => Some(crate::types::AnthropicToolChoice { choice_type: "any".to_string(), name: None }),
                    "required" => Some(crate::types::AnthropicToolChoice { choice_type: "any".to_string(), name: None }),
                    _ => None,
                }
            }
            crate::types::ToolChoice::Object(obj) => {
                Some(crate::types::AnthropicToolChoice {
                    choice_type: "tool".to_string(),
                    name: obj.function.as_ref().map(|f| f.name.clone()),
                })
            }
        }
    }).flatten();

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

fn convert_to_openai(anthropic_resp: &Value) -> Result<Value, String> {
    let id = anthropic_resp.get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(&format!("chatcmpl-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)))
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
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
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

    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = convert_to_anthropic(&openai_req, target_model)?;
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

    if format == OutputFormat::Anthropic {
        Ok(body_str.to_string())
    } else {
        let json: Value = serde_json::from_str(&body_str)
            .map_err(|e| e.to_string())?;
        let openai_resp = convert_to_openai(&json)?;
        serde_json::to_string(&openai_resp).map_err(|e| e.to_string())
    }
}