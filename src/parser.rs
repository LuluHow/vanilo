use std::collections::HashMap;

use crate::component::Component;

/// Recursively resolves all component tags in the HTML content.
/// Component tags are PascalCase: `<Header />` or `<Card title="x">children</Card>`
pub fn resolve(html: &str, components: &HashMap<String, Component>) -> String {
    let mut output = String::with_capacity(html.len());
    let mut remaining = html;

    while !remaining.is_empty() {
        // Find next '<' that could be a component tag (PascalCase)
        match find_component_tag(remaining, components) {
            Some((before, tag_name, props, slot_content, after)) => {
                output.push_str(before);

                if let Some(comp) = components.get(&tag_name) {
                    // Recursively resolve slot content first
                    let resolved_slot = resolve(&slot_content, components);
                    let rendered = crate::component::render(&comp.template, &props, &resolved_slot);
                    // Recursively resolve the rendered template (components inside components)
                    let resolved = resolve(&rendered, components);
                    output.push_str(&resolved);
                }

                remaining = after;
            }
            None => {
                output.push_str(remaining);
                break;
            }
        }
    }

    output
}

/// Finds the next component tag in the input.
/// Returns (text_before, tag_name, props, slot_content, text_after) or None.
fn find_component_tag<'a>(
    input: &'a str,
    components: &HashMap<String, Component>,
) -> Option<(&'a str, String, HashMap<String, String>, String, &'a str)> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' && i + 1 < len && bytes[i + 1].is_ascii_uppercase() {
            // Potential component tag
            let tag_start = i;

            // Extract tag name
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < len
                && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
            {
                name_end += 1;
            }

            let tag_name = &input[name_start..name_end];

            // Check if this is a known component
            if !components.contains_key(tag_name) {
                i += 1;
                continue;
            }

            // Parse attributes
            let (props, after_attrs) = parse_attributes(&input[name_end..]);

            // Check for self-closing or opening tag
            let rest = after_attrs.trim_start();
            if let Some(rest) = rest.strip_prefix("/>") {
                // Self-closing tag
                let before = &input[..tag_start];
                return Some((before, tag_name.to_string(), props, String::new(), rest));
            } else if let Some(rest) = rest.strip_prefix('>') {
                // Opening tag — find matching closing tag
                let closing = format!("</{tag_name}>");
                if let Some(close_pos) = find_matching_close(rest, tag_name) {
                    let slot = &rest[..close_pos];
                    let after = &rest[close_pos + closing.len()..];
                    let before = &input[..tag_start];
                    return Some((
                        before,
                        tag_name.to_string(),
                        props,
                        slot.to_string(),
                        after,
                    ));
                }
            }
        }
        i += 1;
    }

    None
}

/// Parses HTML-style attributes from a string.
/// Returns the parsed props and the remaining string after attributes.
fn parse_attributes(input: &str) -> (HashMap<String, String>, &str) {
    let mut props = HashMap::new();
    let mut remaining = input;

    loop {
        remaining = remaining.trim_start();

        // Stop at tag end markers
        if remaining.starts_with("/>") || remaining.starts_with('>') || remaining.is_empty() {
            break;
        }

        // Parse attribute name
        let name_end = remaining
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(remaining.len());

        if name_end == 0 {
            break;
        }

        let attr_name = &remaining[..name_end];
        remaining = &remaining[name_end..];

        // Check for = and value
        remaining = remaining.trim_start();
        if let Some(rest) = remaining.strip_prefix('=') {
            remaining = rest.trim_start();

            // Parse quoted value
            if remaining.starts_with('"') {
                remaining = &remaining[1..];
                let end = remaining.find('"').unwrap_or(remaining.len());
                let value = &remaining[..end];
                props.insert(attr_name.to_string(), value.to_string());
                remaining = &remaining[(end + 1).min(remaining.len())..];
            } else if remaining.starts_with('\'') {
                remaining = &remaining[1..];
                let end = remaining.find('\'').unwrap_or(remaining.len());
                let value = &remaining[..end];
                props.insert(attr_name.to_string(), value.to_string());
                remaining = &remaining[(end + 1).min(remaining.len())..];
            } else {
                // Unquoted value — take until whitespace or >
                let end = remaining
                    .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
                    .unwrap_or(remaining.len());
                let value = &remaining[..end];
                props.insert(attr_name.to_string(), value.to_string());
                remaining = &remaining[end..];
            }
        } else {
            // Boolean attribute
            props.insert(attr_name.to_string(), String::new());
        }
    }

    (props, remaining)
}

/// Finds the position of the matching closing tag, handling nesting.
fn find_matching_close(input: &str, tag_name: &str) -> Option<usize> {
    let open_tag = format!("<{tag_name}");
    let close_tag = format!("</{tag_name}>");
    let mut depth = 1;
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if input[i..].starts_with(&close_tag) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += close_tag.len();
        } else if input[i..].starts_with(&open_tag) {
            // Check it's actually a tag (followed by space, >, or /)
            let after = i + open_tag.len();
            if after < bytes.len()
                && (bytes[after] == b' '
                    || bytes[after] == b'>'
                    || bytes[after] == b'/'
                    || bytes[after] == b'\n')
            {
                depth += 1;
            }
            i += open_tag.len();
        } else {
            i += 1;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_components() -> HashMap<String, Component> {
        let mut comps = HashMap::new();
        comps.insert(
            "Header".to_string(),
            Component {
                template: "<header><h1>{{title}}</h1></header>".to_string(),
            },
        );
        comps.insert(
            "Card".to_string(),
            Component {
                template: "<div class=\"card\"></div>".to_string(),
            },
        );
        comps
    }

    #[test]
    fn self_closing_component() {
        let comps = make_components();
        let result = resolve("<Header title=\"Hello\" />", &comps);
        assert_eq!(result, "<header><h1>Hello</h1></header>");
    }

    #[test]
    fn block_component_with_children() {
        let comps = make_components();
        let result = resolve("<Card><p>Content</p></Card>", &comps);
        assert_eq!(result, "<div class=\"card\"><p>Content</p></div>");
    }

    #[test]
    fn plain_html_unchanged() {
        let comps = make_components();
        let input = "<div><p>Hello world</p></div>";
        assert_eq!(resolve(input, &comps), input);
    }

    #[test]
    fn mixed_html_and_components() {
        let comps = make_components();
        let input = "<div><Header title=\"Hi\" /><p>text</p></div>";
        let result = resolve(input, &comps);
        assert_eq!(result, "<div><header><h1>Hi</h1></header><p>text</p></div>");
    }
}
