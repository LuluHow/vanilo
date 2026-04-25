use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::functions;

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

/// Ensures the active connection counter is decremented even on panic.
struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn serve(config: Config) -> Result<(), String> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("serving on http://{addr}");

    let config = Arc::new(config);
    let rate_map: Arc<Mutex<HashMap<String, (usize, Instant)>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let active = Arc::new(AtomicUsize::new(0));

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
        active.fetch_add(1, Ordering::Relaxed);
        let guard = ConnectionGuard(active.clone());

        thread::spawn(move || {
            let _guard = guard;
            handle_connection(&mut stream, &rate, &cfg);
        });
    }

    Ok(())
}

fn handle_connection(
    stream: &mut TcpStream,
    rate_map: &Mutex<HashMap<String, (usize, Instant)>>,
    config: &Config,
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

    if raw_path.starts_with("/api/") {
        // Rate limiting by IP
        let ip = stream
            .peer_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();

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
        let (status, body, content_type) = handle_function(effective_method, raw_path, &req_body, config);
        let content_type = sanitize_header_value(&content_type);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\n{sec}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            len = body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        if method != "HEAD" {
            let _ = stream.write_all(body.as_bytes());
        }
    } else {
        // Strip query string for static file serving
        let file_path = decoded_path.split('?').next().unwrap_or(&decoded_path);
        let (status, body, content_type) = resolve_file(file_path);
        let cache = cache_control(content_type);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{sec}\r\nCache-Control: {cache}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        if method != "HEAD" {
            let _ = stream.write_all(&body);
        }
    }
}

/// Handles an /api/* request by executing the matching JS function.
fn handle_function(method: &str, path: &str, body: &str, config: &Config) -> (&'static str, String, String) {
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
fn resolve_file(url_path: &str) -> (&'static str, Vec<u8>, &'static str) {
    let clean = url_path.trim_start_matches('/');

    // Block sensitive file extensions
    if is_blocked_extension(clean) {
        return ("403 Forbidden", b"forbidden".to_vec(), "text/plain");
    }

    let base = PathBuf::from(DIST_DIR);

    // Try candidates in order:
    // 1. Exact file (e.g. /style.css)
    // 2. Directory index (e.g. /about -> /about/index.html)
    // 3. As index.html (e.g. / -> /index.html)
    let candidates = if clean.is_empty() {
        vec![base.join("index.html")]
    } else {
        vec![
            base.join(clean),
            base.join(clean).join("index.html"),
        ]
    };

    // Resolve base directory for symlink/traversal protection
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return ("404 Not Found", b"404 not found".to_vec(), "text/plain"),
    };

    for candidate in &candidates {
        if candidate.is_file() {
            // Verify path stays within dist/ (blocks symlink escapes)
            if let Ok(canonical) = candidate.canonicalize() {
                if !canonical.starts_with(&base_canonical) {
                    continue;
                }
            } else {
                continue;
            }
            match fs::read(candidate) {
                Ok(body) => {
                    let ct = content_type(candidate.to_str().unwrap_or(""));
                    return ("200 OK", body, ct);
                }
                Err(_) => continue,
            }
        }
    }

    ("404 Not Found", b"404 not found".to_vec(), "text/plain")
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

fn cache_control(content_type: &str) -> &'static str {
    if content_type.starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=86400"
    }
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

fn sanitize_header_value(value: &str) -> String {
    value.chars().filter(|c| *c != '\r' && *c != '\n' && *c != '\0').collect()
}
