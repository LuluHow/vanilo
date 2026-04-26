use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use flate2::write::GzEncoder;
use flate2::Compression;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::builder;
use crate::component::{self, Component};
use crate::config::Config;
use crate::content;
use crate::functions;
use crate::parser;

const DIST_DIR: &str = "dist";
const MAX_HEADERS: usize = 65536; // 64KB


/// File extensions that must never be served.
const BLOCKED_EXTENSIONS: &[&str] = &[
    ".db", ".sqlite", ".sqlite3",
    ".env",
    ".key", ".pem", ".p12", ".pfx",
    ".sh", ".bash",
    ".sql",
    ".log",
    ".envrc", ".htaccess",
    ".bak", ".swp", ".swo",
];

const BLOCKED_FILENAMES: &[&str] = &[
    ".ds_store", ".gitignore", ".gitmodules",
];

fn is_blocked_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    if BLOCKED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }
    let filename = lower.rsplit('/').next().unwrap_or(&lower);
    // Block .env.* variants (e.g., .env.local, .env.production)
    if filename.starts_with(".env.") {
        return true;
    }
    BLOCKED_FILENAMES.iter().any(|name| filename == *name)
}

// ---------------------------------------------------------------------------
// Server-side rendering cache
// ---------------------------------------------------------------------------

const SERVER_CACHE_MAX: usize = 1024;

struct ServerCache {
    entries: HashMap<String, (String, Instant, u64)>,
}

impl ServerCache {
    fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Returns cached HTML if the entry exists and is not expired. Purges expired entries lazily.
    fn get(&mut self, key: &str) -> Option<String> {
        if let Some((html, created, ttl)) = self.entries.get(key) {
            if created.elapsed().as_secs() < *ttl {
                return Some(html.clone());
            }
            // Expired — remove
        } else {
            return None;
        }
        self.entries.remove(key);
        None
    }

    /// Stores rendered HTML with a TTL. Evicts oldest entry if at capacity.
    fn put(&mut self, key: String, html: String, ttl: u64) {
        if self.entries.len() >= SERVER_CACHE_MAX {
            // Evict oldest entry
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, created, _))| *created)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(key, (html, Instant::now(), ttl));
    }
}

/// Ensures the active connection counter is decremented even on panic.
struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn serve(config: Config, build_lock: Arc<Mutex<()>>) -> Result<(), String> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("serving on http://{addr}");

    let config = Arc::new(config);
    let rate_map: Arc<Mutex<HashMap<String, (usize, Instant)>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let active = Arc::new(AtomicUsize::new(0));
    let webhook_rate: Arc<Mutex<(usize, Instant)>> =
        Arc::new(Mutex::new((0, Instant::now())));
    let components: Arc<HashMap<String, Component>> = Arc::new(
        component::load_components(std::path::Path::new("components")).unwrap_or_default(),
    );
    let server_cache: Arc<Mutex<ServerCache>> = Arc::new(Mutex::new(ServerCache::new()));

    if config.webhook_path.is_some() && config.webhook_secret.is_some() {
        println!("webhook enabled on {}", config.webhook_path.as_ref().unwrap());
    }

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("connection error: {e}");
                continue;
            }
        };

        if active.load(Ordering::Relaxed) >= config.max_connections {
            let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 15\r\nConnection: close\r\n\r\nserver too busy";
            let _ = stream.write_all(resp.as_bytes());
            continue;
        }

        let rate = rate_map.clone();
        let cfg = config.clone();
        let wh_rate = webhook_rate.clone();
        let wh_lock = build_lock.clone();
        let comps = components.clone();
        let scache = server_cache.clone();
        active.fetch_add(1, Ordering::Relaxed);
        let guard = ConnectionGuard(active.clone());

        thread::spawn(move || {
            let _guard = guard;
            handle_connection(&mut stream, &rate, &cfg, &wh_rate, &wh_lock, &comps, &scache);
        });
    }

    Ok(())
}

fn handle_connection(
    stream: &mut TcpStream,
    rate_map: &Mutex<HashMap<String, (usize, Instant)>>,
    config: &Config,
    webhook_rate: &Mutex<(usize, Instant)>,
    build_lock: &Arc<Mutex<()>>,
    components: &HashMap<String, Component>,
    server_cache: &Mutex<ServerCache>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    // Read headers until \r\n\r\n (up to MAX_HEADERS)
    let mut header_buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break None,
            Ok(n) => n,
        };
        header_buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(pos);
        }
        if header_buf.len() > MAX_HEADERS {
            break None;
        }
    };
    let header_end = match header_end {
        Some(pos) => pos,
        None => return,
    };

    let headers_str = String::from_utf8_lossy(&header_buf[..header_end]).into_owned();
    let first_line = match headers_str.lines().next() {
        Some(line) => line,
        None => return,
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_path = parts.next().unwrap_or("/");

    // Reject Transfer-Encoding — chunked is not supported, prevents request smuggling
    let has_transfer_encoding = headers_str
        .lines()
        .any(|line| line.get(..18).is_some_and(|s| s.eq_ignore_ascii_case("transfer-encoding:")));
    if has_transfer_encoding {
        let resp = "HTTP/1.1 501 Not Implemented\r\nContent-Type: text/plain\r\nContent-Length: 32\r\nConnection: close\r\n\r\ntransfer-encoding not supported";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    // Parse Content-Length and read full body
    let content_length: usize = headers_str
        .lines()
        .find_map(|line| {
            if line.get(..15).is_some_and(|s| s.eq_ignore_ascii_case("content-length:")) {
                line[15..].trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let content_type_header: String = headers_str
        .lines()
        .find_map(|line| {
            if line.get(..13).is_some_and(|s| s.eq_ignore_ascii_case("content-type:")) {
                Some(line[13..].trim().to_lowercase())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let body_start = header_end + 4;
    let body_limit = content_length.min(config.max_body + 1);
    let mut body_buf: Vec<u8> = header_buf[body_start..].to_vec();
    while body_buf.len() < body_limit {
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body_buf.extend_from_slice(&chunk[..n]),
        }
    }
    body_buf.truncate(body_limit);
    let req_body = String::from_utf8_lossy(&body_buf).into_owned();

    // Path traversal protection: decode percent-encoding before checking
    let decoded_path = url_decode(raw_path.split('?').next().unwrap_or(raw_path));
    if decoded_path.contains("..") || decoded_path.contains('\0') {
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nforbidden";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    let sec = config.security_headers();

    // Webhook endpoint — only exists when both path and secret are configured
    if let (Some(hook_path), Some(hook_secret)) = (&config.webhook_path, &config.webhook_secret) {
        if decoded_path == hook_path.as_str() {
            if method != "POST" {
                let resp = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                return;
            }

            // Webhook-specific rate limiting
            {
                let mut wh = webhook_rate.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                if now.duration_since(wh.1).as_secs() >= config.webhook_rate_window {
                    *wh = (0, now);
                }
                wh.0 += 1;
                if wh.0 > config.webhook_rate_limit {
                    eprintln!("webhook: rate limited");
                    let resp = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: 12\r\nConnection: close\r\n\r\nrate limited";
                    let _ = stream.write_all(resp.as_bytes());
                    return;
                }
            }

            // Verify HMAC-SHA256 signature (X-Hub-Signature-256: sha256=...)
            let valid = match extract_header(&headers_str, "x-hub-signature-256") {
                Some(sig) => verify_webhook_signature(hook_secret, body_buf.as_slice(), &sig),
                None => {
                    eprintln!("webhook: rejected (missing signature)");
                    false
                }
            };

            if !valid {
                eprintln!("webhook: rejected (invalid signature)");
                let resp = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nforbidden";
                let _ = stream.write_all(resp.as_bytes());
                return;
            }

            // Respond immediately, rebuild in background
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
            let _ = stream.write_all(resp.as_bytes());

            let lock = build_lock.clone();
            thread::spawn(move || {
                let _guard = match lock.try_lock() {
                    Ok(g) => g,
                    Err(_) => {
                        eprintln!("webhook: rebuild already in progress, skipping");
                        return;
                    }
                };
                eprintln!("webhook: rebuilding...");
                match builder::build() {
                    Ok(()) => eprintln!("webhook: rebuild completed"),
                    Err(e) => eprintln!("webhook: rebuild failed: {e}"),
                }
            });
            return;
        }
    }

    if raw_path.starts_with("/api/") {
        // CORS preflight — respond fast, skip rate limiting and function execution
        if method == "OPTIONS" {
            let origin = extract_header(&headers_str, "origin");
            let cors = cors_headers(config, origin.as_deref());
            if cors.is_empty() {
                let resp = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            } else {
                let resp = format!(
                    "HTTP/1.1 204 No Content\r\n{cors}Content-Length: 0\r\n{sec}Connection: close\r\n\r\n"
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            return;
        }

        // Rate limiting by IP (trust X-Forwarded-For when behind a configured proxy)
        let ip = client_ip(stream, &headers_str, config);

        {
            let mut map = rate_map.lock().unwrap_or_else(|e| e.into_inner());

            // Prune stale entries
            if map.len() > 100 {
                let cutoff = Instant::now();
                map.retain(|_, (_, ts)| cutoff.duration_since(*ts).as_secs() < config.rate_window * 2);
            }

            let now = Instant::now();
            let entry = map.entry(ip).or_insert((0, now));
            if now.duration_since(entry.1).as_secs() >= config.rate_window {
                *entry = (0, now);
            }
            entry.0 += 1;

            if entry.0 > config.rate_limit {
                let resp = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: 12\r\nConnection: close\r\n\r\nrate limited";
                let _ = stream.write_all(resp.as_bytes());
                return;
            }
        }

        // Body size check
        if req_body.len() > config.max_body {
            let resp = "HTTP/1.1 413 Payload Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nConnection: close\r\n\r\npayload too large";
            let _ = stream.write_all(resp.as_bytes());
            return;
        }

        // Require application/json for body-bearing methods (prevents CORS preflight bypass)
        if matches!(method, "POST" | "PUT" | "PATCH")
            && content_length > 0
            && !content_type_header.starts_with("application/json")
        {
            let err_body = r#"{"error":"Content-Type must be application/json"}"#;
            let resp = format!(
                "HTTP/1.1 415 Unsupported Media Type\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{err_body}",
                err_body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            return;
        }

        // HEAD → execute as GET, suppress body in response
        let effective_method = if method == "HEAD" { "GET" } else { method };
        let req_headers = parse_request_headers(&headers_str);
        let (status, body, content_type) = handle_function(effective_method, raw_path, &req_body, req_headers, config);
        let content_type = sanitize_header_value(&content_type);
        let body_bytes = body.as_bytes();

        // CORS headers for the actual response
        let origin = extract_header(&headers_str, "origin");
        let cors = cors_headers(config, origin.as_deref());

        // On-the-fly compression for API responses (prefer brotli > gzip)
        let accept_br = accepts_encoding(&headers_str, "br");
        let accept_gz = accepts_encoding(&headers_str, "gzip");
        let should_compress = (accept_br || accept_gz)
            && is_compressible(&content_type)
            && body_bytes.len() > 256;

        // Build Vary header: combine Origin (CORS) + Accept-Encoding (compression)
        let vary = if cors.contains("Vary: Origin") && should_compress {
            "Vary: Origin, Accept-Encoding\r\n"
        } else if cors.contains("Vary: Origin") {
            "Vary: Origin\r\n"
        } else if should_compress {
            "Vary: Accept-Encoding\r\n"
        } else {
            ""
        };

        // Strip Vary from cors string since we handle it separately
        let cors_no_vary: String = cors.lines()
            .filter(|l| !l.starts_with("Vary:"))
            .map(|l| format!("{l}\r\n"))
            .collect();

        if should_compress {
            let (compressed, enc) = if accept_br {
                (brotli_compress_fast(body_bytes), "br")
            } else {
                (gzip_compress(body_bytes), "gzip")
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Encoding: {enc}\r\n{cors_no_vary}{sec}Cache-Control: no-store\r\n{vary}Connection: close\r\n\r\n",
                compressed.len()
            );
            let _ = stream.write_all(response.as_bytes());
            if method != "HEAD" {
                let _ = stream.write_all(&compressed);
            }
        } else {
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\n{cors_no_vary}{sec}Cache-Control: no-store\r\n{vary}Connection: close\r\n\r\n",
                len = body_bytes.len()
            );
            let _ = stream.write_all(response.as_bytes());
            if method != "HEAD" {
                let _ = stream.write_all(body_bytes);
            }
        }
    } else {
        // Static file serving with pre-compressed variants
        let file_path = decoded_path.split('?').next().unwrap_or(&decoded_path);
        let accept_br = accepts_encoding(&headers_str, "br");
        let accept_gz = accepts_encoding(&headers_str, "gzip");

        // For HTML pages, check if they contain <Server> blocks requiring SSR.
        // Read the raw (uncompressed) file first to check for markers.
        let (status, raw_body) = resolve_file_raw(file_path);
        let content_type = content_type_for_path(file_path);
        let is_html = content_type.starts_with("text/html");
        let has_server_blocks = is_html
            && status == "200 OK"
            && raw_body.as_ref().is_ok_and(|b| {
                std::str::from_utf8(b)
                    .is_ok_and(|s| s.contains(SERVER_MARKER_START))
            });

        if has_server_blocks {
            // Dynamic page: process server blocks, compress on-the-fly
            let raw_bytes = raw_body.unwrap();
            let html_str = String::from_utf8_lossy(&raw_bytes);
            let assembled = process_server_blocks(&html_str, components, server_cache, config);
            let assembled_bytes = assembled.as_bytes();

            let etag = compute_etag(assembled_bytes);
            let if_none_match = extract_header(&headers_str, "if-none-match");
            if if_none_match.as_deref() == Some(etag.as_str()) {
                let response = format!(
                    "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\n{sec}Cache-Control: no-cache\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(response.as_bytes());
                return;
            }

            // On-the-fly compression
            let (body_bytes, encoding): (Vec<u8>, Option<&str>) =
                if accept_br && assembled_bytes.len() > 256 {
                    (brotli_compress_fast(assembled_bytes), Some("br"))
                } else if accept_gz && assembled_bytes.len() > 256 {
                    (gzip_compress(assembled_bytes), Some("gzip"))
                } else {
                    (assembled_bytes.to_vec(), None)
                };

            let mut extra = String::new();
            if let Some(enc) = encoding {
                extra.push_str(&format!(
                    "Content-Encoding: {enc}\r\nVary: Accept-Encoding\r\n"
                ));
            }

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nETag: {etag}\r\n{extra}{sec}Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
                body_bytes.len()
            );
            let _ = stream.write_all(response.as_bytes());
            if method != "HEAD" {
                let _ = stream.write_all(&body_bytes);
            }
        } else {
            // Pure static page: serve normally with pre-compressed variants
            let (status, body, content_type, encoding) =
                resolve_file(file_path, accept_br, accept_gz);
            let cache = cache_control_for(file_path, content_type);

            let etag = compute_etag(&body);
            let if_none_match = extract_header(&headers_str, "if-none-match");
            if status == "200 OK" && if_none_match.as_deref() == Some(etag.as_str()) {
                let response = format!(
                    "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\n{sec}Cache-Control: {cache}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(response.as_bytes());
                return;
            }

            let mut extra = String::new();
            if let Some(enc) = encoding {
                extra.push_str(&format!(
                    "Content-Encoding: {enc}\r\nVary: Accept-Encoding\r\n"
                ));
            }

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nETag: {etag}\r\n{extra}{sec}Cache-Control: {cache}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            if method != "HEAD" {
                let _ = stream.write_all(&body);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server-side rendering of <Server> blocks
// ---------------------------------------------------------------------------

const SERVER_MARKER_START: &str = "<!--vanilo:server ";
const SERVER_MARKER_END: &str = "-->";

/// Processes `<!--vanilo:server ...-->` markers in the HTML, executing edge functions
/// and rendering components at request time.
fn process_server_blocks(
    html: &str,
    components: &HashMap<String, Component>,
    cache: &Mutex<ServerCache>,
    config: &Config,
) -> String {
    let mut result = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(start) = remaining.find(SERVER_MARKER_START) {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start..];

        let end = match remaining.find(SERVER_MARKER_END) {
            Some(p) => p + SERVER_MARKER_END.len(),
            None => {
                result.push_str(SERVER_MARKER_START);
                remaining = &remaining[SERVER_MARKER_START.len()..];
                continue;
            }
        };

        let marker = &remaining[..end];
        remaining = &remaining[end..];

        let rendered = match parse_server_marker(marker) {
            Some((fn_name, comp_name, ttl, params)) => {
                render_server_block(&fn_name, &comp_name, ttl, &params, components, cache, config)
            }
            None => {
                eprintln!("server block: malformed marker: {marker}");
                String::new()
            }
        };
        result.push_str(&rendered);
    }

    result.push_str(remaining);
    result
}

/// Parses a `<!--vanilo:server fn="X" comp="Y" [cache="N"] [params="..."]-->` marker.
/// Returns (function_name, component_name, optional_cache_ttl, params_query).
fn parse_server_marker(marker: &str) -> Option<(String, String, Option<u64>, String)> {
    let inner = marker
        .strip_prefix("<!--vanilo:server ")?
        .strip_suffix("-->")?
        .trim();

    let mut fn_name = None;
    let mut comp_name = None;
    let mut cache_ttl = None;
    let mut params = String::new();

    let mut rem = inner;
    while !rem.is_empty() {
        rem = rem.trim_start();
        if rem.is_empty() {
            break;
        }

        let eq_pos = rem.find('=')?;
        let attr = rem[..eq_pos].trim();
        rem = &rem[eq_pos + 1..];

        // Parse quoted value
        let quote = rem.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        rem = &rem[1..];
        let close = rem.find(quote)?;
        let val = &rem[..close];
        rem = &rem[close + 1..];

        match attr {
            "fn" => fn_name = Some(val.to_string()),
            "comp" => comp_name = Some(val.to_string()),
            "cache" => cache_ttl = val.parse::<u64>().ok(),
            "params" => params = val.to_string(),
            _ => {}
        }
    }

    Some((fn_name?, comp_name?, cache_ttl, params))
}

/// Renders a single server block: cache check → execute function → render components.
fn render_server_block(
    fn_name: &str,
    comp_name: &str,
    ttl: Option<u64>,
    params: &str,
    components: &HashMap<String, Component>,
    cache: &Mutex<ServerCache>,
    config: &Config,
) -> String {
    let cache_key = format!("{fn_name}:{comp_name}:{params}");
    let start = Instant::now();

    // Cache check
    if ttl.is_some() {
        if let Ok(mut c) = cache.lock() {
            if let Some(html) = c.get(&cache_key) {
                return html;
            }
        }
    }

    // Resolve and execute the edge function
    let api_path = format!("/api/{fn_name}");
    let file_path = match functions::resolve_function(&api_path) {
        Some(p) => p,
        None => {
            eprintln!("server block: function not found: {fn_name}");
            return String::new();
        }
    };

    let req = functions::FnRequest {
        method: "GET".to_string(),
        path: api_path,
        body: String::new(),
        query: params.to_string(),
        headers: HashMap::new(),
    };

    let response = match functions::execute(&file_path, &req, config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("server block: function {fn_name} failed: {e}");
            return String::new();
        }
    };

    if response.status >= 400 {
        eprintln!("server block: function {fn_name} returned status {}", response.status);
        return String::new();
    }

    // Parse JSON response
    let value = match content::parse_json(&response.body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("server block: invalid JSON from {fn_name}: {e}");
            return String::new();
        }
    };

    // Get component template
    let template = match components.get(comp_name) {
        Some(c) => &c.template,
        None => {
            eprintln!("server block: component not found: {comp_name}");
            return String::new();
        }
    };

    // Collect items: array → iterate, object → single item
    let items: Vec<&content::Value> = match &value {
        content::Value::Array(arr) => arr.iter().collect(),
        content::Value::Object(_) => vec![&value],
        _ => Vec::new(),
    };

    // Render component for each item
    let mut html = String::new();
    for item in &items {
        if let content::Value::Object(pairs) = item {
            let props: HashMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str()))
                .collect();
            let rendered = component::render(template, &props, "");
            // Resolve nested component tags in the rendered output
            let resolved = parser::resolve(&rendered, components);
            html.push_str(&resolved);
        }
    }

    // Store in cache if TTL is set
    if let Some(ttl_secs) = ttl {
        if let Ok(mut c) = cache.lock() {
            c.put(cache_key, html.clone(), ttl_secs);
        }
    }

    let elapsed = start.elapsed();
    if elapsed.as_millis() > 1000 {
        eprintln!(
            "server block: {fn_name}/{comp_name} took {}ms",
            elapsed.as_millis()
        );
    }

    html
}

/// Handles an /api/* request by executing the matching JS function.
fn handle_function(method: &str, path: &str, body: &str, headers: HashMap<String, String>, config: &Config) -> (&'static str, String, String) {
    let Some(file_path) = functions::resolve_function(path) else {
        return ("404 Not Found", "function not found".into(), "text/plain".into());
    };

    // Split path and query string
    let (clean_path, query) = path.split_once('?').unwrap_or((path, ""));

    let req = functions::FnRequest {
        method: method.to_string(),
        path: clean_path.to_string(),
        body: body.to_string(),
        query: query.to_string(),
        headers,
    };

    match functions::execute(&file_path, &req, config) {
        Ok(resp) => {
            let status = match resp.status {
                200 => "200 OK",
                201 => "201 Created",
                204 => "204 No Content",
                301 => "301 Moved Permanently",
                302 => "302 Found",
                400 => "400 Bad Request",
                401 => "401 Unauthorized",
                403 => "403 Forbidden",
                404 => "404 Not Found",
                405 => "405 Method Not Allowed",
                409 => "409 Conflict",
                415 => "415 Unsupported Media Type",
                422 => "422 Unprocessable Entity",
                429 => "429 Too Many Requests",
                500 => "500 Internal Server Error",
                _ => "200 OK",
            };
            let ct = resp
                .headers
                .get("content-type")
                .cloned()
                .unwrap_or_else(|| "application/json".into());
            (status, resp.body, ct)
        }
        Err(e) => {
            eprintln!("function error: {e}");
            (
                "500 Internal Server Error",
                "{\"error\":\"internal server error\"}".into(),
                "application/json".into(),
            )
        }
    }
}

/// Resolves a URL path to a file in dist/, handling clean URLs.
/// Serves pre-compressed variants (.br, .gz) when available and accepted.
fn resolve_file(
    url_path: &str,
    accept_br: bool,
    accept_gzip: bool,
) -> (&'static str, Vec<u8>, &'static str, Option<&'static str>) {
    let clean = url_path.trim_start_matches('/');

    // Block sensitive file extensions
    if is_blocked_extension(clean) {
        return ("403 Forbidden", b"forbidden".to_vec(), "text/plain", None);
    }

    let base = PathBuf::from(DIST_DIR);

    let candidates = if clean.is_empty() {
        vec![base.join("index.html")]
    } else {
        vec![base.join(clean), base.join(clean).join("index.html")]
    };

    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return ("404 Not Found", b"404 not found".to_vec(), "text/plain", None),
    };

    for candidate in &candidates {
        if candidate.is_file() {
            if let Ok(canonical) = candidate.canonicalize() {
                if !canonical.starts_with(&base_canonical) {
                    continue;
                }
            } else {
                continue;
            }

            let ct = content_type(candidate.to_str().unwrap_or(""));
            let path_str = candidate.display().to_string();

            // Serve pre-compressed brotli if available and accepted
            if accept_br {
                let br_path = format!("{path_str}.br");
                if let Ok(body) = fs::read(&br_path) {
                    return ("200 OK", body, ct, Some("br"));
                }
            }

            // Serve pre-compressed gzip if available and accepted
            if accept_gzip {
                let gz_path = format!("{path_str}.gz");
                if let Ok(body) = fs::read(&gz_path) {
                    return ("200 OK", body, ct, Some("gzip"));
                }
            }

            // Serve uncompressed
            match fs::read(candidate) {
                Ok(body) => return ("200 OK", body, ct, None),
                Err(_) => continue,
            }
        }
    }

    ("404 Not Found", b"404 not found".to_vec(), "text/plain", None)
}

/// Resolves a URL path to the raw (uncompressed) file in dist/.
/// Used for HTML pages that may contain <Server> blocks requiring SSR.
fn resolve_file_raw(url_path: &str) -> (&'static str, Result<Vec<u8>, ()>) {
    let clean = url_path.trim_start_matches('/');

    if is_blocked_extension(clean) {
        return ("403 Forbidden", Err(()));
    }

    let base = PathBuf::from(DIST_DIR);
    let candidates = if clean.is_empty() {
        vec![base.join("index.html")]
    } else {
        vec![base.join(clean), base.join(clean).join("index.html")]
    };

    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return ("404 Not Found", Err(())),
    };

    for candidate in &candidates {
        if candidate.is_file() {
            if let Ok(canonical) = candidate.canonicalize() {
                if !canonical.starts_with(&base_canonical) {
                    continue;
                }
            } else {
                continue;
            }
            match fs::read(candidate) {
                Ok(body) => return ("200 OK", Ok(body)),
                Err(_) => continue,
            }
        }
    }

    ("404 Not Found", Err(()))
}

/// Returns the content type for a URL path (without reading the file).
/// Clean URLs (no extension) resolve to HTML.
fn content_type_for_path(url_path: &str) -> &'static str {
    let clean = url_path.trim_start_matches('/');
    if clean.is_empty() || clean.ends_with('/') || !clean.contains('.') {
        return "text/html; charset=utf-8";
    }
    content_type(clean)
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".ttf") {
        "font/ttf"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".mp4") {
        "video/mp4"
    } else if path.ends_with(".webm") {
        "video/webm"
    } else {
        "application/octet-stream"
    }
}

/// Cache-Control based on path and content type.
/// - HTML: always revalidate
/// - Hashed files (name.HASH.ext): immutable, 1 year
/// - Other static: 1 day + ETag for revalidation
fn cache_control_for(url_path: &str, content_type: &str) -> &'static str {
    if content_type.starts_with("text/html") {
        "no-cache"
    } else if has_content_hash(url_path) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=86400"
    }
}

/// Detects content-hash pattern: name.XXXXXXXX.ext (8 hex chars before final extension).
fn has_content_hash(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    let hash_part = parts[parts.len() - 2];
    hash_part.len() == 8 && hash_part.chars().all(|c| c.is_ascii_hexdigit())
}

/// Checks if a specific encoding is accepted.
fn accepts_encoding(headers: &str, encoding: &str) -> bool {
    for line in headers.lines() {
        if line.get(..16).is_some_and(|s| s.eq_ignore_ascii_case("accept-encoding:")) {
            return line[16..].to_lowercase().contains(encoding);
        }
    }
    false
}

/// Returns true for content types that benefit from compression.
fn is_compressible(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.contains("javascript")
        || content_type.contains("json")
        || content_type.contains("svg")
        || content_type.contains("xml")
}

/// Compresses data with gzip (fast mode for on-the-fly API responses).
fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_else(|_| data.to_vec())
}

/// Compresses data with brotli (quality 4 for on-the-fly API responses).
fn brotli_compress_fast(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut output, 4096, 4, 22);
        let _ = writer.write_all(data);
    }
    output
}

/// Computes ETag from content bytes (FNV-1a hash).
fn compute_etag(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("\"{hash:x}\"")
}

/// Extracts a header value by name (case-insensitive).
fn extract_header(headers: &str, name: &str) -> Option<String> {
    let prefix_len = name.len() + 1;
    for line in headers.lines() {
        if line.len() > prefix_len
            && line[..prefix_len].eq_ignore_ascii_case(&format!("{name}:"))
        {
            return Some(line[prefix_len..].trim().to_string());
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                result.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Extracts the client IP for rate limiting.
/// When `trusted_proxy` is configured and the peer matches, uses X-Forwarded-For.
fn client_ip(stream: &TcpStream, headers: &str, config: &Config) -> String {
    let peer = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();

    if let Some(ref trusted) = config.trusted_proxy {
        if peer == *trusted || trusted == "*" {
            if let Some(xff) = extract_header(headers, "x-forwarded-for") {
                // Take the leftmost (client) IP from the X-Forwarded-For chain
                if let Some(first) = xff.split(',').next() {
                    let ip = first.trim();
                    if !ip.is_empty() {
                        return ip.to_string();
                    }
                }
            }
        }
    }

    peer
}

fn sanitize_header_value(value: &str) -> String {
    value.chars().filter(|c| *c != '\r' && *c != '\n' && *c != '\0').collect()
}

/// Builds CORS response headers based on the api_cors config and the request Origin.
/// Returns an empty string if api_cors is not configured (no CORS = same-origin only).
fn cors_headers(config: &Config, request_origin: Option<&str>) -> String {
    if config.api_cors.is_empty() {
        return String::new();
    }

    let allowed_origin = if config.api_cors == "*" {
        "*".to_string()
    } else if let Some(origin) = request_origin {
        let allowed: Vec<&str> = config.api_cors.split(',').map(|s| s.trim()).collect();
        if allowed.iter().any(|a| a.eq_ignore_ascii_case(origin)) {
            origin.to_string()
        } else {
            return String::new();
        }
    } else {
        return String::new();
    };

    let mut h = String::new();
    h.push_str(&format!("Access-Control-Allow-Origin: {}\r\n", sanitize_header_value(&allowed_origin)));
    h.push_str("Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS\r\n");
    h.push_str("Access-Control-Allow-Headers: Content-Type, Authorization, X-Requested-With\r\n");
    h.push_str("Access-Control-Max-Age: 86400\r\n");
    if allowed_origin != "*" {
        h.push_str("Vary: Origin\r\n");
    }
    h
}

/// Parses raw HTTP headers into a HashMap, filtering sensitive headers.
/// Header names are lowercased for consistent access in JS.
fn parse_request_headers(headers: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in headers.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_lowercase();
            let value = sanitize_header_value(value.trim());
            if !functions::is_filtered_request_header(&name) && !name.is_empty() {
                map.insert(name, value);
            }
        }
    }
    map
}

/// Verifies a GitHub-style HMAC-SHA256 webhook signature.
/// Header format: sha256=<hex-encoded-hmac>
fn verify_webhook_signature(secret: &str, payload: &[u8], signature_header: &str) -> bool {
    let hex_sig = match signature_header.strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };
    let sig_bytes = match hex_decode(hex_sig) {
        Some(b) => b,
        None => return false,
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key size");
    mac.update(payload);
    mac.verify_slice(&sig_bytes).is_ok()
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Cache control
    // -----------------------------------------------------------------------

    #[test]
    fn cache_html_is_no_cache() {
        assert_eq!(cache_control_for("/index.html", "text/html; charset=utf-8"), "no-cache");
        assert_eq!(cache_control_for("/about/index.html", "text/html; charset=utf-8"), "no-cache");
    }

    #[test]
    fn cache_hashed_files_are_immutable() {
        assert_eq!(
            cache_control_for("/style.a1b2c3d4.css", "text/css"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(
            cache_control_for("/main.deadbeef.js", "application/javascript"),
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn cache_non_hashed_assets_short_lived() {
        // Images without hash get 1-day cache, not immutable
        assert_eq!(cache_control_for("/photo.png", "image/png"), "public, max-age=86400");
        assert_eq!(cache_control_for("/logo.svg", "image/svg+xml"), "public, max-age=86400");
    }

    // -----------------------------------------------------------------------
    // Content hash detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_hashed_filenames() {
        assert!(has_content_hash("/style.a1b2c3d4.css"));
        assert!(has_content_hash("/main.deadbeef.js"));
        assert!(has_content_hash("/css/app.00ff00ff.css"));
    }

    #[test]
    fn rejects_non_hashed_filenames() {
        assert!(!has_content_hash("/style.css"));
        assert!(!has_content_hash("/main.js"));
        assert!(!has_content_hash("/photo.png"));
        assert!(!has_content_hash("/index.html"));
        // Too short to be a hash
        assert!(!has_content_hash("/style.abc.css"));
        // Too long
        assert!(!has_content_hash("/style.a1b2c3d4e5.css"));
        // Non-hex chars
        assert!(!has_content_hash("/style.ghijklmn.css"));
    }

    // -----------------------------------------------------------------------
    // Accept-Encoding parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parses_accept_encoding_br() {
        let headers = "GET / HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip, br\r\n";
        assert!(accepts_encoding(headers, "br"));
        assert!(accepts_encoding(headers, "gzip"));
        assert!(!accepts_encoding(headers, "zstd"));
    }

    #[test]
    fn parses_no_accept_encoding() {
        let headers = "GET / HTTP/1.1\r\nHost: localhost\r\n";
        assert!(!accepts_encoding(headers, "br"));
        assert!(!accepts_encoding(headers, "gzip"));
    }

    // -----------------------------------------------------------------------
    // Content type detection
    // -----------------------------------------------------------------------

    #[test]
    fn content_type_css() {
        assert_eq!(content_type("style.css"), "text/css");
        assert_eq!(content_type("style.a1b2c3d4.css"), "text/css");
    }

    #[test]
    fn content_type_js() {
        assert_eq!(content_type("main.js"), "application/javascript");
    }

    #[test]
    fn content_type_images() {
        assert_eq!(content_type("photo.png"), "image/png");
        assert_eq!(content_type("photo.jpg"), "image/jpeg");
        assert_eq!(content_type("photo.webp"), "image/webp");
        assert_eq!(content_type("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn content_type_fonts() {
        assert_eq!(content_type("font.woff2"), "font/woff2");
        assert_eq!(content_type("font.woff"), "font/woff");
        assert_eq!(content_type("font.ttf"), "font/ttf");
    }

    #[test]
    fn content_type_unknown_fallback() {
        assert_eq!(content_type("file.xyz"), "application/octet-stream");
    }

    // -----------------------------------------------------------------------
    // Compressibility
    // -----------------------------------------------------------------------

    #[test]
    fn compressible_types() {
        assert!(is_compressible("text/html; charset=utf-8"));
        assert!(is_compressible("text/css"));
        assert!(is_compressible("application/javascript"));
        assert!(is_compressible("application/json"));
        assert!(is_compressible("image/svg+xml"));
    }

    #[test]
    fn non_compressible_types() {
        assert!(!is_compressible("image/png"));
        assert!(!is_compressible("image/jpeg"));
        assert!(!is_compressible("font/woff2"));
        assert!(!is_compressible("application/octet-stream"));
    }

    // -----------------------------------------------------------------------
    // Compression functions
    // -----------------------------------------------------------------------

    #[test]
    fn gzip_roundtrip() {
        let original = b"hello world, this is a test of gzip compression";
        let compressed = gzip_compress(original);
        assert!(compressed.len() < original.len() || original.len() < 50);

        // Verify it's valid gzip (starts with magic bytes 1f 8b)
        assert_eq!(compressed[0], 0x1f);
        assert_eq!(compressed[1], 0x8b);
    }

    #[test]
    fn brotli_produces_output() {
        let original = b"hello world, this is a test of brotli compression that needs to be reasonably long";
        let compressed = brotli_compress_fast(original);
        assert!(!compressed.is_empty());
        assert!(compressed.len() < original.len());
    }

    // -----------------------------------------------------------------------
    // ETag
    // -----------------------------------------------------------------------

    #[test]
    fn etag_is_deterministic() {
        let data = b"test content";
        assert_eq!(compute_etag(data), compute_etag(data));
    }

    #[test]
    fn etag_differs_for_different_content() {
        assert_ne!(compute_etag(b"hello"), compute_etag(b"world"));
    }

    #[test]
    fn etag_is_quoted() {
        let etag = compute_etag(b"test");
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
    }

    // -----------------------------------------------------------------------
    // Header extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_header_found() {
        let headers = "GET / HTTP/1.1\r\nHost: example.com\r\nIf-None-Match: \"abc123\"\r\n";
        assert_eq!(
            extract_header(headers, "if-none-match"),
            Some("\"abc123\"".to_string())
        );
    }

    #[test]
    fn extract_header_not_found() {
        let headers = "GET / HTTP/1.1\r\nHost: example.com\r\n";
        assert_eq!(extract_header(headers, "if-none-match"), None);
    }

    #[test]
    fn extract_header_case_insensitive() {
        let headers = "GET / HTTP/1.1\r\nHost: localhost\r\n";
        assert_eq!(extract_header(headers, "host"), Some("localhost".to_string()));
    }

    // -----------------------------------------------------------------------
    // URL decode
    // -----------------------------------------------------------------------

    #[test]
    fn url_decode_plain() {
        assert_eq!(url_decode("/about"), "/about");
    }

    #[test]
    fn url_decode_encoded() {
        assert_eq!(url_decode("/hello%20world"), "/hello world");
        assert_eq!(url_decode("/a%2Fb"), "/a/b");
    }

    #[test]
    fn url_decode_traversal_attempt() {
        let decoded = url_decode("/..%2F..%2Fetc%2Fpasswd");
        assert!(decoded.contains(".."));
    }

    // -----------------------------------------------------------------------
    // Blocked extensions
    // -----------------------------------------------------------------------

    #[test]
    fn blocks_dangerous_paths() {
        assert!(is_blocked_extension("data.db"));
        assert!(is_blocked_extension("/path/to/.env"));
        assert!(is_blocked_extension("secret.key"));
        assert!(is_blocked_extension("deploy.sh"));
        assert!(is_blocked_extension("path/.env.production"));
    }

    #[test]
    fn allows_safe_paths() {
        assert!(!is_blocked_extension("style.css"));
        assert!(!is_blocked_extension("app.js"));
        assert!(!is_blocked_extension("image.png"));
        assert!(!is_blocked_extension("page/index.html"));
    }

    // -----------------------------------------------------------------------
    // Header sanitization
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_strips_crlf() {
        assert_eq!(
            sanitize_header_value("value\r\nInjected: bad"),
            "valueInjected: bad"
        );
    }

    #[test]
    fn sanitize_strips_null() {
        assert_eq!(sanitize_header_value("value\0hidden"), "valuehidden");
    }

    #[test]
    fn sanitize_passes_clean_value() {
        assert_eq!(sanitize_header_value("application/json"), "application/json");
    }

    // -----------------------------------------------------------------------
    // CORS
    // -----------------------------------------------------------------------

    #[test]
    fn cors_empty_when_not_configured() {
        let config = Config::default();
        assert_eq!(cors_headers(&config, Some("https://evil.com")), "");
    }

    #[test]
    fn cors_wildcard() {
        let mut config = Config::default();
        config.api_cors = "*".to_string();
        let h = cors_headers(&config, Some("https://example.com"));
        assert!(h.contains("Access-Control-Allow-Origin: *"));
        assert!(h.contains("Access-Control-Allow-Methods:"));
        assert!(h.contains("Access-Control-Max-Age: 86400"));
        assert!(!h.contains("Vary: Origin"));
    }

    #[test]
    fn cors_specific_origin_match() {
        let mut config = Config::default();
        config.api_cors = "https://example.com".to_string();
        let h = cors_headers(&config, Some("https://example.com"));
        assert!(h.contains("Access-Control-Allow-Origin: https://example.com"));
        assert!(h.contains("Vary: Origin"));
    }

    #[test]
    fn cors_specific_origin_no_match() {
        let mut config = Config::default();
        config.api_cors = "https://example.com".to_string();
        let h = cors_headers(&config, Some("https://evil.com"));
        assert_eq!(h, "");
    }

    #[test]
    fn cors_multiple_origins() {
        let mut config = Config::default();
        config.api_cors = "https://a.com, https://b.com".to_string();
        let h = cors_headers(&config, Some("https://b.com"));
        assert!(h.contains("Access-Control-Allow-Origin: https://b.com"));
    }

    #[test]
    fn cors_no_origin_header() {
        let mut config = Config::default();
        config.api_cors = "https://example.com".to_string();
        let h = cors_headers(&config, None);
        assert_eq!(h, "");
    }

    // -----------------------------------------------------------------------
    // Request header parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_headers_basic() {
        let raw = "GET /api/test HTTP/1.1\r\nAuthorization: Bearer token123\r\nX-Custom: value\r\nHost: localhost\r\n";
        let headers = parse_request_headers(raw);
        assert_eq!(headers.get("authorization"), Some(&"Bearer token123".to_string()));
        assert_eq!(headers.get("x-custom"), Some(&"value".to_string()));
        assert!(headers.get("host").is_none());
    }

    #[test]
    fn parse_headers_filters_sensitive() {
        let raw = "POST /api/test HTTP/1.1\r\nHost: localhost\r\nContent-Length: 42\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\nCookie: session=abc\r\n";
        let headers = parse_request_headers(raw);
        assert!(headers.get("host").is_none());
        assert!(headers.get("content-length").is_none());
        assert!(headers.get("transfer-encoding").is_none());
        assert!(headers.get("connection").is_none());
        assert_eq!(headers.get("cookie"), Some(&"session=abc".to_string()));
    }

    #[test]
    fn parse_headers_lowercases_names() {
        let raw = "GET /api/test HTTP/1.1\r\nX-Request-ID: 123\r\nAccept: application/json\r\n";
        let headers = parse_request_headers(raw);
        assert!(headers.get("x-request-id").is_some());
        assert!(headers.get("accept").is_some());
    }

    // -----------------------------------------------------------------------
    // Server block marker parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_marker_basic() {
        let marker = r#"<!--vanilo:server fn="articles" comp="Card"-->"#;
        let (f, c, ttl, params) = parse_server_marker(marker).unwrap();
        assert_eq!(f, "articles");
        assert_eq!(c, "Card");
        assert!(ttl.is_none());
        assert_eq!(params, "");
    }

    #[test]
    fn parse_marker_with_cache() {
        let marker = r#"<!--vanilo:server fn="articles" comp="Card" cache="3600"-->"#;
        let (f, c, ttl, params) = parse_server_marker(marker).unwrap();
        assert_eq!(f, "articles");
        assert_eq!(c, "Card");
        assert_eq!(ttl, Some(3600));
        assert_eq!(params, "");
    }

    #[test]
    fn parse_marker_with_params() {
        let marker = r#"<!--vanilo:server fn="articles" comp="Card" params="category=rust&limit=10"-->"#;
        let (f, c, ttl, params) = parse_server_marker(marker).unwrap();
        assert_eq!(f, "articles");
        assert_eq!(c, "Card");
        assert!(ttl.is_none());
        assert_eq!(params, "category=rust&limit=10");
    }

    #[test]
    fn parse_marker_with_all() {
        let marker = r#"<!--vanilo:server fn="articles" comp="Card" cache="60" params="category=rust"-->"#;
        let (f, c, ttl, params) = parse_server_marker(marker).unwrap();
        assert_eq!(f, "articles");
        assert_eq!(c, "Card");
        assert_eq!(ttl, Some(60));
        assert_eq!(params, "category=rust");
    }

    #[test]
    fn parse_marker_invalid() {
        assert!(parse_server_marker("<!-- not a server marker -->").is_none());
        assert!(parse_server_marker("random text").is_none());
    }

    // -----------------------------------------------------------------------
    // ServerCache
    // -----------------------------------------------------------------------

    #[test]
    fn cache_put_and_get() {
        let mut cache = ServerCache::new();
        cache.put("key".into(), "<div>cached</div>".into(), 60);
        assert_eq!(cache.get("key"), Some("<div>cached</div>".into()));
    }

    #[test]
    fn cache_miss() {
        let mut cache = ServerCache::new();
        assert_eq!(cache.get("missing"), None);
    }

    #[test]
    fn cache_evicts_at_capacity() {
        let mut cache = ServerCache::new();
        for i in 0..SERVER_CACHE_MAX + 5 {
            cache.put(format!("k{i}"), format!("v{i}"), 3600);
        }
        assert!(cache.entries.len() <= SERVER_CACHE_MAX);
    }

    // -----------------------------------------------------------------------
    // Content type for clean URLs
    // -----------------------------------------------------------------------

    #[test]
    fn clean_url_is_html() {
        assert!(content_type_for_path("/blog").starts_with("text/html"));
        assert!(content_type_for_path("/about").starts_with("text/html"));
        assert!(content_type_for_path("/").starts_with("text/html"));
    }

    #[test]
    fn file_url_keeps_type() {
        assert_eq!(content_type_for_path("/style.css"), "text/css");
        assert_eq!(content_type_for_path("/main.js"), "application/javascript");
    }

    // -----------------------------------------------------------------------
    // Component rendering with parsed JSON (integration)
    // -----------------------------------------------------------------------

    #[test]
    fn render_components_from_json_array() {
        // Simulate what process_server_blocks does:
        // 1. Parse JSON from function response
        // 2. Render component for each item

        let json_str = r#"[{"title":"Hello World","description":"First post"},{"title":"Server Blocks","description":"SSR for Vanilo"}]"#;
        let value = content::parse_json(json_str).unwrap();

        let template = "<div class=\"card\">\n    <h3>{{title}}</h3>\n    <p>{{description}}</p>\n</div>";

        let items: Vec<&content::Value> = match &value {
            content::Value::Array(arr) => arr.iter().collect(),
            content::Value::Object(_) => vec![&value],
            _ => Vec::new(),
        };

        assert_eq!(items.len(), 2);

        let mut html = String::new();
        for item in &items {
            if let content::Value::Object(pairs) = item {
                let props: HashMap<String, String> = pairs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str()))
                    .collect();
                assert!(props.contains_key("title"), "props missing 'title': {:?}", props);
                assert!(props.contains_key("description"), "props missing 'description': {:?}", props);
                let rendered = component::render(template, &props, "");
                html.push_str(&rendered);
            }
        }

        assert!(html.contains("<h3>Hello World</h3>"), "rendered HTML: {html}");
        assert!(html.contains("<h3>Server Blocks</h3>"), "rendered HTML: {html}");
        assert!(html.contains("<p>First post</p>"), "rendered HTML: {html}");
        assert!(html.contains("<p>SSR for Vanilo</p>"), "rendered HTML: {html}");
    }

    #[test]
    fn render_single_object_from_json() {
        let json_str = r#"{"title":"Solo","description":"Single object"}"#;
        let value = content::parse_json(json_str).unwrap();

        let template = "<div><h3>{{title}}</h3></div>";

        let items: Vec<&content::Value> = match &value {
            content::Value::Array(arr) => arr.iter().collect(),
            content::Value::Object(_) => vec![&value],
            _ => Vec::new(),
        };

        assert_eq!(items.len(), 1);

        if let content::Value::Object(pairs) = items[0] {
            let props: HashMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str()))
                .collect();
            let rendered = component::render(template, &props, "");
            assert!(rendered.contains("<h3>Solo</h3>"), "rendered: {rendered}");
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn render_empty_array_produces_nothing() {
        let json_str = "[]";
        let value = content::parse_json(json_str).unwrap();

        let items: Vec<&content::Value> = match &value {
            content::Value::Array(arr) => arr.iter().collect(),
            _ => Vec::new(),
        };

        assert_eq!(items.len(), 0);
    }
}
