use crate::types::{AdminDistributionResponse, AdminUsageStats, ModelDistribution};
use crate::AppState;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};

#[derive(Debug, serde::Deserialize)]
pub struct UsageQuery {
    pub period: Option<String>,
}

/// Model colors for distribution chart
fn get_model_color(model: &str) -> &'static str {
    if model.contains("gpt") || model.contains("GPT") {
        "#3fb950"
    } else if model.contains("claude") || model.contains("Claude") {
        "#58a6ff"
    } else if model.contains("gemini") || model.contains("Gemini") {
        "#d29922"
    } else if model.contains("deepseek") || model.contains("DeepSeek") {
        "#a78bfa"
    } else {
        "#8b949e"
    }
}

fn get_period_hours(period: &str) -> u64 {
    match period {
        "1d" => 24,
        "7d" => 24 * 7,
        "30d" => 24 * 30,
        _ => 24 * 7, // default 7 days
    }
}

/// GET /api/admin/usage/stats
pub async fn usage_stats(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> impl IntoResponse {
    let period = query.period.unwrap_or_else(|| "7d".to_string());
    let hours = get_period_hours(&period);

    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(hours * 3600);

    // Get stats from router's usage tracker
    let all_recent = state.router.get_usage_tracker().get_recent(10000);
    let prev_cutoff = cutoff.saturating_sub(hours * 3600);

    // Filter by period
    let filtered: Vec<_> = all_recent
        .iter()
        .filter(|r| r.timestamp > cutoff)
        .collect();

    let token_input: u64 = filtered.iter().map(|r| r.prompt_tokens as u64).sum();
    let token_output: u64 = filtered.iter().map(|r| r.completion_tokens as u64).sum();
    let token_total = token_input + token_output;

    // Estimate cost (simplified pricing: $0.001/1K input, $0.003/1K output)
    let cost = (token_input as f64 * 0.000001) + (token_output as f64 * 0.000003);

    // Calculate change percent (compare with previous period)
    let prev_total: u64 = all_recent
        .iter()
        .filter(|r| r.timestamp > prev_cutoff && r.timestamp <= cutoff)
        .map(|r| (r.prompt_tokens + r.completion_tokens) as u64)
        .sum();

    let change_percent = if prev_total > 0 {
        ((token_total as f32 - prev_total as f32) / prev_total as f32) * 100.0
    } else {
        0.0
    };

    Json(AdminUsageStats {
        token_total,
        token_input,
        token_output,
        cost,
        change_percent,
        period,
    })
}

/// GET /api/admin/usage/distribution
pub async fn usage_distribution(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> impl IntoResponse {
    let period = query.period.unwrap_or_else(|| "7d".to_string());
    let hours = get_period_hours(&period);

    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(hours * 3600);

    let recent = state.router.get_usage_tracker().get_recent(10000);
    let filtered: Vec<_> = recent
        .iter()
        .filter(|r| r.timestamp > cutoff)
        .collect();

    // Calculate total tokens per model
    let mut by_model: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for r in &filtered {
        let tokens = (r.prompt_tokens + r.completion_tokens) as u64;
        *by_model.entry(r.model.clone()).or_insert(0) += tokens;
    }

    let total: u64 = by_model.values().sum();
    let distributions: Vec<ModelDistribution> = by_model
        .into_iter()
        .map(|(name, tokens)| {
            let color = get_model_color(&name).to_string();
            let percent = if total > 0 {
                (tokens as f32 / total as f32) * 100.0
            } else {
                0.0
            };
            ModelDistribution {
                name,
                color,
                percent,
                tokens,
            }
        })
        .collect();

    Json(AdminDistributionResponse {
        models: distributions,
        period,
    })
}