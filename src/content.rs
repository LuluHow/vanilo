use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Value type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Navigate a dot-separated path: `"contact.email"` or array index `"0"`.
    pub fn get(&self, path: &str) -> Option<&Value> {
        let mut current = self;
        for key in path.split('.') {
            match current {
                Value::Object(pairs) => {
                    current = pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)?;
                }
                Value::Array(items) => {
                    let idx: usize = key.parse().ok()?;
                    current = items.get(idx)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }

    /// Converts to a display string (for template interpolation).
    pub fn as_str(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format_number(*n),
            Value::Str(s) => s.clone(),
            Value::Array(_) | Value::Object(_) => to_json(self),
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Value::Array(items) = self {
            Some(items)
        } else {
            None
        }
    }
}

fn format_number(n: f64) -> String {
    if n == (n as i64) as f64 {
        (n as i64).to_string()
    } else {
        n.to_string()
    }
}

fn to_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(*n),
        Value::Str(s) => format!("\"{}\"", escape_json_str(s)),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(to_json).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(pairs) => {
            let inner: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", escape_json_str(k), to_json(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// JSON parser (recursive descent)
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<Value, String> {
        self.skip_ws();
        let value = self.value()?;
        self.skip_ws();
        if self.pos < self.input.len() {
            return Err(format!(
                "trailing content at position {}",
                self.pos
            ));
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => self.string().map(Value::Str),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b't') | Some(b'f') => self.boolean(),
            Some(b'n') => self.null(),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected '{}' at {}", *c as char, self.pos)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s = String::new();
        loop {
            match self.next_byte() {
                Some(b'"') => return Ok(s),
                Some(b'\\') => match self.next_byte() {
                    Some(b'"') => s.push('"'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'/') => s.push('/'),
                    Some(b'n') => s.push('\n'),
                    Some(b'r') => s.push('\r'),
                    Some(b't') => s.push('\t'),
                    Some(b'b') => s.push('\u{0008}'),
                    Some(b'f') => s.push('\u{000C}'),
                    Some(b'u') => {
                        let hex = self.take_n(4)?;
                        let code = u32::from_str_radix(&hex, 16)
                            .map_err(|_| format!("invalid unicode: \\u{hex}"))?;
                        if let Some(c) = char::from_u32(code) {
                            s.push(c);
                        }
                    }
                    Some(c) => return Err(format!("invalid escape: \\{}", c as char)),
                    None => return Err("unterminated string".to_string()),
                },
                Some(c) if c < 0x80 => s.push(c as char),
                Some(_) => {
                    // Multi-byte UTF-8: rewind and decode
                    self.pos -= 1;
                    let rest = std::str::from_utf8(&self.input[self.pos..])
                        .map_err(|e| format!("invalid UTF-8: {e}"))?;
                    let ch = rest.chars().next().unwrap();
                    self.pos += ch.len_utf8();
                    s.push(ch);
                }
                None => return Err("unterminated string".to_string()),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut pairs = Vec::new();
        if self.peek() == Some(&b'}') {
            self.pos += 1;
            return Ok(Value::Object(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let val = self.value()?;
            pairs.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(pairs));
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.pos)),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(&b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let val = self.value()?;
            items.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at {}", self.pos)),
            }
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(&b'-') {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(&b'.') {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if self.peek() == Some(&b'e') || self.peek() == Some(&b'E') {
            self.pos += 1;
            if self.peek() == Some(&b'+') || self.peek() == Some(&b'-') {
                self.pos += 1;
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|e| format!("invalid number: {e}"))?;
        let n: f64 = s.parse().map_err(|e| format!("invalid number '{s}': {e}"))?;
        Ok(Value::Number(n))
    }

    fn boolean(&mut self) -> Result<Value, String> {
        if self.input[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Value::Bool(true))
        } else if self.input[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Value::Bool(false))
        } else {
            Err(format!("expected boolean at {}", self.pos))
        }
    }

    fn null(&mut self) -> Result<Value, String> {
        if self.input[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(Value::Null)
        } else {
            Err(format!("expected null at {}", self.pos))
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<&u8> {
        self.input.get(self.pos)
    }

    fn next_byte(&mut self) -> Option<u8> {
        if self.pos < self.input.len() {
            let c = self.input[self.pos];
            self.pos += 1;
            Some(c)
        } else {
            None
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        match self.next_byte() {
            Some(b) if b == c => Ok(()),
            Some(b) => Err(format!(
                "expected '{}', got '{}' at {}",
                c as char, b as char, self.pos - 1
            )),
            None => Err(format!("expected '{}', got end of input", c as char)),
        }
    }

    fn take_n(&mut self, n: usize) -> Result<String, String> {
        if self.pos + n > self.input.len() {
            return Err("unexpected end of input".to_string());
        }
        let s = std::str::from_utf8(&self.input[self.pos..self.pos + n])
            .map_err(|e| format!("invalid UTF-8: {e}"))?;
        self.pos += n;
        Ok(s.to_string())
    }
}

pub fn parse_json(input: &str) -> Result<Value, String> {
    Parser::new(input).parse()
}

// ---------------------------------------------------------------------------
// Content loading
// ---------------------------------------------------------------------------

/// Loads all `.json` files from the content directory.
/// Returns a map of filename (without extension) → parsed Value.
pub fn load(dir: &Path) -> Result<HashMap<String, Value>, String> {
    let mut content = HashMap::new();

    if !dir.exists() {
        return Ok(content);
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid filename: {}", path.display()))?
            .to_string();

        let text =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;

        let value =
            parse_json(&text).map_err(|e| format!("{}: {e}", path.display()))?;

        content.insert(name, value);
    }

    Ok(content)
}

// ---------------------------------------------------------------------------
// Template resolution
// ---------------------------------------------------------------------------

/// Resolves a content reference like `"site.contact.email"` against the content map.
/// First segment is the filename, rest is the dot-path within the value.
fn resolve_ref<'a>(content: &'a HashMap<String, Value>, path: &str) -> Option<&'a Value> {
    let (file, rest) = match path.find('.') {
        Some(i) => (&path[..i], Some(&path[i + 1..])),
        None => (path, None),
    };
    let value = content.get(file)?;
    match rest {
        Some(subpath) => value.get(subpath),
        None => Some(value),
    }
}

/// Replaces content references in HTML:
/// - `{{@path}}` → HTML-escaped value (safe default)
/// - `{{{@path}}}` → raw unescaped value (opt-in for trusted HTML)
/// Unresolved references are removed (empty string).
pub fn resolve_placeholders(html: &str, content: &HashMap<String, Value>) -> String {
    let mut result = String::with_capacity(html.len());
    let mut remaining = html;

    loop {
        let Some(pos) = remaining.find("{{") else {
            result.push_str(remaining);
            break;
        };

        let after = &remaining[pos..];

        if after.starts_with("{{{@") {
            // Raw (unescaped): {{{@path}}}
            result.push_str(&remaining[..pos]);
            let path_start = &after[4..];
            if let Some(end) = path_start.find("}}}") {
                let path = path_start[..end].trim();
                if let Some(value) = resolve_ref(content, path) {
                    result.push_str(&value.as_str());
                }
                remaining = &path_start[end + 3..];
            } else {
                result.push_str("{{{@");
                remaining = path_start;
            }
        } else if after.starts_with("{{@") {
            // Escaped: {{@path}}
            result.push_str(&remaining[..pos]);
            let path_start = &after[3..];
            if let Some(end) = path_start.find("}}") {
                let path = path_start[..end].trim();
                if let Some(value) = resolve_ref(content, path) {
                    result.push_str(&crate::component::escape_html(&value.as_str()));
                }
                remaining = &path_start[end + 2..];
            } else {
                result.push_str("{{@");
                remaining = path_start;
            }
        } else {
            // Not a content reference (e.g. {{prop}}), pass through
            result.push_str(&remaining[..pos + 2]);
            remaining = &after[2..];
        }
    }

    result
}

/// Expands `<Each content="name">...</Each>` blocks.
/// For each item in the referenced array, renders the inner template with the item's properties.
pub fn expand_each(html: &str, content: &HashMap<String, Value>) -> String {
    let mut result = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(start) = remaining.find("<Each ") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start..];

        if let Some((content_path, inner, after)) = parse_each_tag(remaining) {
            if let Some(value) = resolve_ref(content, &content_path) {
                if let Some(items) = value.as_array() {
                    for item in items {
                        if let Value::Object(pairs) = item {
                            let mut rendered = inner.to_string();
                            for (key, val) in pairs {
                                // Raw (unescaped): {{{key}}} — must be replaced BEFORE {{key}}
                                let mut raw_ph = String::with_capacity(key.len() + 6);
                                raw_ph.push_str("{{{");
                                raw_ph.push_str(key);
                                raw_ph.push_str("}}}");
                                rendered = rendered.replace(&raw_ph, &val.as_str());

                                // Escaped: {{key}}
                                let placeholder = format!("{{{{{key}}}}}");
                                rendered = rendered.replace(&placeholder, &crate::component::escape_html(&val.as_str()));
                            }
                            result.push_str(&rendered);
                        }
                    }
                }
            }
            remaining = after;
        } else {
            // Could not parse — skip and move on
            result.push_str("<Each ");
            remaining = &remaining[6..];
        }
    }

    result.push_str(remaining);
    result
}

/// Parses `<Each content="name">inner</Each>`.
/// Returns `(content_path, inner_html, text_after_close)`.
fn parse_each_tag(input: &str) -> Option<(String, &str, &str)> {
    let rest = input.strip_prefix("<Each ")?;

    // Find content="..."
    let attr_pos = rest.find("content=")?;
    let after_eq = &rest[attr_pos + 8..];
    let quote = after_eq.as_bytes().first()?;
    if *quote != b'"' && *quote != b'\'' {
        return None;
    }
    let q = *quote;
    let val_start = 1;
    let val_end = after_eq[val_start..].find(q as char)?;
    let content_path = after_eq[val_start..val_start + val_end].to_string();

    // Find closing > of opening tag
    let tag_close = rest.find('>')?;
    let inner_start = tag_close + 1;

    // Find </Each>
    let close_tag = "</Each>";
    let close_pos = rest[inner_start..].find(close_tag)?;
    let inner = &rest[inner_start..inner_start + close_pos];
    let after = &rest[inner_start + close_pos + close_tag.len()..];

    Some((content_path, inner, after))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- JSON parser ---------------------------------------------------------

    #[test]
    fn parse_string() {
        let v = parse_json(r#""hello""#).unwrap();
        assert_eq!(v.as_str(), "hello");
    }

    #[test]
    fn parse_string_escapes() {
        let v = parse_json(r#""line\nbreak""#).unwrap();
        assert_eq!(v.as_str(), "line\nbreak");
    }

    #[test]
    fn parse_unicode_escape() {
        let v = parse_json(r#""\u0041""#).unwrap();
        assert_eq!(v.as_str(), "A");
    }

    #[test]
    fn parse_integer() {
        let v = parse_json("42").unwrap();
        assert_eq!(v.as_str(), "42");
    }

    #[test]
    fn parse_negative_float() {
        let v = parse_json("-3.14").unwrap();
        assert_eq!(v.as_str(), "-3.14");
    }

    #[test]
    fn parse_scientific() {
        let v = parse_json("1e3").unwrap();
        assert_eq!(v.as_str(), "1000");
    }

    #[test]
    fn parse_bool_true() {
        let v = parse_json("true").unwrap();
        assert_eq!(v.as_str(), "true");
    }

    #[test]
    fn parse_bool_false() {
        let v = parse_json("false").unwrap();
        assert_eq!(v.as_str(), "false");
    }

    #[test]
    fn parse_null() {
        let v = parse_json("null").unwrap();
        assert_eq!(v.as_str(), "");
    }

    #[test]
    fn parse_empty_object() {
        let v = parse_json("{}").unwrap();
        assert!(matches!(v, Value::Object(ref p) if p.is_empty()));
    }

    #[test]
    fn parse_empty_array() {
        let v = parse_json("[]").unwrap();
        assert!(matches!(v, Value::Array(ref a) if a.is_empty()));
    }

    #[test]
    fn parse_object() {
        let v = parse_json(r#"{"name": "simple", "version": 1}"#).unwrap();
        assert_eq!(v.get("name").unwrap().as_str(), "simple");
        assert_eq!(v.get("version").unwrap().as_str(), "1");
    }

    #[test]
    fn parse_array() {
        let v = parse_json(r#"[1, 2, 3]"#).unwrap();
        let items = v.as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].as_str(), "1");
    }

    #[test]
    fn parse_nested() {
        let v = parse_json(r#"{"a": {"b": {"c": "deep"}}}"#).unwrap();
        assert_eq!(v.get("a.b.c").unwrap().as_str(), "deep");
    }

    #[test]
    fn parse_array_index() {
        let v = parse_json(r#"{"items": ["x", "y", "z"]}"#).unwrap();
        assert_eq!(v.get("items.1").unwrap().as_str(), "y");
    }

    #[test]
    fn parse_whitespace() {
        let v = parse_json("  {  \"a\"  :  1  }  ").unwrap();
        assert_eq!(v.get("a").unwrap().as_str(), "1");
    }

    #[test]
    fn parse_error_trailing() {
        assert!(parse_json("42 garbage").is_err());
    }

    #[test]
    fn parse_error_unterminated_string() {
        assert!(parse_json(r#""hello"#).is_err());
    }

    #[test]
    fn parse_error_invalid_escape() {
        assert!(parse_json(r#""\x""#).is_err());
    }

    // -- to_json roundtrip ---------------------------------------------------

    #[test]
    fn to_json_roundtrip() {
        let input = r#"{"title":"hello","tags":["a","b"],"count":3,"active":true,"extra":null}"#;
        let v = parse_json(input).unwrap();
        let output = to_json(&v);
        let v2 = parse_json(&output).unwrap();
        assert_eq!(to_json(&v2), output);
    }

    // -- Content loading -----------------------------------------------------

    #[test]
    fn load_empty_dir() {
        let dir = std::env::temp_dir().join(format!("content_empty_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let content = load(&dir).unwrap();
        assert!(content.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_json_files() {
        let dir = std::env::temp_dir().join(format!("content_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("site.json"),
            r#"{"title": "Test", "desc": "A site"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("posts.json"), r#"[{"title": "Post 1"}]"#).unwrap();
        std::fs::write(dir.join("readme.txt"), "ignored").unwrap();

        let content = load(&dir).unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(
            content.get("site").unwrap().get("title").unwrap().as_str(),
            "Test"
        );
        assert_eq!(content.get("posts").unwrap().as_array().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_dir() {
        let content = load(Path::new("/tmp/does_not_exist_simple_test")).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn load_invalid_json() {
        let dir = std::env::temp_dir().join(format!("content_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("broken.json"), "{invalid}").unwrap();
        let result = load(&dir);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- resolve_placeholders ------------------------------------------------

    #[test]
    fn resolve_simple() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"title": "Hello"}"#).unwrap(),
        );
        let result = resolve_placeholders("<h1>{{@site.title}}</h1>", &content);
        assert_eq!(result, "<h1>Hello</h1>");
    }

    #[test]
    fn resolve_nested_path() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"contact": {"email": "hi@example.com"}}"#).unwrap(),
        );
        let result = resolve_placeholders("{{@site.contact.email}}", &content);
        assert_eq!(result, "hi@example.com");
    }

    #[test]
    fn resolve_missing_removed() {
        let content = HashMap::new();
        let result = resolve_placeholders("<p>{{@site.missing}}</p>", &content);
        assert_eq!(result, "<p></p>");
    }

    #[test]
    fn resolve_no_placeholders() {
        let content = HashMap::new();
        let result = resolve_placeholders("<p>hello</p>", &content);
        assert_eq!(result, "<p>hello</p>");
    }

    #[test]
    fn resolve_whole_file() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"title": "Hi"}"#).unwrap(),
        );
        // {{@site}} is escaped — quotes become &quot;
        let result = resolve_placeholders("{{@site}}", &content);
        assert_eq!(result, r#"{&quot;title&quot;:&quot;Hi&quot;}"#);
        // {{{@site}}} is raw — preserves JSON as-is
        let raw = resolve_placeholders("{{{@site}}}", &content);
        assert_eq!(raw, r#"{"title":"Hi"}"#);
    }

    #[test]
    fn resolve_number_value() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"port": 3000}"#).unwrap(),
        );
        let result = resolve_placeholders("{{@site.port}}", &content);
        assert_eq!(result, "3000");
    }

    #[test]
    fn resolve_preserves_component_props() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"title": "Hi"}"#).unwrap(),
        );
        let result =
            resolve_placeholders("<Header title=\"{{@site.title}}\" />{{label}}", &content);
        assert_eq!(result, "<Header title=\"Hi\" />{{label}}");
    }

    #[test]
    fn resolve_escapes_html() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"title": "<script>alert(1)</script>"}"#).unwrap(),
        );
        let result = resolve_placeholders("<h1>{{@site.title}}</h1>", &content);
        assert_eq!(result, "<h1>&lt;script&gt;alert(1)&lt;/script&gt;</h1>");
    }

    #[test]
    fn resolve_triple_brace_raw() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"html": "<b>bold</b>"}"#).unwrap(),
        );
        let result = resolve_placeholders("<div>{{{@site.html}}}</div>", &content);
        assert_eq!(result, "<div><b>bold</b></div>");
    }

    #[test]
    fn resolve_mixed_escaped_and_raw() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"bio": "<em>hi</em>"}"#).unwrap(),
        );
        let result = resolve_placeholders(
            "<p>{{@site.bio}}</p><div>{{{@site.bio}}}</div>",
            &content,
        );
        assert_eq!(
            result,
            "<p>&lt;em&gt;hi&lt;/em&gt;</p><div><em>hi</em></div>"
        );
    }

    // -- expand_each ---------------------------------------------------------

    #[test]
    fn each_simple() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json(r#"[{"name": "A"}, {"name": "B"}]"#).unwrap(),
        );
        let result = expand_each(
            "<ul><Each content=\"items\"><li>{{name}}</li></Each></ul>",
            &content,
        );
        assert_eq!(result, "<ul><li>A</li><li>B</li></ul>");
    }

    #[test]
    fn each_with_multiple_props() {
        let mut content = HashMap::new();
        content.insert(
            "team".to_string(),
            parse_json(r#"[{"name": "Alice", "role": "Dev"}]"#).unwrap(),
        );
        let result = expand_each(
            "<Each content=\"team\"><p>{{name}} — {{role}}</p></Each>",
            &content,
        );
        assert_eq!(result, "<p>Alice — Dev</p>");
    }

    #[test]
    fn each_nested_content_path() {
        let mut content = HashMap::new();
        content.insert(
            "site".to_string(),
            parse_json(r#"{"team": [{"name": "Bob"}]}"#).unwrap(),
        );
        let result = expand_each(
            "<Each content=\"site.team\"><span>{{name}}</span></Each>",
            &content,
        );
        assert_eq!(result, "<span>Bob</span>");
    }

    #[test]
    fn each_empty_array() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json("[]").unwrap(),
        );
        let result = expand_each(
            "<Each content=\"items\"><p>{{x}}</p></Each>",
            &content,
        );
        assert_eq!(result, "");
    }

    #[test]
    fn each_missing_content() {
        let content = HashMap::new();
        let result = expand_each(
            "<Each content=\"nope\"><p>{{x}}</p></Each>",
            &content,
        );
        assert_eq!(result, "");
    }

    #[test]
    fn each_preserves_surrounding() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json(r#"[{"v": "X"}]"#).unwrap(),
        );
        let result = expand_each(
            "<div>before</div><Each content=\"items\"><p>{{v}}</p></Each><div>after</div>",
            &content,
        );
        assert_eq!(result, "<div>before</div><p>X</p><div>after</div>");
    }

    #[test]
    fn each_with_component_tags() {
        let mut content = HashMap::new();
        content.insert(
            "posts".to_string(),
            parse_json(r#"[{"title": "Hello", "slug": "hello"}]"#).unwrap(),
        );
        let result = expand_each(
            r#"<Each content="posts"><Card title="{{title}}" href="/blog/{{slug}}" /></Each>"#,
            &content,
        );
        assert_eq!(
            result,
            r#"<Card title="Hello" href="/blog/hello" />"#
        );
    }

    #[test]
    fn each_escapes_html_values() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json(r#"[{"name": "<b>bold</b>"}]"#).unwrap(),
        );
        let result = expand_each(
            "<Each content=\"items\"><p>{{name}}</p></Each>",
            &content,
        );
        assert_eq!(result, "<p>&lt;b&gt;bold&lt;/b&gt;</p>");
    }

    #[test]
    fn each_triple_brace_raw() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json(r#"[{"html": "<b>bold</b>"}]"#).unwrap(),
        );
        let result = expand_each(
            "<Each content=\"items\"><div>{{{html}}}</div></Each>",
            &content,
        );
        assert_eq!(result, "<div><b>bold</b></div>");
    }

    #[test]
    fn each_leaves_unmatched_placeholders() {
        let mut content = HashMap::new();
        content.insert(
            "items".to_string(),
            parse_json(r#"[{"a": "1"}]"#).unwrap(),
        );
        let result = expand_each(
            "<Each content=\"items\"><p>{{a}} {{unknown}}</p></Each>",
            &content,
        );
        assert_eq!(result, "<p>1 {{unknown}}</p>");
    }
}
