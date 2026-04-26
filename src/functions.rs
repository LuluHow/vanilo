use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Function, Runtime};
use rquickjs::function::Opt;

use crate::config::Config;

const FUNCTIONS_DIR: &str = "functions";
const DB_FILE: &str = "data.db";
const MAX_FETCH_RESPONSE: usize = 10 * 1024 * 1024; // 10 MB

fn to_js_err(msg: &str) -> rquickjs::Error {
    rquickjs::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, msg.to_string()))
}

pub struct FnRequest {
    pub method: String,
    pub path: String,
    pub body: String,
    pub query: String,
}

pub struct FnResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Resolves /api/hello -> functions/hello.js
pub fn resolve_function(api_path: &str) -> Option<String> {
    let path = api_path.split('?').next().unwrap_or(api_path);
    let clean = path.strip_prefix("/api/").unwrap_or(path).trim_end_matches('/');

    if clean.is_empty() || clean.contains("..") || clean.starts_with('/') {
        return None;
    }

    let file_path = format!("{FUNCTIONS_DIR}/{clean}.js");
    if Path::new(&file_path).is_file() {
        return Some(file_path);
    }
    let index_path = format!("{FUNCTIONS_DIR}/{clean}/index.js");
    if Path::new(&index_path).is_file() {
        return Some(index_path);
    }
    None
}

/// Executes a JS function file with the given request.
pub fn execute(file_path: &str, req: &FnRequest, config: &Config) -> Result<FnResponse, String> {
    let source =
        fs::read_to_string(file_path).map_err(|e| format!("read {file_path}: {e}"))?;

    let rt = Runtime::new().map_err(|e| format!("quickjs runtime: {e}"))?;
    rt.set_memory_limit(config.memory);
    rt.set_max_stack_size(1024 * 1024); // 1 MB — prevents stack overflow crash via deep recursion

    // Timeout: interrupt after configured seconds
    let timeout_secs = config.timeout;
    let start = Instant::now();
    rt.set_interrupt_handler(Some(Box::new(move || {
        start.elapsed().as_secs() >= timeout_secs
    })));

    let ctx = Context::full(&rt).map_err(|e| format!("quickjs context: {e}"))?;

    // Lazy DB — only opened on first db call
    let db_conn: Rc<RefCell<Option<rusqlite::Connection>>> = Rc::new(RefCell::new(None));

    ctx.with(|ctx| {
        let globals = ctx.globals();

        // __db_query(sql, params_json) -> json string
        let conn_q = db_conn.clone();
        let db_query = Function::new(ctx.clone(), move |sql: String, params_json: Opt<String>| -> rquickjs::Result<String> {
            open_db(&conn_q).map_err(|e| to_js_err(&e))?;
            let slot = conn_q.borrow();
            let c = slot.as_ref().unwrap();
            let params = parse_json_params(&params_json.0.unwrap_or_default());
            db_query_impl(c, &sql, &params).map_err(|e| to_js_err(&e))
        }).map_err(|e| format!("create __db_query: {e}"))?;
        globals.set("__db_query", db_query).map_err(|e| format!("set __db_query: {e}"))?;

        // __db_exec(sql, params_json) -> json string
        let conn_e = db_conn.clone();
        let db_exec = Function::new(ctx.clone(), move |sql: String, params_json: Opt<String>| -> rquickjs::Result<String> {
            open_db(&conn_e).map_err(|e| to_js_err(&e))?;
            let slot = conn_e.borrow();
            let c = slot.as_ref().unwrap();
            let params = parse_json_params(&params_json.0.unwrap_or_default());
            db_exec_impl(c, &sql, &params).map_err(|e| to_js_err(&e))
        }).map_err(|e| format!("create __db_exec: {e}"))?;
        globals.set("__db_exec", db_exec).map_err(|e| format!("set __db_exec: {e}"))?;

        // __fetch(url, opts_json) -> json string
        let fetch_timeout = Duration::from_secs(config.fetch_timeout);
        let fetch_fn = Function::new(ctx.clone(), move |url: String, opts_json: Opt<String>| -> rquickjs::Result<String> {
            if is_private_url(&url) {
                return Err(to_js_err("fetch: blocked request to private/internal address"));
            }
            let resolved_ip = check_resolved_ips(&url).map_err(|e| to_js_err(&e))?;
            let opts = opts_json.0.unwrap_or_default();
            fetch_impl(&url, &opts, resolved_ip, fetch_timeout).map_err(|e| to_js_err(&e))
        }).map_err(|e| format!("create __fetch: {e}"))?;
        globals.set("__fetch", fetch_fn).map_err(|e| format!("set __fetch: {e}"))?;

        let wrapper = format!(
            r#"var db = {{
    query: function(sql, params) {{
        return JSON.parse(__db_query(sql, JSON.stringify(params || [])));
    }},
    exec: function(sql, params) {{
        return JSON.parse(__db_exec(sql, JSON.stringify(params || [])));
    }}
}};

var fetch = function(url, opts) {{
    return JSON.parse(__fetch(url, JSON.stringify(opts || {{}})));
}};

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
    try {{
        __result = __fn(__req);
    }} catch(e) {{
        __result = {{ status: 400, body: JSON.stringify({{ error: "bad request" }}), __error: String(e) }};
    }}
}} else {{
    __result = {{ status: 500, body: "no handler function found" }};
}}

JSON.stringify({{
    status: __result.status || 200,
    headers: __result.headers || {{}},
    body: typeof __result.body === 'string' ? __result.body : JSON.stringify(__result.body || "")
}});"#,
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

// ---------------------------------------------------------------------------
// DB implementation
// ---------------------------------------------------------------------------

fn open_db(conn: &Rc<RefCell<Option<rusqlite::Connection>>>) -> Result<(), String> {
    let mut slot = conn.borrow_mut();
    if slot.is_none() {
        let c = rusqlite::Connection::open(DB_FILE)
            .map_err(|e| format!("open {DB_FILE}: {e}"))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA max_page_count=65536;")
            .map_err(|e| format!("pragma: {e}"))?;
        // Authorizer: whitelist safe operations, deny everything else
        c.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| -> rusqlite::hooks::Authorization {
            use rusqlite::hooks::{AuthAction, Authorization};
            match ctx.action {
                // DML: normal data operations
                AuthAction::Read { .. }
                | AuthAction::Select
                | AuthAction::Insert { .. }
                | AuthAction::Update { .. }
                | AuthAction::Delete { .. } => Authorization::Allow,

                // Basic DDL: table and index management
                AuthAction::CreateTable { .. }
                | AuthAction::DropTable { .. }
                | AuthAction::CreateIndex { .. }
                | AuthAction::DropIndex { .. } => Authorization::Allow,

                // Temp objects: per-connection, safe
                AuthAction::CreateTempTable { .. }
                | AuthAction::DropTempTable { .. }
                | AuthAction::CreateTempIndex { .. }
                | AuthAction::DropTempIndex { .. } => Authorization::Allow,

                // Transaction control
                AuthAction::Transaction { .. }
                | AuthAction::Savepoint { .. } => Authorization::Allow,

                // Recursive CTEs
                AuthAction::Recursive => Authorization::Allow,

                // Read-only introspection PRAGMAs only
                AuthAction::Pragma { pragma_name, .. } => {
                    match pragma_name {
                        "table_info" | "index_list" | "foreign_keys"
                        | "busy_timeout" => Authorization::Allow,
                        _ => Authorization::Deny,
                    }
                }

                // SQL functions: block load_extension
                AuthAction::Function { function_name } => {
                    match function_name {
                        "load_extension" => Authorization::Deny,
                        _ => Authorization::Allow,
                    }
                }

                // Deny everything else: Attach, Detach, CreateTrigger,
                // DropTrigger, CreateView, DropView, CreateVtable,
                // DropVtable, AlterTable, Reindex, Analyze,
                // CreateTempTrigger, DropTempTrigger,
                // CreateTempView, DropTempView
                _ => Authorization::Deny,
            }
        })).map_err(|e| format!("authorizer: {e}"))?;
        *slot = Some(c);
    }
    Ok(())
}

fn is_blocked_sql(sql: &str) -> Option<&'static str> {
    // Strip leading SQL comments to prevent bypass via /**/VACUUM or --\nVACUUM
    let mut s = sql.trim();
    loop {
        if s.starts_with("--") {
            s = match s.find('\n') {
                Some(i) => &s[i + 1..],
                None => "",
            };
            s = s.trim_start();
        } else if s.starts_with("/*") {
            s = match s[2..].find("*/") {
                Some(i) => &s[2 + i + 2..],
                None => "",
            };
            s = s.trim_start();
        } else {
            break;
        }
    }
    let upper: String = s.chars().take(10).collect::<String>().to_uppercase();
    if upper.starts_with("VACUUM") {
        return Some("VACUUM statements are not allowed");
    }
    None
}

fn db_query_impl(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<String, String> {
    if let Some(msg) = is_blocked_sql(sql) {
        return Err(format!("db.query: {msg}"));
    }
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("db.query: {e}"))?;

    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            let mut cols = Vec::with_capacity(col_count);
            for i in 0..col_count {
                cols.push(row.get::<_, rusqlite::types::Value>(i)?);
            }
            Ok(cols)
        })
        .map_err(|e| format!("db.query: {e}"))?;

    let mut json = String::from("[");
    let mut first_row = true;
    for row_result in rows {
        let cols = row_result.map_err(|e| format!("db.query row: {e}"))?;
        if !first_row {
            json.push(',');
        }
        json.push('{');
        let mut first_col = true;
        for (i, val) in cols.iter().enumerate() {
            if !first_col {
                json.push(',');
            }
            json.push_str(&json_string(&col_names[i]));
            json.push(':');
            json.push_str(&sqlite_val_to_json(val));
            first_col = false;
        }
        json.push('}');
        first_row = false;
    }
    json.push(']');

    Ok(json)
}

fn db_exec_impl(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<String, String> {
    if let Some(msg) = is_blocked_sql(sql) {
        return Err(format!("db.exec: {msg}"));
    }
    let changes = conn
        .execute(sql, rusqlite::params_from_iter(params))
        .map_err(|e| format!("db.exec: {e}"))?;

    Ok(format!("{{\"changes\":{changes}}}"))
}

fn sqlite_val_to_json(val: &rusqlite::types::Value) -> String {
    match val {
        rusqlite::types::Value::Null => "null".to_string(),
        rusqlite::types::Value::Integer(n) => n.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        rusqlite::types::Value::Text(s) => json_string(s),
        rusqlite::types::Value::Blob(b) => {
            let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
            json_string(&hex)
        }
    }
}

/// Parses a JSON array of simple values into rusqlite params.
fn parse_json_params(json: &str) -> Vec<rusqlite::types::Value> {
    let json = json.trim();
    if json.is_empty() || json == "[]" {
        return Vec::new();
    }

    let inner = json
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(json);

    let mut params = Vec::new();
    let mut remaining = inner.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.starts_with('"') {
            // String value
            let (val, rest) = parse_json_string_value(&remaining[1..]);
            params.push(rusqlite::types::Value::Text(val));
            remaining = rest.trim_start().strip_prefix(',').unwrap_or(rest);
        } else if remaining.starts_with("null") {
            params.push(rusqlite::types::Value::Null);
            remaining = remaining[4..].trim_start().strip_prefix(',').unwrap_or(&remaining[4..]);
        } else if remaining.starts_with("true") {
            params.push(rusqlite::types::Value::Integer(1));
            remaining = remaining[4..].trim_start().strip_prefix(',').unwrap_or(&remaining[4..]);
        } else if remaining.starts_with("false") {
            params.push(rusqlite::types::Value::Integer(0));
            remaining = remaining[5..].trim_start().strip_prefix(',').unwrap_or(&remaining[5..]);
        } else {
            // Number
            let end = remaining
                .find(|c: char| c == ',' || c == ']' || c.is_ascii_whitespace())
                .unwrap_or(remaining.len());
            let num_str = &remaining[..end];
            if num_str.contains('.') {
                if let Ok(f) = num_str.parse::<f64>() {
                    params.push(rusqlite::types::Value::Real(f));
                }
            } else if let Ok(n) = num_str.parse::<i64>() {
                params.push(rusqlite::types::Value::Integer(n));
            }
            remaining = remaining[end..].trim_start().strip_prefix(',').unwrap_or(&remaining[end..]);
        }
    }

    params
}

fn parse_json_string_value(input: &str) -> (String, &str) {
    let mut result = String::new();
    let mut chars = input.chars();
    let mut byte_offset = 0;
    while let Some(c) = chars.next() {
        byte_offset += c.len_utf8();
        match c {
            '"' => return (result, &input[byte_offset..]),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    byte_offset += escaped.len_utf8();
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
    (result, "")
}

// ---------------------------------------------------------------------------
// fetch() implementation
// ---------------------------------------------------------------------------

fn fetch_impl(url: &str, opts_json: &str, resolved_ip: Option<std::net::IpAddr>, timeout: Duration) -> Result<String, String> {
    let opts = opts_json.trim();
    let method = extract_json_string(opts, "method")
        .unwrap_or_else(|| "GET".to_string())
        .to_uppercase();
    let body = extract_json_string(opts, "body").unwrap_or_default();
    let headers: HashMap<String, String> = extract_json_object(opts, "headers")
        .into_iter()
        .filter(|(k, _)| !is_blocked_header(k))
        .collect();

    // For HTTP, rewrite URL with resolved IP to eliminate DNS rebinding TOCTOU.
    // HTTPS is protected by TLS certificate validation (SNI mismatch = handshake failure).
    let (effective_url, host_override) = if let Some(ip) = resolved_ip {
        if url.to_lowercase().starts_with("http://") {
            let original_host = extract_host(url).to_string();
            let ip_str = if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() };
            (replace_url_host(url, &ip_str), Some(original_host))
        } else {
            (url.to_string(), None)
        }
    } else {
        (url.to_string(), None)
    };

    let agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build()
        .new_agent();

    let response = match method.as_str() {
        "GET" | "HEAD" | "DELETE" => {
            let mut req = match method.as_str() {
                "HEAD" => agent.head(&effective_url),
                "DELETE" => agent.delete(&effective_url),
                _ => agent.get(&effective_url),
            };
            if let Some(ref host) = host_override {
                req = req.header("Host", host);
            }
            for (k, v) in &headers {
                req = req.header(k, v);
            }
            req.call().map_err(|e| format!("fetch: {e}"))?
        }
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method.as_str() {
                "PUT" => agent.put(&effective_url),
                "PATCH" => agent.patch(&effective_url),
                _ => agent.post(&effective_url),
            };
            if let Some(ref host) = host_override {
                req = req.header("Host", host);
            }
            for (k, v) in &headers {
                req = req.header(k, v);
            }
            if body.is_empty() {
                req.send("").map_err(|e| format!("fetch: {e}"))?
            } else {
                req.send(&body).map_err(|e| format!("fetch: {e}"))?
            }
        }
        _ => return Err(format!("fetch: unsupported method {method}")),
    };

    let status: u16 = response.status().into();
    let resp_body = response
        .into_body()
        .into_with_config()
        .limit(MAX_FETCH_RESPONSE as u64)
        .read_to_string()
        .map_err(|e| format!("fetch body: {e}"))?;

    Ok(format!(
        "{{\"status\":{status},\"body\":{resp}}}",
        resp = json_string(&resp_body)
    ))
}

// ---------------------------------------------------------------------------
// SSRF protection
// ---------------------------------------------------------------------------

fn is_blocked_header(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "host" | "transfer-encoding" | "content-length"
            | "connection" | "upgrade" | "proxy-authorization"
            | "te" | "trailer"
    )
}

/// Pre-resolve DNS, validate IPs, and return a validated IP for URL rewriting.
/// Fail-closed: DNS errors block the request (prevents silent bypass).
fn check_resolved_ips(url: &str) -> Result<Option<std::net::IpAddr>, String> {
    let host = extract_host(url);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(None);
    }
    use std::net::ToSocketAddrs;
    let addrs: Vec<std::net::SocketAddr> = (host, 80u16)
        .to_socket_addrs()
        .map_err(|e| format!("fetch: DNS resolution failed for {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("fetch: no addresses for {host}"));
    }
    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(format!("fetch: {host} resolves to private address"));
        }
    }
    Ok(Some(addrs[0].ip()))
}

fn is_private_url(url: &str) -> bool {
    // Scheme allowlist: only http(s)
    let lower_url = url.to_lowercase();
    if !lower_url.starts_with("http://") && !lower_url.starts_with("https://") {
        return true;
    }

    let host = extract_host(url);
    let host = host.trim_start_matches('[').trim_end_matches(']');

    if host.is_empty() {
        return true;
    }

    let lower = host.to_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower == "metadata.google.internal"
        || lower == "169.254.169.254"
    {
        return true;
    }

    // Block numeric-encoded IPs (decimal, hex, octal)
    if looks_like_numeric_ip(&lower) {
        return true;
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return is_private_ip(ip);
    }

    false
}

fn looks_like_numeric_ip(host: &str) -> bool {
    // Decimal: all digits (e.g., 2130706433)
    if !host.is_empty() && host.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // Hex: 0x prefix (e.g., 0x7f000001)
    if host.starts_with("0x") || host.starts_with("0X") {
        return true;
    }
    // Octal: starts with 0 and digits/dots only (e.g., 0177.0.0.1)
    if host.starts_with('0')
        && host.len() > 1
        && host.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return true;
    }
    false
}

fn extract_host(url: &str) -> &str {
    let after_scheme = url.find("://").map(|i| &url[i + 3..]).unwrap_or(url);
    // Isolate authority: stop at / ? # to prevent fragment confusion SSRF
    let authority_end = after_scheme
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // Strip userinfo (rfind handles multiple @ correctly per RFC 3986)
    let after_user = authority
        .rfind('@')
        .map(|i| &authority[i + 1..])
        .unwrap_or(authority);
    // Handle IPv6 bracket notation
    if after_user.starts_with('[') {
        let end = after_user.find(']').map(|i| i + 1).unwrap_or(after_user.len());
        return &after_user[..end];
    }
    // Strip port
    let end = after_user
        .find(':')
        .unwrap_or(after_user.len());
    &after_user[..end]
}

/// Replaces the host in a URL with a new value (used for DNS rebinding prevention).
fn replace_url_host(url: &str, new_host: &str) -> String {
    let old_host = extract_host(url);
    if old_host.is_empty() {
        return url.to_string();
    }
    // extract_host returns a slice of url, so pointer arithmetic gives exact position
    let start = old_host.as_ptr() as usize - url.as_ptr() as usize;
    let end = start + old_host.len();
    format!("{}{}{}", &url[..start], new_host, &url[end..])
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 169 && o[1] == 254)
                || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT
                || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            // IPv6-mapped IPv4 (::ffff:x.x.x.x)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ip(std::net::IpAddr::V4(v4));
            }
            // IPv4-compatible (::x.x.x.x, deprecated but parseable)
            let segs = v6.segments();
            if segs[0] == 0 && segs[1] == 0 && segs[2] == 0
                && segs[3] == 0 && segs[4] == 0 && segs[5] == 0 {
                let [a, b] = segs[6].to_be_bytes();
                let [c, d] = segs[7].to_be_bytes();
                let v4 = std::net::Ipv4Addr::new(a, b, c, d);
                if !v4.is_unspecified() {
                    return is_private_ip(std::net::IpAddr::V4(v4));
                }
            }
            let seg0 = v6.segments()[0];
            // Link-local: fe80::/10
            if (seg0 & 0xffc0) == 0xfe80 {
                return true;
            }
            // Unique local: fc00::/7
            if (seg0 & 0xfe00) == 0xfc00 {
                return true;
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn parse_response(json: &str) -> Result<FnResponse, String> {
    let status = extract_json_number(json, "status").unwrap_or(200);
    let body = extract_json_string(json, "body").unwrap_or_default();
    let headers = extract_json_object(json, "headers");
    // Log caught handler errors server-side (not exposed to client)
    if let Some(err) = extract_json_string(json, "__error") {
        eprintln!("handler error: {err}");
    }
    Ok(FnResponse { status, headers, body })
}

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
            c if c < '\x20' => out.push_str(&format!("\\u{:04x}", c as u32)),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Finds the value substring for a top-level key in a JSON object.
/// Skips over string values properly to avoid matching keys inside strings.
fn find_value_for_key<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let bytes = json.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut depth = 0u32;

    while i < len {
        match bytes[i] {
            b'"' => {
                let str_start = i + 1;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        break;
                    }
                    i += 1;
                }
                let str_end = i;
                i += 1;

                if depth == 1
                    && str_end - str_start == key.len()
                    && &json[str_start..str_end] == key
                {
                    while i < len && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < len && bytes[i] == b':' {
                        i += 1;
                        while i < len && bytes[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        return Some(&json[i..]);
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    None
}

fn extract_json_number(json: &str, key: &str) -> Option<u16> {
    let rest = find_value_for_key(json, key)?;
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let rest = find_value_for_key(json, key)?;
    if !rest.starts_with('"') {
        return None;
    }
    let (val, _) = parse_json_string_value(&rest[1..]);
    Some(val)
}

fn extract_json_object(json: &str, key: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(rest) = find_value_for_key(json, key) else {
        return map;
    };
    if !rest.starts_with('{') {
        return map;
    }
    let mut remaining = &rest[1..];
    loop {
        remaining = remaining.trim_start();
        if remaining.starts_with('}') || remaining.is_empty() {
            break;
        }
        if remaining.starts_with(',') {
            remaining = &remaining[1..];
            continue;
        }
        if !remaining.starts_with('"') {
            break;
        }
        let (k, after_key) = parse_json_string_value(&remaining[1..]);
        remaining = after_key.trim_start();
        if !remaining.starts_with(':') {
            break;
        }
        remaining = remaining[1..].trim_start();
        if !remaining.starts_with('"') {
            break;
        }
        let (v, after_val) = parse_json_string_value(&remaining[1..]);
        remaining = after_val;
        map.insert(k, v);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // is_private_ip
    // -----------------------------------------------------------------------

    #[test]
    fn private_ip_loopback() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("127.255.255.255".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
    }

    #[test]
    fn private_ip_rfc1918() {
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("10.255.255.255".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("172.31.255.255".parse().unwrap()));
        assert!(is_private_ip("192.168.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.255.255".parse().unwrap()));
    }

    #[test]
    fn private_ip_link_local() {
        assert!(is_private_ip("169.254.0.1".parse().unwrap()));
        assert!(is_private_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn private_ip_cgnat() {
        assert!(is_private_ip("100.64.0.1".parse().unwrap()));
        assert!(is_private_ip("100.127.255.255".parse().unwrap()));
    }

    #[test]
    fn private_ip_unspecified() {
        assert!(is_private_ip("0.0.0.0".parse().unwrap()));
        assert!(is_private_ip("::".parse().unwrap()));
    }

    #[test]
    fn private_ip_ipv6_mapped() {
        // ::ffff:127.0.0.1
        assert!(is_private_ip("::ffff:127.0.0.1".parse().unwrap()));
        // ::ffff:10.0.0.1
        assert!(is_private_ip("::ffff:10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn private_ip_ipv6_link_local() {
        assert!(is_private_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn private_ip_ipv6_unique_local() {
        assert!(is_private_ip("fc00::1".parse().unwrap()));
        assert!(is_private_ip("fd12:3456::1".parse().unwrap()));
    }

    #[test]
    fn public_ips_not_private() {
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip("93.184.216.34".parse().unwrap()));
        assert!(!is_private_ip("172.32.0.1".parse().unwrap())); // just outside 172.16-31
        assert!(!is_private_ip("100.128.0.1".parse().unwrap())); // just outside CGNAT
        assert!(!is_private_ip("2606:4700::1".parse().unwrap())); // public IPv6
    }

    // -----------------------------------------------------------------------
    // is_private_url
    // -----------------------------------------------------------------------

    #[test]
    fn private_url_localhost() {
        assert!(is_private_url("http://localhost/"));
        assert!(is_private_url("https://localhost:8080/path"));
    }

    #[test]
    fn private_url_local_suffix() {
        assert!(is_private_url("http://server.local/api"));
        assert!(is_private_url("http://router.internal/"));
    }

    #[test]
    fn private_url_metadata() {
        assert!(is_private_url("http://169.254.169.254/latest/meta-data/"));
        assert!(is_private_url("http://metadata.google.internal/"));
    }

    #[test]
    fn private_url_rfc1918_ip() {
        assert!(is_private_url("http://10.0.0.1/admin"));
        assert!(is_private_url("http://192.168.1.1/"));
    }

    #[test]
    fn private_url_non_http_schemes_blocked() {
        assert!(is_private_url("file:///etc/passwd"));
        assert!(is_private_url("ftp://internal-server/file"));
        assert!(is_private_url("gopher://evil.com/"));
    }

    #[test]
    fn public_urls_allowed() {
        assert!(!is_private_url("https://example.com/api"));
        assert!(!is_private_url("http://api.github.com/users"));
    }

    // -----------------------------------------------------------------------
    // looks_like_numeric_ip
    // -----------------------------------------------------------------------

    #[test]
    fn numeric_ip_decimal() {
        assert!(looks_like_numeric_ip("2130706433")); // 127.0.0.1 in decimal
    }

    #[test]
    fn numeric_ip_hex() {
        assert!(looks_like_numeric_ip("0x7f000001")); // 127.0.0.1 in hex
        assert!(looks_like_numeric_ip("0X7F000001"));
    }

    #[test]
    fn numeric_ip_octal() {
        assert!(looks_like_numeric_ip("0177.0.0.1")); // 127.0.0.1 in octal
    }

    #[test]
    fn not_numeric_ip() {
        assert!(!looks_like_numeric_ip("example.com"));
        assert!(!looks_like_numeric_ip("api.github.com"));
        assert!(!looks_like_numeric_ip("")); // empty
    }

    // -----------------------------------------------------------------------
    // extract_host
    // -----------------------------------------------------------------------

    #[test]
    fn extract_host_simple() {
        assert_eq!(extract_host("http://example.com/path"), "example.com");
    }

    #[test]
    fn extract_host_with_port() {
        assert_eq!(extract_host("http://example.com:8080/path"), "example.com");
    }

    #[test]
    fn extract_host_with_userinfo() {
        assert_eq!(extract_host("http://user:pass@example.com/path"), "example.com");
    }

    #[test]
    fn extract_host_ipv6() {
        assert_eq!(extract_host("http://[::1]:8080/path"), "[::1]");
    }

    #[test]
    fn extract_host_query_fragment() {
        assert_eq!(extract_host("http://example.com?foo=bar"), "example.com");
        assert_eq!(extract_host("http://example.com#frag"), "example.com");
    }

    // -----------------------------------------------------------------------
    // replace_url_host
    // -----------------------------------------------------------------------

    #[test]
    fn replace_host_basic() {
        assert_eq!(
            replace_url_host("http://example.com/path", "1.2.3.4"),
            "http://1.2.3.4/path"
        );
    }

    #[test]
    fn replace_host_with_port() {
        assert_eq!(
            replace_url_host("http://example.com:8080/path", "1.2.3.4"),
            "http://1.2.3.4:8080/path"
        );
    }

    // -----------------------------------------------------------------------
    // is_blocked_sql
    // -----------------------------------------------------------------------

    #[test]
    fn blocked_sql_vacuum() {
        assert!(is_blocked_sql("VACUUM").is_some());
        assert!(is_blocked_sql("  VACUUM  ").is_some());
    }

    #[test]
    fn blocked_sql_vacuum_comment_bypass() {
        assert!(is_blocked_sql("-- bypass\nVACUUM").is_some());
        assert!(is_blocked_sql("/* bypass */VACUUM").is_some());
    }

    #[test]
    fn allowed_sql() {
        assert!(is_blocked_sql("SELECT * FROM t").is_none());
        assert!(is_blocked_sql("INSERT INTO t VALUES (1)").is_none());
        assert!(is_blocked_sql("CREATE TABLE t (id INTEGER)").is_none());
    }

    // -----------------------------------------------------------------------
    // is_blocked_header
    // -----------------------------------------------------------------------

    #[test]
    fn blocked_headers() {
        assert!(is_blocked_header("Host"));
        assert!(is_blocked_header("transfer-encoding"));
        assert!(is_blocked_header("Connection"));
    }

    #[test]
    fn allowed_headers() {
        assert!(!is_blocked_header("Authorization"));
        assert!(!is_blocked_header("Content-Type"));
        assert!(!is_blocked_header("Accept"));
    }

    // -----------------------------------------------------------------------
    // resolve_function
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_strips_api_prefix() {
        // This test checks the path logic, actual file lookup requires filesystem
        assert_eq!(resolve_function("/api/"), None); // empty after strip
    }

    #[test]
    fn resolve_blocks_traversal() {
        assert_eq!(resolve_function("/api/../etc/passwd"), None);
    }

    // -----------------------------------------------------------------------
    // parse_json_params
    // -----------------------------------------------------------------------

    #[test]
    fn parse_params_empty() {
        assert!(parse_json_params("").is_empty());
        assert!(parse_json_params("[]").is_empty());
    }

    #[test]
    fn parse_params_mixed() {
        let params = parse_json_params(r#"["hello", 42, null, true, 3.14]"#);
        assert_eq!(params.len(), 5);
        assert!(matches!(&params[0], rusqlite::types::Value::Text(s) if s == "hello"));
        assert!(matches!(&params[1], rusqlite::types::Value::Integer(42)));
        assert!(matches!(&params[2], rusqlite::types::Value::Null));
        assert!(matches!(&params[3], rusqlite::types::Value::Integer(1))); // true → 1
        assert!(matches!(&params[4], rusqlite::types::Value::Real(f) if (*f - 3.14).abs() < 0.001));
    }

    // -----------------------------------------------------------------------
    // json_string
    // -----------------------------------------------------------------------

    #[test]
    fn json_string_escapes() {
        assert_eq!(json_string("hello"), "\"hello\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
    }
}
