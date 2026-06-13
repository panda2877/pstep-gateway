use crate::types::GatewayConfig;
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
    let parsed: serde_yaml::Value = serde_yaml::from_str(&raw).expect("配置文件解析失败");

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

fn resolve_env_vars(value: &serde_yaml::Value) -> serde_yaml::Value {
    match value {
        serde_yaml::Value::String(s) => {
            let resolved = resolve_env_refs(s);
            serde_yaml::Value::String(resolved)
        }
        serde_yaml::Value::Mapping(m) => {
            let mut new_map = serde_yaml::Mapping::new();
            for (k, v) in m {
                new_map.insert(k.clone(), resolve_env_vars(&v));
            }
            serde_yaml::Value::Mapping(new_map)
        }
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.iter().map(resolve_env_vars).collect())
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
    if config.upstreams.is_empty() {
        eprintln!("❌ 配置错误：至少需要一个 upstream");
        std::process::exit(1);
    }

    if config.models.is_empty() {
        eprintln!("❌ 配置错误：至少需要一个 model");
        std::process::exit(1);
    }

    for (name, route) in &config.models {
        if !config.upstreams.contains_key(&route.upstream) {
            eprintln!("❌ 模型 \"{}\" 引用了不存在的 upstream \"{}\"", name, route.upstream);
            std::process::exit(1);
        }
        if let Some(fallback) = &route.fallback {
            if !config.models.contains_key(fallback) {
                eprintln!("❌ 模型 \"{}\" 的 fallback \"{}\" 不存在", name, fallback);
                std::process::exit(1);
            }
        }
    }

    for (name, upstream) in &config.upstreams {
        if upstream.api_key.is_empty() {
            eprintln!("❌ upstream \"{}\" 未设置 api_key", name);
            std::process::exit(1);
        }
    }
}