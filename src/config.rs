use crate::types::GatewayConfig;
use serde_yaml::Value;
use std::fs;
use std::path::Path;

const CONFIG_SEARCH_PATHS: &[&str] = &[
    "config.yaml",
    "/etc/pstep-gateway/config.yaml",
];

pub fn load_config() -> GatewayConfig {
    let config_path = find_config_path();

    if config_path.is_none() {
        eprintln!("❌ 未找到配置文件！");
        eprintln!("   请创建 config.yaml（参考 config.yaml.template）");
        eprintln!("   或设置 CONFIG_PATH 环境变量");
        std::process::exit(1);
    }

    let config_path = config_path.unwrap();
    println!("📄 加载配置: {}", config_path.display());

    let raw = fs::read_to_string(&config_path).expect("无法读取配置文件");
    let mut parsed: Value = serde_yaml::from_str(&raw).expect("配置文件解析失败");

    migrate_legacy(&mut parsed);

    let resolved = resolve_env_vars(&parsed);
    let config: GatewayConfig = serde_yaml::from_value(resolved).expect("配置结构不匹配");

    validate_config(&config);
    config
}

pub fn get_config_path() -> Option<std::path::PathBuf> {
    find_config_path()
}

pub fn save_config(config: &GatewayConfig) -> Result<(), String> {
    let config_path = find_config_path()
        .ok_or_else(|| "无法找到配置文件路径".to_string())?;

    let yaml = serde_yaml::to_string(config)
        .map_err(|e| format!("序列化配置失败: {}", e))?;

    fs::write(&config_path, yaml)
        .map_err(|e| format!("写入配置文件失败: {}", e))?;

    // 限制文件权限为 0600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&config_path)
            .map_err(|e| format!("stat 配置失败: {}", e))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&config_path, perms)
            .map_err(|e| format!("chmod 600 失败: {}", e))?;
    }

    Ok(())
}

fn find_config_path() -> Option<std::path::PathBuf> {
    if let Ok(env_path) = std::env::var("CONFIG_PATH") {
        let path = Path::new(&env_path);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }

    for path_str in CONFIG_SEARCH_PATHS {
        let path = Path::new(path_str);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }

    None
}

/// 旧 config → 新 config 的就地迁移。
///
/// 旧结构：
/// ```yaml
/// upstreams:
///   anthropic: { type: anthropic, base_url: ..., api_key: ... }
/// models:
///   claude-sonnet:
///     upstream: anthropic
///     model: claude-3-5-sonnet-20241022
///     fallback_chain: [mimo, gpt-4o]   # 或旧字段 fallback
///     metadata: { status: active, ... }
/// ```
///
/// 新结构：
/// ```yaml
/// models:
///   claude-sonnet:
///     type: anthropic
///     base_url: ...
///     api_key: ...
///     model: claude-3-5-sonnet-20241022
///     fallback_policy: legacy_chain
/// fallback_policies:
///   legacy_chain: { chain: [...] }
/// ```
fn migrate_legacy(value: &mut Value) {
    // 提取 upstreams（之后删除）
    let legacy_upstreams = value
        .as_mapping_mut()
        .and_then(|m| m.remove(Value::from("upstreams")));

    if value.get("models").is_none() {
        return;
    }

    let upstreams_map = legacy_upstreams
        .as_ref()
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();

    let models = value
        .as_mapping_mut()
        .and_then(|m| m.get_mut(Value::from("models")))
        .and_then(|v| v.as_mapping_mut());

    let Some(models) = models else { return };

    // 收集需要创建的 fallback_policies（从 model.fallback_chain 转过来）
    let mut new_policies = serde_yaml::Mapping::new();
    let mut policy_counter: usize = 0;

    for (id, model_val) in models.iter_mut() {
        let Some(model_map) = model_val.as_mapping_mut() else {
            continue;
        };

        // 1. 旧 upstream → 新 4 字段
        let legacy_upstream_id = model_map
            .remove(Value::from("upstream"))
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            });

        if let Some(uid) = &legacy_upstream_id {
            if let Some(upstream_def) = upstreams_map.get(Value::from(uid.as_str())) {
                if let Some(upstream_map) = upstream_def.as_mapping() {
                    // 拷贝 type / base_url / api_key（保留 model_map 已有值优先）
                    for key in ["type", "base_url", "api_key"] {
                        if !model_map.contains_key(Value::from(key)) {
                            if let Some(v) = upstream_map.get(Value::from(key)) {
                                model_map.insert(Value::from(key), v.clone());
                            }
                        }
                    }
                }
            }
        }

        // 2. 旧 fallback_chain → 新 fallback_policy
        let legacy_chain = model_map
            .remove(Value::from("fallback_chain"))
            .and_then(|v| match v {
                Value::Sequence(s) => Some(s),
                _ => None,
            })
            .unwrap_or_default();

        let legacy_fallback = model_map
            .remove(Value::from("fallback"))
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            });

        // 把旧 fallback 拼到 chain 末尾
        let mut chain: Vec<Value> = legacy_chain.clone();
        if let Some(lf) = &legacy_fallback {
            if !chain.iter().any(|n| {
                n.as_mapping()
                    .and_then(|m| m.get(Value::from("model")))
                    .and_then(|v| v.as_str())
                    == Some(lf.as_str())
            }) {
                chain.push(serde_yaml::to_value(crate::types::ChainNodeConfig {
                    upstream: String::new(),
                    model: lf.clone(),
                })
                .unwrap_or(Value::Null));
            }
        }

        if !chain.is_empty() {
            // 提取 chain 中的 model 字段，构造策略
            let chain_nodes: Vec<crate::types::ChainNodeConfig> = chain
                .iter()
                .filter_map(|n| {
                    serde_yaml::from_value(n.clone()).ok()
                })
                .collect();

            if !chain_nodes.is_empty() {
                policy_counter += 1;
                let policy_id = format!("legacy_chain_{}", policy_counter);
                let policy = crate::types::FallbackPolicyConfig {
                    description: Some(format!("从 {} 的旧 fallback_chain 迁移", id.as_str().unwrap_or("?"))),
                    enabled: true,
                    chain: chain_nodes,
                };
                new_policies.insert(
                    Value::from(policy_id.clone()),
                    serde_yaml::to_value(policy).unwrap_or(Value::Null),
                );
                model_map.insert(
                    Value::from("fallback_policy"),
                    Value::from(policy_id),
                );
            }
        }

        // 3. metadata 兼容：旧 `status: 'active'` 字符串 → enum 已由 serde 处理
        //    旧字段（reasoning / input / context_window）由 ModelMetadata 内部的 _legacy_* 字段承接

        // 4. 如果是 model 自身没有 type/base_url/api_key 且没有 legacy_upstream，赋默认
        if !model_map.contains_key(Value::from("type")) {
            model_map.insert(Value::from("type"), Value::from("openai"));
        }
        if !model_map.contains_key(Value::from("base_url")) {
            model_map.insert(Value::from("base_url"), Value::from(""));
        }
        if !model_map.contains_key(Value::from("api_key")) {
            model_map.insert(Value::from("api_key"), Value::from(""));
        }
        if !model_map.contains_key(Value::from("model")) {
            // 兜底：使用 model id 作为上游 model
            if let Some(id_str) = id.as_str() {
                model_map.insert(Value::from("model"), Value::from(id_str));
            }
        }
    }

    // 5. 合并 legacy fallback_policies 与新生成的
    if !new_policies.is_empty() {
        let existing = value
            .as_mapping_mut()
            .and_then(|m| m.get_mut(Value::from("fallback_policies")))
            .and_then(|v| v.as_mapping_mut());
        match existing {
            Some(existing_map) => {
                for (k, v) in new_policies {
                    existing_map.entry(k).or_insert(v);
                }
            }
            None => {
                if let Some(top) = value.as_mapping_mut() {
                    top.insert(
                        Value::from("fallback_policies"),
                        Value::Mapping(new_policies),
                    );
                }
            }
        }
    }
}

fn resolve_env_vars(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            let resolved = resolve_env_refs(s);
            Value::String(resolved)
        }
        Value::Mapping(m) => {
            let mut new_map = serde_yaml::Mapping::new();
            for (k, v) in m {
                new_map.insert(k.clone(), resolve_env_vars(&v));
            }
            Value::Mapping(new_map)
        }
        Value::Sequence(seq) => {
            Value::Sequence(seq.iter().map(resolve_env_vars).collect())
        }
        _ => value.clone(),
    }
}

fn resolve_env_refs(s: &str) -> String {
    let mut result = s.to_string();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            if j < bytes.len() {
                let env_var = String::from_utf8_lossy(&bytes[start..j]);
                let env_value = std::env::var(env_var.as_ref()).unwrap_or_else(|_| {
                    eprintln!("⚠️  环境变量 {} 未设置", env_var);
                    String::new()
                });
                let pattern = format!("${{{}}}", env_var);
                result = result.replace(&pattern, &env_value);
                i = j + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    result
}

fn validate_config(config: &GatewayConfig) {
    if config.models.is_empty() {
        eprintln!("❌ 配置错误：至少需要一个 model");
        std::process::exit(1);
    }

    for (name, route) in &config.models {
        if route.api_key.is_empty() {
            eprintln!("❌ 模型 \"{}\" 未设置 api_key", name);
            std::process::exit(1);
        }
        if let Some(policy_id) = &route.fallback_policy {
            if !config.fallback_policies.contains_key(policy_id) {
                eprintln!(
                    "❌ 模型 \"{}\" 引用了不存在的 fallback_policy \"{}\"",
                    name, policy_id
                );
                std::process::exit(1);
            }
        }
    }

    for (key_id, key_cfg) in &config.client_api_keys {
        if let Some(policy_id) = &key_cfg.fallback_policy {
            if !config.fallback_policies.contains_key(policy_id) {
                eprintln!(
                    "❌ 客户端 key \"{}\" 引用了不存在的 fallback_policy \"{}\"",
                    key_id, policy_id
                );
                std::process::exit(1);
            }
        }
    }
}
