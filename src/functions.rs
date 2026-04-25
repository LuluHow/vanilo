use std::collections::HashMap;
use std::fs;
use std::path::Path;

use rquickjs::{Context, Runtime};

const FUNCTIONS_DIR: &str = "functions";

/// Request passed to the JS function.
pub struct FnRequest {
    pub method: String,
    pub path: String,
    pub body: String,
    pub query: String,
}

/// Response returned by the JS function.
pub struct FnResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Resolves an API path to a function file.
/// /api/hello      -> functions/hello.js
/// /api/users/list -> functions/users/list.js
pub fn resolve_function(api_path: &str) -> Option<String> {
    let clean = api_path
        .strip_prefix("/api/")
        .unwrap_or(api_path)
        .trim_end_matches('/');

    if clean.is_empty() {
        return None;
    }

    let file_path = format!("{FUNCTIONS_DIR}/{clean}.js");
    if Path::new(&file_path).is_file() {
        Some(file_path)
    } else {
        // Try index.js inside directory
        let index_path = format!("{FUNCTIONS_DIR}/{clean}/index.js");
        if Path::new(&index_path).is_file() {
            Some(index_path)
        } else {
            None
        }
    }
}

/// Executes a JS function file with the given request.
pub fn execute(file_path: &str, req: &FnRequest) -> Result<FnResponse, String> {
    let source = fs::read_to_string(file_path)
        .map_err(|e| format!("read {file_path}: {e}"))?;

    let rt = Runtime::new().map_err(|e| format!("quickjs runtime: {e}"))?;
    let ctx = Context::full(&rt).map_err(|e| format!("quickjs context: {e}"))?;

    ctx.with(|ctx| {
        // Build the wrapper script that:
        // 1. Evaluates the user module to get the default export
        // 2. Calls it with the request object
        // 3. Returns the result as JSON
        let wrapper = format!(
            r#"
            var __req = {{
                method: {method},
                path: {path},
                body: {body},
                query: {query}
            }};

            {source}

            var __fn = typeof handler === 'function' ? handler : null;
            var __result;

            if (__fn) {{
                __result = __fn(__req);
            }} else {{
                __result = {{ status: 500, body: "no handler function found" }};
            }}

            JSON.stringify({{
                status: __result.status || 200,
                headers: __result.headers || {{}},
                body: typeof __result.body === 'string' ? __result.body : JSON.stringify(__result.body || "")
            }});
            "#,
            method = json_string(&req.method),
            path = json_string(&req.path),
            body = json_string(&req.body),
            query = json_string(&req.query),
        );

        let result: String = ctx
            .eval(wrapper)
            .map_err(|e| format!("execute {file_path}: {e}"))?;

        parse_response(&result)
    })
}

/// Parses the JSON response string from the JS function.
fn parse_response(json: &str) -> Result<FnResponse, String> {
    // Minimal JSON parsing — no serde dependency
    let status = extract_json_number(json, "status").unwrap_or(200);
    let body = extract_json_string(json, "body").unwrap_or_default();
    let headers = extract_json_object(json, "headers");

    Ok(FnResponse {
        status,
        headers,
        body,
    })
}

/// Escapes a Rust string into a JSON string literal (with quotes).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Extracts a number value for a key from a flat JSON object.
fn extract_json_number(json: &str, key: &str) -> Option<u16> {
    let pattern = format!("\"{}\":", key);
    let pos = json.find(&pattern)? + pattern.len();
    let rest = json[pos..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Extracts a string value for a key from a flat JSON object.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":", key);
    let pos = json.find(&pattern)? + pattern.len();
    let rest = json[pos..].trim_start();

    if !rest.starts_with('"') {
        return None;
    }

    let inner = &rest[1..];
    let mut result = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(result),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    match escaped {
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        _ => {
                            result.push('\\');
                            result.push(escaped);
                        }
                    }
                }
            }
            _ => result.push(c),
        }
    }
    None
}

/// Extracts a flat object of string key-value pairs from JSON.
fn extract_json_object(json: &str, key: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let pattern = format!("\"{}\":", key);
    let Some(pos) = json.find(&pattern) else {
        return map;
    };
    let rest = json[pos + pattern.len()..].trim_start();
    if !rest.starts_with('{') {
        return map;
    }
    let inner = &rest[1..];
    let end = inner.find('}').unwrap_or(inner.len());
    let obj = &inner[..end];

    // Parse simple "key":"value" pairs
    let mut remaining = obj;
    while let Some(key_start) = remaining.find('"') {
        remaining = &remaining[key_start + 1..];
        let Some(key_end) = remaining.find('"') else {
            break;
        };
        let k = remaining[..key_end].to_string();
        remaining = &remaining[key_end + 1..];

        // Find : then "value"
        let Some(colon) = remaining.find(':') else {
            break;
        };
        remaining = remaining[colon + 1..].trim_start();
        if !remaining.starts_with('"') {
            break;
        }
        remaining = &remaining[1..];
        let Some(val_end) = remaining.find('"') else {
            break;
        };
        let v = remaining[..val_end].to_string();
        remaining = &remaining[val_end + 1..];

        map.insert(k, v);
    }

    map
}

/// Checks if the functions directory exists.
pub fn has_functions() -> bool {
    Path::new(FUNCTIONS_DIR).is_dir()
}
