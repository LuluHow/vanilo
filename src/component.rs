use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct Component {
    pub template: String,
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
/// 1. Replacing `{{prop}}` with prop values
/// 2. Inserting children before the closing tag of the root element
pub fn render(template: &str, props: &HashMap<String, String>, children: &str) -> String {
    let mut output = template.to_string();

    // Replace {{prop_name}} for each prop
    for (key, value) in props {
        let placeholder = format!("{{{{{key}}}}}");
        output = output.replace(&placeholder, value);
    }

    // Clean up unreplaced {{placeholders}}
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
        let placeholder = format!("{{{{{key}}}}}");
        output = output.replace(&placeholder, value);
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
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'{') {
            let mut placeholder = String::from("{{");
            chars.next();
            let mut found_close = false;
            while let Some(pc) = chars.next() {
                placeholder.push(pc);
                if pc == '}' && chars.peek() == Some(&'}') {
                    chars.next();
                    placeholder.push('}');
                    found_close = true;
                    break;
                }
            }
            if !found_close {
                cleaned.push_str(&placeholder);
            }
        } else {
            cleaned.push(c);
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
    fn layout_inserts_in_body() {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "Test".to_string());
        let layout = "<html><head><title>{{title}}</title></head><body></body></html>";
        let result = render_layout(layout, &props, "<p>content</p>");
        assert_eq!(result, "<html><head><title>Test</title></head><body><p>content</p></body></html>");
    }
}
