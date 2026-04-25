use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

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
        let path = match request.lines().next() {
            Some(line) => line.split_whitespace().nth(1).unwrap_or("/"),
            None => continue,
        };

        let (status, body, content_type) = resolve_file(path);

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );

        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
    }

    Ok(())
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
