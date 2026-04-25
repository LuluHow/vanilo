use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use crate::functions;

const DIST_DIR: &str = "dist";

pub fn serve(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("serving on http://{addr}");

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("connection error: {e}");
                continue;
            }
        };

        let mut buf = [0u8; 4096];
        let n = match stream.read(&mut buf) {
            Ok(n) => n,
            Err(_) => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);
        let first_line = match request.lines().next() {
            Some(line) => line,
            None => continue,
        };
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("GET");
        let path = parts.next().unwrap_or("/");

        // Extract request body (after the empty line)
        let req_body = request
            .find("\r\n\r\n")
            .map(|i| &request[i + 4..])
            .unwrap_or("");

        if path.starts_with("/api/") {
            let (status, body, content_type) = handle_function(method, path, req_body);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        } else {
            let (status, body, content_type) = resolve_file(path);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&body);
        }
    }

    Ok(())
}

/// Handles an /api/* request by executing the matching JS function.
fn handle_function(method: &str, path: &str, body: &str) -> (&'static str, String, String) {
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

    match functions::execute(&file_path, &req) {
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
                format!("{{\"error\":\"{e}\"}}"),
                "application/json".into(),
            )
        }
    }
}

/// Resolves a URL path to a file in dist/, handling clean URLs.
fn resolve_file(url_path: &str) -> (&'static str, Vec<u8>, &'static str) {
    let clean = url_path.trim_start_matches('/');
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

    for candidate in &candidates {
        if candidate.is_file() {
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
