use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct Component {
    pub template: String,
}

/// Escapes HTML special characters to prevent XSS.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Loads all `.html` files from the components directory.
pub fn load_components(dir: &Path) -> Result<HashMap<String, Component>, String> {
    let mut components = HashMap::new();

    if !dir.exists() {
        return Ok(components);
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("read dir entry: {e}"))?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid filename: {}", path.display()))?
            .to_string();

        let template =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;

        components.insert(name, Component { template });
    }

    Ok(components)
}

/// Renders a component by:
/// 1. Replacing `{{{prop}}}` with raw (unescaped) prop values
/// 2. Replacing `{{prop}}` with HTML-escaped prop values
/// 3. Inserting children before the closing tag of the root element
pub fn render(template: &str, props: &HashMap<String, String>, children: &str) -> String {
    let mut output = template.to_string();

    for (key, value) in props {
        // Raw (unescaped): {{{prop}}} — must be replaced BEFORE {{prop}}
        let mut raw_ph = String::with_capacity(key.len() + 6);
        raw_ph.push_str("{{{");
        raw_ph.push_str(key);
        raw_ph.push_str("}}}");
        output = output.replace(&raw_ph, value);

        // Escaped: {{prop}}
        let placeholder = format!("{{{{{key}}}}}");
        output = output.replace(&placeholder, &escape_html(value));
    }

    // Clean up unreplaced {{placeholders}} and {{{placeholders}}}
    output = clean_placeholders(&output);

    // Insert children before the root element's closing tag
    if !children.is_empty() {
        output = insert_children(&output, children);
    }

    output
}

/// Same as render but specifically for layout files — inserts before </body>.
pub fn render_layout(template: &str, props: &HashMap<String, String>, children: &str) -> String {
    let mut output = template.to_string();

    for (key, value) in props {
        // Raw (unescaped): {{{prop}}} — must be replaced BEFORE {{prop}}
        let mut raw_ph = String::with_capacity(key.len() + 6);
        raw_ph.push_str("{{{");
        raw_ph.push_str(key);
        raw_ph.push_str("}}}");
        output = output.replace(&raw_ph, value);

        // Escaped: {{prop}}
        let placeholder = format!("{{{{{key}}}}}");
        output = output.replace(&placeholder, &escape_html(value));
    }

    output = clean_placeholders(&output);

    if !children.is_empty() {
        if let Some(pos) = output.rfind("</body>") {
            output.insert_str(pos, children);
        }
    }

    output
}

/// Finds the root element's tag name and inserts children before its closing tag.
fn insert_children(template: &str, children: &str) -> String {
    let trimmed = template.trim();

    // Find the first opening tag to get the root element name
    if let Some(root_tag) = find_root_tag(trimmed) {
        let closing = format!("</{root_tag}>");
        // Find the LAST occurrence of this closing tag (the root's close)
        if let Some(pos) = template.rfind(&closing) {
            let mut result = String::with_capacity(template.len() + children.len());
            result.push_str(&template[..pos]);
            result.push_str(children);
            result.push_str(&template[pos..]);
            return result;
        }
    }

    // Fallback: just append
    format!("{template}{children}")
}

/// Extracts the tag name from the first opening tag in the HTML.
fn find_root_tag(html: &str) -> Option<String> {
    let html = html.trim();
    if !html.starts_with('<') {
        return None;
    }

    // Skip special tags like <!DOCTYPE, <!--
    if html.starts_with("<!") || html.starts_with("<?") {
        return None;
    }

    let after = &html[1..];
    let end = after
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(after.len());

    let tag = &after[..end];
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

fn clean_placeholders(input: &str) -> String {
    let mut cleaned = String::with_capacity(input.len());
    let mut remaining = input;

    loop {
        let Some(pos) = remaining.find("{{") else {
            cleaned.push_str(remaining);
            break;
        };

        cleaned.push_str(&remaining[..pos]);
        let after = &remaining[pos..];

        if after.starts_with("{{{") {
            // Triple brace: find }}}
            if let Some(end) = after[3..].find("}}}") {
                remaining = &after[3 + end + 3..];
            } else {
                cleaned.push_str("{{{");
                remaining = &after[3..];
            }
        } else {
            // Double brace: find }}
            if let Some(end) = after[2..].find("}}") {
                remaining = &after[2 + end + 2..];
            } else {
                cleaned.push_str("{{");
                remaining = &after[2..];
            }
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_go_inside_root() {
        let props = HashMap::new();
        let result = render("<div class=\"card\"></div>", &props, "<p>hello</p>");
        assert_eq!(result, "<div class=\"card\"><p>hello</p></div>");
    }

    #[test]
    fn children_in_nested_component() {
        let props = HashMap::new();
        let tpl = "<header>\n    <h1>Title</h1>\n</header>";
        let result = render(tpl, &props, "<nav>links</nav>");
        assert_eq!(result, "<header>\n    <h1>Title</h1>\n<nav>links</nav></header>");
    }

    #[test]
    fn props_replaced() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "Hello".to_string());
        let result = render("<h1>{{title}}</h1>", &props, "");
        assert_eq!(result, "<h1>Hello</h1>");
    }

    #[test]
    fn props_html_escaped() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "<script>alert(1)</script>".to_string());
        let result = render("<h1>{{title}}</h1>", &props, "");
        assert_eq!(result, "<h1>&lt;script&gt;alert(1)&lt;/script&gt;</h1>");
    }

    #[test]
    fn props_triple_brace_raw() {
        let mut props = HashMap::new();
        props.insert("html".to_string(), "<b>bold</b>".to_string());
        let result = render("<div>{{{html}}}</div>", &props, "");
        assert_eq!(result, "<div><b>bold</b></div>");
    }

    #[test]
    fn props_mixed_escaped_and_raw() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "<em>hi</em>".to_string());
        let result = render("<h1>{{title}}</h1><div>{{{title}}}</div>", &props, "");
        assert_eq!(
            result,
            "<h1>&lt;em&gt;hi&lt;/em&gt;</h1><div><em>hi</em></div>"
        );
    }

    #[test]
    fn escape_html_special_chars() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("he said \"hi\""), "he said &quot;hi&quot;");
        assert_eq!(escape_html("it's"), "it&#x27;s");
    }

    #[test]
    fn clean_triple_brace_placeholders() {
        let mut props = HashMap::new();
        props.insert("a".to_string(), "yes".to_string());
        // {{{missing}}} should be cleaned
        let result = render("<p>{{a}} {{{missing}}}</p>", &props, "");
        assert_eq!(result, "<p>yes </p>");
    }

    #[test]
    fn layout_inserts_in_body() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "Test".to_string());
        let layout = "<html><head><title>{{title}}</title></head><body></body></html>";
        let result = render_layout(layout, &props, "<p>content</p>");
        assert_eq!(result, "<html><head><title>Test</title></head><body><p>content</p></body></html>");
    }

    #[test]
    fn layout_escapes_props() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "A & B".to_string());
        let layout = "<html><head><title>{{title}}</title></head><body></body></html>";
        let result = render_layout(layout, &props, "");
        assert!(result.contains("<title>A &amp; B</title>"));
    }
}
