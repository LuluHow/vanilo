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
    // security headers (empty string = don't send)
    pub content_security_policy: String,
    pub strict_transport_security: String,
    pub x_frame_options: String,
    pub referrer_policy: String,
    pub permissions_policy: String,
    pub cross_origin_opener_policy: String,
    pub cross_origin_resource_policy: String,
    // proxy
    pub trusted_proxy: Option<String>,
    // build
    pub minify_js: bool,
    // webhook
    pub webhook_path: Option<String>,
    pub webhook_secret: Option<String>,
    pub webhook_rate_limit: usize,
    pub webhook_rate_window: u64,
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
            content_security_policy: "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'".into(),
            strict_transport_security: "max-age=63072000; includeSubDomains".into(),
            x_frame_options: "DENY".into(),
            referrer_policy: "strict-origin-when-cross-origin".into(),
            permissions_policy: "camera=(), microphone=(), geolocation=()".into(),
            cross_origin_opener_policy: "same-origin".into(),
            cross_origin_resource_policy: "same-origin".into(),
            trusted_proxy: None,
            minify_js: false,
            webhook_path: None,
            webhook_secret: None,
            webhook_rate_limit: 5,
            webhook_rate_window: 60,
        }
    }
}

impl Config {
    /// Builds the security headers block for HTTP responses.
    /// Each non-empty header gets a line. X-Content-Type-Options: nosniff is always sent.
    pub fn security_headers(&self) -> String {
        let mut h = String::with_capacity(512);
        let headers: &[(&str, &str)] = &[
            ("Content-Security-Policy", &self.content_security_policy),
            ("Strict-Transport-Security", &self.strict_transport_security),
            ("X-Frame-Options", &self.x_frame_options),
            ("Referrer-Policy", &self.referrer_policy),
            ("Permissions-Policy", &self.permissions_policy),
            ("Cross-Origin-Opener-Policy", &self.cross_origin_opener_policy),
            ("Cross-Origin-Resource-Policy", &self.cross_origin_resource_policy),
        ];
        for (name, value) in headers {
            if !value.is_empty() {
                h.push_str(name);
                h.push_str(": ");
                // Strip CR/LF/null to prevent header injection
                for c in value.chars() {
                    if c != '\r' && c != '\n' && c != '\0' {
                        h.push(c);
                    }
                }
                h.push_str("\r\n");
            }
        }
        // Always on, not configurable
        h.push_str("X-Content-Type-Options: nosniff\r\n");
        h
    }
}

/// Loads config with priority: env vars > vanilo.toml > defaults.
pub fn load() -> Config {
    let mut config = Config::default();

    if let Ok(content) = fs::read_to_string("vanilo.toml") {
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

        // Security headers — override individual headers, empty string disables
        if let Some(s) = v.get("content_security_policy") { config.content_security_policy = s.clone(); }
        if let Some(s) = v.get("strict_transport_security") { config.strict_transport_security = s.clone(); }
        if let Some(s) = v.get("x_frame_options") { config.x_frame_options = s.clone(); }
        if let Some(s) = v.get("referrer_policy") { config.referrer_policy = s.clone(); }
        if let Some(s) = v.get("permissions_policy") { config.permissions_policy = s.clone(); }
        if let Some(s) = v.get("cross_origin_opener_policy") { config.cross_origin_opener_policy = s.clone(); }
        if let Some(s) = v.get("cross_origin_resource_policy") { config.cross_origin_resource_policy = s.clone(); }

        // Proxy
        if let Some(s) = v.get("trusted_proxy") {
            let s = s.trim().to_string();
            if !s.is_empty() {
                config.trusted_proxy = Some(s);
            }
        }

        // Build
        if let Some(s) = v.get("minify_js") {
            config.minify_js = s == "true" || s == "1";
        }

        // Webhook
        if let Some(s) = v.get("webhook_path") {
            let s = s.trim().to_string();
            if !s.is_empty() {
                config.webhook_path = Some(if s.starts_with('/') { s } else { format!("/{s}") });
            }
        }
        if let Some(s) = v.get("webhook_secret") {
            let s = s.trim().to_string();
            if !s.is_empty() {
                config.webhook_secret = Some(s);
            }
        }
        if let Some(n) = v.get("webhook_rate_limit").and_then(|s| s.parse().ok()) {
            config.webhook_rate_limit = n;
        }
        if let Some(n) = v.get("webhook_rate_window").and_then(|s| s.parse().ok()) {
            config.webhook_rate_window = n;
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
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            // Parse quoted values (preserving # inside quotes)
            let (value, _) = if value.starts_with('"') {
                if let Some(end) = value[1..].find('"') {
                    (&value[1..1 + end], &value[2 + end..])
                } else {
                    (value, "")
                }
            } else if value.starts_with('\'') {
                if let Some(end) = value[1..].find('\'') {
                    (&value[1..1 + end], &value[2 + end..])
                } else {
                    (value, "")
                }
            } else {
                // Unquoted: strip inline comment
                let v = value.split('#').next().unwrap_or(value).trim();
                (v, "")
            };
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

    #[test]
    fn parse_toml_hash_in_quoted_string() {
        let input = r#"csp = "default-src 'self'; script-src 'self' #hash""#;
        let values = parse_toml(input);
        assert_eq!(
            values.get("csp").unwrap(),
            "default-src 'self'; script-src 'self' #hash"
        );
    }

    #[test]
    fn parse_toml_unquoted_inline_comment() {
        let input = "timeout = 10 # seconds\n";
        let values = parse_toml(input);
        assert_eq!(values.get("timeout").unwrap(), "10");
    }

    #[test]
    fn security_headers_default() {
        let config = Config::default();
        let h = config.security_headers();
        assert!(h.contains("Content-Security-Policy: default-src 'self'"));
        assert!(h.contains("Strict-Transport-Security: max-age=63072000"));
        assert!(h.contains("X-Content-Type-Options: nosniff"));
        assert!(h.contains("Cross-Origin-Opener-Policy: same-origin"));
    }

    #[test]
    fn security_headers_override_and_disable() {
        let mut config = Config::default();
        config.content_security_policy = "default-src 'self' https://js.stripe.com".into();
        config.strict_transport_security = String::new(); // disable HSTS
        let h = config.security_headers();
        assert!(h.contains("https://js.stripe.com"));
        assert!(!h.contains("Strict-Transport-Security"));
        // nosniff is always present
        assert!(h.contains("X-Content-Type-Options: nosniff"));
    }

    #[test]
    fn security_headers_strips_injection() {
        let mut config = Config::default();
        config.x_frame_options = "DENY\r\nInjected: bad".into();
        let h = config.security_headers();
        assert!(h.contains("X-Frame-Options: DENYInjected: bad"));
        assert!(!h.contains("\r\nInjected"));
    }
}
