use std::fs;
use serde::Deserialize;

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
    // CORS for /api/* endpoints
    pub api_cors: String,
    // proxy
    pub trusted_proxy: Option<String>,
    // build
    pub minify_js: bool,
    pub build_dir: Option<String>,
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
            api_cors: String::new(),
            trusted_proxy: None,
            minify_js: false,
            build_dir: None,
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

// ---------------------------------------------------------------------------
// Raw TOML deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct RawConfig {
    port: Option<u16>,
    host: Option<String>,
    max_body: Option<usize>,
    max_connections: Option<usize>,
    rate_limit: Option<usize>,
    rate_window: Option<u64>,
    timeout: Option<u64>,
    memory: Option<usize>,
    fetch_timeout: Option<u64>,
    api_cors: Option<String>,
    trusted_proxy: Option<String>,
    minify_js: Option<bool>,
    build_dir: Option<String>,
    // Flat legacy keys (backward compat)
    content_security_policy: Option<String>,
    strict_transport_security: Option<String>,
    x_frame_options: Option<String>,
    referrer_policy: Option<String>,
    permissions_policy: Option<String>,
    cross_origin_opener_policy: Option<String>,
    cross_origin_resource_policy: Option<String>,
    webhook_path: Option<String>,
    webhook_secret: Option<String>,
    webhook_rate_limit: Option<usize>,
    webhook_rate_window: Option<u64>,
    // Nested tables
    security_headers: Option<SecurityHeaders>,
    webhook: Option<Webhook>,
}

#[derive(Deserialize, Default)]
struct SecurityHeaders {
    content_security_policy: Option<String>,
    strict_transport_security: Option<String>,
    x_frame_options: Option<String>,
    referrer_policy: Option<String>,
    permissions_policy: Option<String>,
    cross_origin_opener_policy: Option<String>,
    cross_origin_resource_policy: Option<String>,
}

#[derive(Deserialize, Default)]
struct Webhook {
    webhook_path: Option<String>,
    webhook_secret: Option<String>,
    webhook_rate_limit: Option<usize>,
    webhook_rate_window: Option<u64>,
}

/// Loads config with priority: env vars > vanilo.toml > defaults.
pub fn load() -> Config {
    let mut config = if let Ok(content) = fs::read_to_string("vanilo.toml") {
        match load_from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("vanilo.toml parse error: {e}");
                Config::default()
            }
        }
    } else {
        Config::default()
    };

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

/// Parses a TOML string into a Config.
fn load_from_str(content: &str) -> Result<Config, String> {
    let mut config = Config::default();
    let raw: RawConfig = toml::from_str(content).map_err(|e| e.to_string())?;

    if let Some(v) = raw.port { config.port = v; }
    if let Some(v) = raw.host { config.host = v; }
    if let Some(v) = raw.max_body { config.max_body = v * 1024 * 1024; }
    if let Some(v) = raw.max_connections { config.max_connections = v; }
    if let Some(v) = raw.rate_limit { config.rate_limit = v; }
    if let Some(v) = raw.rate_window { config.rate_window = v; }
    if let Some(v) = raw.timeout { config.timeout = v; }
    if let Some(v) = raw.memory { config.memory = v * 1024 * 1024; }
    if let Some(v) = raw.fetch_timeout { config.fetch_timeout = v; }
    if let Some(v) = raw.api_cors { config.api_cors = v; }
    if let Some(v) = raw.minify_js { config.minify_js = v; }
    if let Some(v) = raw.build_dir {
        let v = v.trim().to_string();
        if !v.is_empty() { config.build_dir = Some(v); }
    }

    if let Some(v) = raw.trusted_proxy {
        let v = v.trim().to_string();
        if !v.is_empty() { config.trusted_proxy = Some(v); }
    }

    // Security headers: flat keys first, then nested table overrides
    if let Some(v) = raw.content_security_policy { config.content_security_policy = v; }
    if let Some(v) = raw.strict_transport_security { config.strict_transport_security = v; }
    if let Some(v) = raw.x_frame_options { config.x_frame_options = v; }
    if let Some(v) = raw.referrer_policy { config.referrer_policy = v; }
    if let Some(v) = raw.permissions_policy { config.permissions_policy = v; }
    if let Some(v) = raw.cross_origin_opener_policy { config.cross_origin_opener_policy = v; }
    if let Some(v) = raw.cross_origin_resource_policy { config.cross_origin_resource_policy = v; }

    if let Some(sh) = raw.security_headers {
        if let Some(v) = sh.content_security_policy { config.content_security_policy = v; }
        if let Some(v) = sh.strict_transport_security { config.strict_transport_security = v; }
        if let Some(v) = sh.x_frame_options { config.x_frame_options = v; }
        if let Some(v) = sh.referrer_policy { config.referrer_policy = v; }
        if let Some(v) = sh.permissions_policy { config.permissions_policy = v; }
        if let Some(v) = sh.cross_origin_opener_policy { config.cross_origin_opener_policy = v; }
        if let Some(v) = sh.cross_origin_resource_policy { config.cross_origin_resource_policy = v; }
    }

    // Webhook: flat keys first, then nested table overrides
    if let Some(v) = raw.webhook_path {
        let v = v.trim().to_string();
        if !v.is_empty() {
            config.webhook_path = Some(if v.starts_with('/') { v } else { format!("/{v}") });
        }
    }
    if let Some(v) = raw.webhook_secret {
        let v = v.trim().to_string();
        if !v.is_empty() { config.webhook_secret = Some(v); }
    }
    if let Some(v) = raw.webhook_rate_limit { config.webhook_rate_limit = v; }
    if let Some(v) = raw.webhook_rate_window { config.webhook_rate_window = v; }

    if let Some(wh) = raw.webhook {
        if let Some(v) = wh.webhook_path {
            let v = v.trim().to_string();
            if !v.is_empty() {
                config.webhook_path = Some(if v.starts_with('/') { v } else { format!("/{v}") });
            }
        }
        if let Some(v) = wh.webhook_secret {
            let v = v.trim().to_string();
            if !v.is_empty() { config.webhook_secret = Some(v); }
        }
        if let Some(v) = wh.webhook_rate_limit { config.webhook_rate_limit = v; }
        if let Some(v) = wh.webhook_rate_window { config.webhook_rate_window = v; }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_flat_basic() {
        let cfg = load_from_str("port = 8080\nhost = \"0.0.0.0\"\n").unwrap();
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.host, "0.0.0.0");
    }

    #[test]
    fn config_nested_security_headers() {
        let input = r#"
[security_headers]
content_security_policy = "default-src 'none'"
x_frame_options = "SAMEORIGIN"
"#;
        let cfg = load_from_str(input).unwrap();
        assert_eq!(cfg.content_security_policy, "default-src 'none'");
        assert_eq!(cfg.x_frame_options, "SAMEORIGIN");
    }

    #[test]
    fn config_nested_webhook() {
        let input = r#"
[webhook]
webhook_path = "/_hook/deploy"
webhook_secret = "s3cret"
webhook_rate_limit = 10
"#;
        let cfg = load_from_str(input).unwrap();
        assert_eq!(cfg.webhook_path.as_deref(), Some("/_hook/deploy"));
        assert_eq!(cfg.webhook_secret.as_deref(), Some("s3cret"));
        assert_eq!(cfg.webhook_rate_limit, 10);
    }

    #[test]
    fn config_flat_and_nested_merge() {
        let input = r#"
content_security_policy = "flat-value"

[security_headers]
content_security_policy = "nested-value"
"#;
        let cfg = load_from_str(input).unwrap();
        // Nested table wins over flat key
        assert_eq!(cfg.content_security_policy, "nested-value");
    }

    #[test]
    fn config_defaults() {
        let cfg = load_from_str("").unwrap();
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.max_body, 1024 * 1024);
        assert_eq!(cfg.memory, 32 * 1024 * 1024);
        assert!(!cfg.minify_js);
        assert!(cfg.webhook_path.is_none());
    }

    #[test]
    fn config_unit_conversion() {
        let input = "max_body = 2\nmemory = 64\n";
        let cfg = load_from_str(input).unwrap();
        assert_eq!(cfg.max_body, 2 * 1024 * 1024);
        assert_eq!(cfg.memory, 64 * 1024 * 1024);
    }

    #[test]
    fn config_boolean() {
        let cfg = load_from_str("minify_js = true\n").unwrap();
        assert!(cfg.minify_js);
    }

    #[test]
    fn config_multiline_string() {
        let input = r#"
content_security_policy = """
default-src 'self';
script-src 'self' 'unsafe-inline'
"""
"#;
        let cfg = load_from_str(input).unwrap();
        assert!(cfg.content_security_policy.contains("default-src 'self'"));
        assert!(cfg.content_security_policy.contains("script-src"));
    }

    #[test]
    fn config_array_in_toml() {
        // Verify arrays parse without error even if not used by Config fields
        let input = r#"
port = 3000
# Arrays should not cause parse errors
"#;
        let cfg = load_from_str(input).unwrap();
        assert_eq!(cfg.port, 3000);
    }

    #[test]
    fn config_invalid_toml() {
        let result = load_from_str("port = [invalid\n");
        assert!(result.is_err());
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

    #[test]
    fn config_api_cors() {
        let cfg = load_from_str("api_cors = \"https://example.com\"\n").unwrap();
        assert_eq!(cfg.api_cors, "https://example.com");
    }

    #[test]
    fn api_cors_default_empty() {
        let config = Config::default();
        assert!(config.api_cors.is_empty());
    }

    #[test]
    fn security_headers_does_not_include_cors() {
        let mut config = Config::default();
        config.api_cors = "*".into();
        let h = config.security_headers();
        assert!(!h.contains("Access-Control"));
    }

    #[test]
    fn config_webhook_path_gets_slash() {
        let cfg = load_from_str("webhook_path = \"hook/deploy\"\n").unwrap();
        assert_eq!(cfg.webhook_path.as_deref(), Some("/hook/deploy"));
    }

    #[test]
    fn config_hash_in_quoted_string() {
        let input = r#"content_security_policy = "default-src 'self'; script-src 'self' #hash""#;
        let cfg = load_from_str(input).unwrap();
        assert_eq!(
            cfg.content_security_policy,
            "default-src 'self'; script-src 'self' #hash"
        );
    }
}
