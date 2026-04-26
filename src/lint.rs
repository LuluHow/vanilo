use std::fs;
use std::path::Path;

/// A lint warning with file location and message.
struct Warning {
    file: String,
    line: usize,
    message: String,
    severity: Severity,
}

#[derive(Debug, PartialEq)]
enum Severity {
    Error,
    Warn,
}

/// Runs all lint checks on pages, components, and functions.
/// Returns Err with a summary if any errors are found.
pub fn check(
    pages_dir: &Path,
    components_dir: &Path,
    functions_dir: &Path,
) -> Result<(), String> {
    let mut warnings: Vec<Warning> = Vec::new();

    // Lint HTML files (pages + components)
    if pages_dir.exists() {
        lint_html_dir(pages_dir, &mut warnings);
    }
    if components_dir.exists() {
        lint_html_dir(components_dir, &mut warnings);
    }

    // Lint JS edge functions
    if functions_dir.exists() {
        lint_js_dir(functions_dir, &mut warnings);
    }

    // Print warnings
    let errors: Vec<&Warning> = warnings.iter().filter(|w| w.severity == Severity::Error).collect();
    let warns: Vec<&Warning> = warnings.iter().filter(|w| w.severity == Severity::Warn).collect();

    for w in &warns {
        eprintln!("  warn: {}:{} — {}", w.file, w.line, w.message);
    }
    for w in &errors {
        eprintln!("  error: {}:{} — {}", w.file, w.line, w.message);
    }

    if !errors.is_empty() {
        return Err(format!(
            "build blocked: {} error(s), {} warning(s)",
            errors.len(),
            warns.len()
        ));
    }

    if !warns.is_empty() {
        eprintln!("  {} warning(s)", warns.len());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// HTML linting
// ---------------------------------------------------------------------------

fn lint_html_dir(dir: &Path, warnings: &mut Vec<Warning>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            lint_html_dir(&path, warnings);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            let file = path.display().to_string();
            lint_html(&file, &content, warnings);
        }
    }
}

fn lint_html(file: &str, content: &str, warnings: &mut Vec<Warning>) {
    for (i, line) in content.lines().enumerate() {
        let lineno = i + 1;
        let lower = line.to_lowercase();
        let trimmed = lower.trim();

        // 1. {{prop}} in event handler attributes (onclick, onload, onerror, etc.)
        if has_template_in_event_handler(line) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "{{prop}} in event handler — XSS risk, use data attributes + JS instead"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        // 2. {{prop}} inside javascript: URI
        if lower.contains("javascript:") && line.contains("{{") {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "{{prop}} in javascript: URI — XSS risk".to_string(),
                severity: Severity::Error,
            });
        }

        // 3. <script> tag containing {{prop}}
        if trimmed.starts_with("<script") && line.contains("{{") && line.contains("}}") {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "{{prop}} inside <script> tag — injection risk, use data attributes"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        // 4. Inline style with {{prop}} (CSS injection)
        if has_template_in_style_attr(line) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "{{prop}} in style attribute — CSS injection risk".to_string(),
                severity: Severity::Error,
            });
        }

        // 5. <img> without alt attribute
        if trimmed.starts_with("<img ") || trimmed.starts_with("<img>") {
            if !lower.contains("alt=") {
                warnings.push(Warning {
                    file: file.to_string(),
                    line: lineno,
                    message: "<img> missing alt attribute (accessibility + Lighthouse)"
                        .to_string(),
                    severity: Severity::Warn,
                });
            }
        }

        // 6. <a target="_blank"> without rel="noopener"
        if lower.contains("target=\"_blank\"") || lower.contains("target='_blank'") {
            if !lower.contains("rel=") || !lower.contains("noopener") {
                warnings.push(Warning {
                    file: file.to_string(),
                    line: lineno,
                    message: "<a target=\"_blank\"> without rel=\"noopener\" — security risk"
                        .to_string(),
                    severity: Severity::Warn,
                });
            }
        }
    }

    // Multi-line: detect {{prop}} between <script>...</script>
    check_template_in_script_block(file, content, warnings);
}

/// Detects {{prop}} inside on* event handler attributes.
fn has_template_in_event_handler(line: &str) -> bool {
    let lower = line.to_lowercase();
    let event_attrs = [
        "onclick=", "onload=", "onerror=", "onsubmit=", "onchange=",
        "onmouseover=", "onmouseout=", "onfocus=", "onblur=", "oninput=",
        "onkeydown=", "onkeyup=", "onkeypress=",
    ];

    for attr in &event_attrs {
        if let Some(pos) = lower.find(attr) {
            // Check if there's a {{...}} in the attribute value
            let after = &line[pos + attr.len()..];
            if let Some(end) = find_attr_value_end(after) {
                let value = &after[..end];
                if value.contains("{{") && value.contains("}}") {
                    return true;
                }
            }
        }
    }
    false
}

/// Detects {{prop}} inside style="..." attributes.
fn has_template_in_style_attr(line: &str) -> bool {
    let lower = line.to_lowercase();
    if let Some(pos) = lower.find("style=") {
        let after = &line[pos + 6..];
        if let Some(end) = find_attr_value_end(after) {
            let value = &after[..end];
            if value.contains("{{") && value.contains("}}") {
                return true;
            }
        }
    }
    false
}

/// Returns the end position of a quoted attribute value (including the opening quote).
fn find_attr_value_end(s: &str) -> Option<usize> {
    let s = s.trim_start();
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &s[1..];
    rest.find(quote).map(|i| 1 + i + 1) // +1 for opening quote, +1 for closing
}

/// Detects {{prop}} between <script> and </script> across multiple lines.
fn check_template_in_script_block(file: &str, content: &str, warnings: &mut Vec<Warning>) {
    let lower = content.to_lowercase();
    let mut search_from = 0;

    while let Some(open) = lower[search_from..].find("<script") {
        let abs_open = search_from + open;
        // Find the > that closes the opening tag
        let tag_end = match lower[abs_open..].find('>') {
            Some(i) => abs_open + i + 1,
            None => break,
        };
        let close = match lower[tag_end..].find("</script>") {
            Some(i) => tag_end + i,
            None => break,
        };

        let block = &content[tag_end..close];
        if block.contains("{{") && block.contains("}}") {
            // Find the line number of the first {{
            let offset = tag_end + block.find("{{").unwrap();
            let lineno = content[..offset].matches('\n').count() + 1;
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "{{prop}} inside <script> block — injection risk, use data attributes"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        search_from = close + 9;
    }
}

// ---------------------------------------------------------------------------
// JS linting
// ---------------------------------------------------------------------------

fn lint_js_dir(dir: &Path, warnings: &mut Vec<Warning>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            lint_js_dir(&path, warnings);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            let file = path.display().to_string();
            lint_js(&file, &content, warnings);
        }
    }
}

fn lint_js(file: &str, content: &str, warnings: &mut Vec<Warning>) {
    for (i, line) in content.lines().enumerate() {
        let lineno = i + 1;
        let trimmed = line.trim();

        // Skip comment lines
        if trimmed.starts_with("//") {
            continue;
        }

        // 1. SQL string concatenation: db.query("..."+  or  db.exec("..."+
        if has_sql_concat(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "SQL string concatenation — use parameterized queries: db.query(\"... WHERE id = ?\", [id])"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        // 2. eval() usage
        if has_eval(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "eval() is dangerous — code injection risk".to_string(),
                severity: Severity::Error,
            });
        }

        // 3. new Function() — dynamic code execution
        if trimmed.contains("new Function(") || trimmed.contains("new Function (") {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "new Function() is dangerous — code injection risk".to_string(),
                severity: Severity::Error,
            });
        }

        // 4. innerHTML assignment with dynamic content
        if has_innerhtml_assign(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "innerHTML with dynamic content — XSS risk, use textContent instead"
                    .to_string(),
                severity: Severity::Warn,
            });
        }

        // 5. Unescaped user input in response body (string concat with req.body/req.query)
        if has_unescaped_req_in_response(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "req.body/req.query concatenated into response — escape or validate first"
                    .to_string(),
                severity: Severity::Warn,
            });
        }

        // 6. setTimeout/setInterval with string first argument (equivalent to eval)
        if has_settimeout_string(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "setTimeout/setInterval with string argument — code injection risk, use a function instead"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        // 7. document.write — XSS sink
        if trimmed.contains("document.write(") || trimmed.contains("document.writeln(") {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "document.write() is dangerous — XSS risk, use DOM methods instead"
                    .to_string(),
                severity: Severity::Error,
            });
        }

        // 8. outerHTML assignment with dynamic content
        if has_outerhtml_assign(trimmed) {
            warnings.push(Warning {
                file: file.to_string(),
                line: lineno,
                message: "outerHTML with dynamic content — XSS risk, use DOM methods instead"
                    .to_string(),
                severity: Severity::Warn,
            });
        }
    }
}

/// Detects patterns like: db.query("SELECT..." + variable  or  db.exec(`...${var}...`)
/// Only inspects the SQL string (first argument), not the params array.
fn has_sql_concat(line: &str) -> bool {
    let db_call = if let Some(pos) = line.find("db.query(") {
        Some(pos + 9)
    } else {
        line.find("db.exec(").map(|pos| pos + 8)
    };

    let Some(start) = db_call else { return false };
    let after = &line[start..];

    // Extract only the first argument (the SQL string) by finding the end of the
    // first string literal, then checking for concat before the params separator.
    // Look for ", [" or ", )" which marks the boundary between SQL and params.
    let sql_arg = if let Some(sep) = find_params_separator(after) {
        &after[..sep]
    } else {
        after
    };

    // Check for string concat with +
    if sql_arg.contains("\" +") || sql_arg.contains("' +") || sql_arg.contains("+ \"") || sql_arg.contains("+ '") {
        return true;
    }

    // Check for template literals with ${
    if sql_arg.contains('`') && sql_arg.contains("${") {
        return true;
    }

    false
}

/// Finds the position of the params separator (`, [` or end `)`) after the first
/// string argument, skipping over quoted content.
fn find_params_separator(s: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut in_quote: Option<u8> = None;

    while i < len {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == b'\\' {
                i += 2; // skip escaped char
                continue;
            }
            if b == q {
                in_quote = None;
            }
        } else {
            if b == b'"' || b == b'\'' || b == b'`' {
                in_quote = Some(b);
            } else if b == b',' {
                // Found separator between SQL string and params
                return Some(i);
            } else if b == b')' {
                // End of call — no params
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Detects eval() calls (but not "evaluation" or similar words).
fn has_eval(line: &str) -> bool {
    let mut search = line;
    while let Some(pos) = search.find("eval(") {
        // Make sure it's not part of a larger identifier
        if pos > 0 {
            let prev = search.as_bytes()[pos - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' {
                search = &search[pos + 5..];
                continue;
            }
        }
        return true;
    }
    false
}

/// Detects innerHTML = ... with dynamic content (not a plain string literal).
fn has_innerhtml_assign(line: &str) -> bool {
    if let Some(pos) = line.find("innerHTML") {
        let after = line[pos + 9..].trim_start();
        if after.starts_with('=') && !after.starts_with("==") {
            let rhs = after[1..].trim_start();
            // Allow pure string literals
            if (rhs.starts_with('"') || rhs.starts_with('\''))
                && !rhs.contains('+')
                && !rhs.contains("${")
            {
                return false;
            }
            return true;
        }
    }
    false
}

/// Detects setTimeout("string", ...) or setInterval("string", ...) — eval equivalent.
fn has_settimeout_string(line: &str) -> bool {
    for func in &["setTimeout(", "setInterval("] {
        if let Some(pos) = line.find(func) {
            let after = line[pos + func.len()..].trim_start();
            if after.starts_with('"') || after.starts_with('\'') {
                return true;
            }
        }
    }
    false
}

/// Detects outerHTML = ... with dynamic content.
fn has_outerhtml_assign(line: &str) -> bool {
    if let Some(pos) = line.find("outerHTML") {
        let after = line[pos + 9..].trim_start();
        if after.starts_with('=') && !after.starts_with("==") {
            let rhs = after[1..].trim_start();
            if (rhs.starts_with('"') || rhs.starts_with('\''))
                && !rhs.contains('+')
                && !rhs.contains("${")
            {
                return false;
            }
            return true;
        }
    }
    false
}

/// Detects req.body or req.query used in string concatenation for response building.
fn has_unescaped_req_in_response(line: &str) -> bool {
    if !line.contains("req.body") && !line.contains("req.query") {
        return false;
    }
    // Flag if concatenated with + into a string
    line.contains("\" + req.body") || line.contains("\" + req.query")
        || line.contains("req.body + \"") || line.contains("req.query + \"")
        || (line.contains('`') && (line.contains("${req.body") || line.contains("${req.query")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sql_concat() {
        assert!(has_sql_concat(r#"db.query("SELECT * FROM users WHERE id = " + id)"#));
        assert!(has_sql_concat(r#"db.exec(`INSERT INTO t VALUES (${val})`)"#));
        assert!(!has_sql_concat(r#"db.query("SELECT * FROM users WHERE id = ?", [id])"#));
    }

    #[test]
    fn detects_eval() {
        assert!(has_eval("eval(userInput)"));
        assert!(has_eval("  eval('code')"));
        assert!(!has_eval("// evaluation of results"));
        assert!(!has_eval("someeval(x)"));
    }

    #[test]
    fn detects_innerhtml() {
        assert!(has_innerhtml_assign("el.innerHTML = data"));
        assert!(has_innerhtml_assign("el.innerHTML = '<b>' + name"));
        assert!(!has_innerhtml_assign("el.innerHTML = \"<b>static</b>\""));
    }

    #[test]
    fn detects_event_handler_template() {
        assert!(has_template_in_event_handler(r#"<button onclick="do({{action}})">"#));
        assert!(!has_template_in_event_handler(r#"<button class="{{cls}}">"#));
    }

    #[test]
    fn detects_style_template() {
        assert!(has_template_in_style_attr(r#"<div style="color: {{color}}">"#));
        assert!(!has_template_in_style_attr(r#"<div class="{{cls}}">"#));
    }

    #[test]
    fn detects_settimeout_string() {
        assert!(has_settimeout_string(r#"setTimeout("alert(1)", 100)"#));
        assert!(has_settimeout_string(r#"setInterval('tick()', 1000)"#));
        assert!(!has_settimeout_string("setTimeout(function() {}, 100)"));
        assert!(!has_settimeout_string("setTimeout(() => {}, 100)"));
    }

    #[test]
    fn detects_outerhtml() {
        assert!(has_outerhtml_assign("el.outerHTML = data"));
        assert!(!has_outerhtml_assign("el.outerHTML = \"<b>static</b>\""));
        assert!(!has_outerhtml_assign("x = el.outerHTML"));
    }

    // -----------------------------------------------------------------------
    // SQL concat edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn sql_concat_template_literal() {
        assert!(has_sql_concat(r#"db.query(`SELECT * FROM ${table}`)"#));
        assert!(has_sql_concat(r#"db.exec(`INSERT INTO users (name) VALUES ('${name}')`)"#));
    }

    #[test]
    fn sql_concat_string_plus() {
        assert!(has_sql_concat(r#"db.query("SELECT * FROM users WHERE name = '" + name + "'")"#));
    }

    #[test]
    fn sql_parameterized_is_safe() {
        assert!(!has_sql_concat(r#"db.query("SELECT * FROM users WHERE id = ?", [id])"#));
        assert!(!has_sql_concat(r#"db.exec("INSERT INTO t VALUES (?, ?)", [a, b])"#));
        assert!(!has_sql_concat(r#"db.query("SELECT count FROM visits")"#));
    }

    #[test]
    fn sql_concat_in_params_is_safe() {
        // String concat inside the params array is NOT SQL injection
        assert!(!has_sql_concat(r#"db.query("SELECT * FROM t WHERE name LIKE ?", ["%" + q + "%"])"#));
        assert!(!has_sql_concat(r#"db.exec("INSERT INTO t VALUES (?)", [a + b])"#));
    }

    // -----------------------------------------------------------------------
    // eval edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn eval_in_context() {
        assert!(has_eval("var x = eval(input)"));
        assert!(has_eval("return eval(code)"));
        // Comments are filtered at lint_js level, not has_eval
        assert!(!has_eval("myeval(x)")); // part of identifier
        assert!(!has_eval("obj.eval(x)")); // method call, not global eval
    }

    // -----------------------------------------------------------------------
    // innerHTML edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn innerhtml_with_template_literal() {
        assert!(has_innerhtml_assign("el.innerHTML = `<b>${name}</b>`"));
    }

    #[test]
    fn innerhtml_comparison_is_safe() {
        assert!(!has_innerhtml_assign("if (el.innerHTML == 'test')"));
        assert!(!has_innerhtml_assign("if (el.innerHTML === '')"));
    }

    // -----------------------------------------------------------------------
    // HTML template injection
    // -----------------------------------------------------------------------

    #[test]
    fn template_in_various_event_handlers() {
        assert!(has_template_in_event_handler(r#"<div onload="{{fn}}">"#));
        assert!(has_template_in_event_handler(r#"<img onerror="{{handler}}">"#));
        assert!(has_template_in_event_handler(r#"<form onsubmit="{{action}}">"#));
        assert!(has_template_in_event_handler(r#"<input onchange="{{cb}}">"#));
    }

    #[test]
    fn template_in_safe_attributes() {
        assert!(!has_template_in_event_handler(r#"<div id="{{id}}">"#));
        assert!(!has_template_in_event_handler(r#"<a href="{{url}}">"#));
        assert!(!has_template_in_event_handler(r#"<span class="{{cls}}">"#));
        assert!(!has_template_in_event_handler(r#"<p data-value="{{val}}">"#));
    }

    // -----------------------------------------------------------------------
    // Script block detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_template_in_script_block() {
        let mut warnings = Vec::new();
        check_template_in_script_block(
            "test.html",
            "<script>\nvar x = {{value}};\n</script>",
            &mut warnings,
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Error);
    }

    #[test]
    fn no_template_in_script_is_safe() {
        let mut warnings = Vec::new();
        check_template_in_script_block(
            "test.html",
            "<script>\nvar x = 42;\n</script>",
            &mut warnings,
        );
        assert!(warnings.is_empty());
    }

    // -----------------------------------------------------------------------
    // Unescaped req in response
    // -----------------------------------------------------------------------

    #[test]
    fn detects_req_body_concat() {
        assert!(has_unescaped_req_in_response(r#"body: "<p>" + req.body"#));
        assert!(has_unescaped_req_in_response(r#"body: `<div>${req.query}</div>`"#));
    }

    #[test]
    fn req_in_safe_context() {
        // JSON.stringify is safe
        assert!(!has_unescaped_req_in_response("body: JSON.stringify(req.body)"));
        // Plain variable reference without concat
        assert!(!has_unescaped_req_in_response("var data = req.body"));
    }

    // -----------------------------------------------------------------------
    // Full lint integration
    // -----------------------------------------------------------------------

    #[test]
    fn lint_integration_clean_files() {
        let dir = std::env::temp_dir().join(format!("lint_clean_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let pages = dir.join("pages");
        let comps = dir.join("components");
        let funcs = dir.join("functions");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&comps).unwrap();
        std::fs::create_dir_all(&funcs).unwrap();

        std::fs::write(
            pages.join("index.html"),
            r#"<div class="{{cls}}"><p>hello</p></div>"#,
        )
        .unwrap();
        std::fs::write(
            funcs.join("api.js"),
            r#"function handler(req) { return { body: db.query("SELECT * FROM t WHERE id = ?", [req.body]) } }"#,
        )
        .unwrap();

        let result = check(&pages, &comps, &funcs);
        assert!(result.is_ok(), "clean files should pass lint");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lint_integration_blocks_sql_injection() {
        let dir = std::env::temp_dir().join(format!("lint_sqli_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let pages = dir.join("pages");
        let comps = dir.join("components");
        let funcs = dir.join("functions");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&comps).unwrap();
        std::fs::create_dir_all(&funcs).unwrap();

        std::fs::write(
            funcs.join("bad.js"),
            r#"function handler(req) { return { body: db.query("SELECT * FROM t WHERE id = " + req.body) } }"#,
        )
        .unwrap();

        let result = check(&pages, &comps, &funcs);
        assert!(result.is_err(), "SQL concat should block build");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lint_integration_blocks_xss_in_event_handler() {
        let dir = std::env::temp_dir().join(format!("lint_xss_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let pages = dir.join("pages");
        let comps = dir.join("components");
        let funcs = dir.join("functions");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&comps).unwrap();
        std::fs::create_dir_all(&funcs).unwrap();

        std::fs::write(
            pages.join("xss.html"),
            r#"<button onclick="do({{action}})">click</button>"#,
        )
        .unwrap();

        let result = check(&pages, &comps, &funcs);
        assert!(result.is_err(), "XSS in event handler should block build");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
