use std::collections::HashMap;
use std::fs;

#[derive(Clone)]
pub struct Config {
    // network
    pub port: u16,
    pub host: String,
    // server limits
    pub max_body: usize,        // bytes
    pub max_connections: usize,
    pub rate_limit: usize,      // requests per window
    pub rate_window: u64,       // seconds
    // JS runtime
    pub timeout: u64,           // seconds
    pub memory: usize,          // bytes
    pub fetch_timeout: u64,     // seconds
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 3000,
            host: "127.0.0.1".to_string(),
            max_body: 1024 * 1024,       // 1 MB
            max_connections: 128,
            rate_limit: 60,
            rate_window: 60,
            timeout: 5,
            memory: 32 * 1024 * 1024,    // 32 MB
            fetch_timeout: 10,
        }
    }
}

/// Loads config with priority: env vars > simple.toml > defaults.
pub fn load() -> Config {
    let mut config = Config::default();

    if let Ok(content) = fs::read_to_string("simple.toml") {
        let v = parse_toml(&content);

        if let Some(port) = v.get("port").and_then(|s| s.parse().ok()) {
            config.port = port;
        }
        if let Some(host) = v.get("host") {
            config.host = host.clone();
        }
        if let Some(mb) = v.get("max_body").and_then(|s| s.parse::<usize>().ok()) {
            config.max_body = mb * 1024 * 1024;
        }
        if let Some(n) = v.get("max_connections").and_then(|s| s.parse().ok()) {
            config.max_connections = n;
        }
        if let Some(n) = v.get("rate_limit").and_then(|s| s.parse().ok()) {
            config.rate_limit = n;
        }
        if let Some(n) = v.get("rate_window").and_then(|s| s.parse().ok()) {
            config.rate_window = n;
        }
        if let Some(n) = v.get("timeout").and_then(|s| s.parse().ok()) {
            config.timeout = n;
        }
        if let Some(mb) = v.get("memory").and_then(|s| s.parse::<usize>().ok()) {
            config.memory = mb * 1024 * 1024;
        }
        if let Some(n) = v.get("fetch_timeout").and_then(|s| s.parse().ok()) {
            config.fetch_timeout = n;
        }
    }

    // Env vars override config file
    if let Ok(v) = std::env::var("PORT") {
        if let Ok(p) = v.parse() {
            config.port = p;
        }
    }
    if let Ok(v) = std::env::var("HOST") {
        config.host = v;
    }

    config
}

fn parse_toml(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        // Strip inline comments
        let line = line.split('#').next().unwrap_or(line).trim();
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_toml_basic() {
        let input = "port = 8080\nhost = \"0.0.0.0\"\n# comment\n";
        let values = parse_toml(input);
        assert_eq!(values.get("port").unwrap(), "8080");
        assert_eq!(values.get("host").unwrap(), "0.0.0.0");
    }

    #[test]
    fn parse_toml_inline_comments() {
        let input = "timeout = 10 # seconds\nmemory = 64 # MB\n";
        let values = parse_toml(input);
        assert_eq!(values.get("timeout").unwrap(), "10");
        assert_eq!(values.get("memory").unwrap(), "64");
    }

    #[test]
    fn parse_toml_skips_sections_and_comments() {
        let input = "[server]\n# port config\nport = 3000\n";
        let values = parse_toml(input);
        assert_eq!(values.get("port").unwrap(), "3000");
    }
}
