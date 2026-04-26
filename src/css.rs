use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns a pruned copy of `css` containing only rules whose selectors
/// reference tags/classes/IDs present in `html`.
/// Always keeps @font-face, @keyframes, :root, and * rules.
/// Selectors listed in `/* vanilo:keep .selector */` comments are preserved
/// regardless of HTML usage (useful for classes added dynamically by JS).
pub fn tree_shake(css: &str, html: &str) -> String {
    let safelist = extract_safelist(css);
    let cleaned = strip_comments(css);
    let blocks = parse_blocks(&cleaned);
    let mut tokens = extract_html_tokens(html);

    // Merge safelist into tokens so safelisted selectors always match
    for entry in &safelist {
        if entry.starts_with('.') {
            tokens.classes.insert(entry[1..].to_string());
        } else if entry.starts_with('#') {
            tokens.ids.insert(entry[1..].to_string());
        } else {
            tokens.tags.insert(entry.to_lowercase());
        }
    }

    let mut out = String::new();
    for block in &blocks {
        emit_if_used(block, &tokens, &mut out);
    }

    compact(&out)
}

/// Extracts selectors from `/* vanilo:keep .selector */` comments.
fn extract_safelist(css: &str) -> Vec<String> {
    let mut safelist = Vec::new();
    let mut remaining = css;
    while let Some(start) = remaining.find("/* vanilo:keep ") {
        let after = &remaining[start + 15..];
        if let Some(end) = after.find("*/") {
            let entries = after[..end].trim();
            for entry in entries.split_whitespace() {
                if !entry.is_empty() {
                    safelist.push(entry.to_string());
                }
            }
            remaining = &after[end + 2..];
        } else {
            break;
        }
    }
    safelist
}

// ---------------------------------------------------------------------------
// CSS data model
// ---------------------------------------------------------------------------

enum Block {
    /// selector { body }
    Rule { selector: String, body: String },
    /// @font-face { body }, @keyframes name { body }, etc.
    AtRule { header: String, body: String },
    /// @media query { children } or @supports { children }
    Media { query: String, children: Vec<Block> },
}

// ---------------------------------------------------------------------------
// Comment stripping
// ---------------------------------------------------------------------------

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '/' {
            chars.next();
            if chars.peek() == Some(&'*') {
                chars.next();
                loop {
                    match chars.next() {
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            } else {
                out.push('/');
            }
        } else {
            out.push(c);
            chars.next();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CSS parsing
// ---------------------------------------------------------------------------

fn parse_blocks(css: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut remaining = css;

    loop {
        remaining = remaining.trim();
        if remaining.is_empty() {
            break;
        }

        let open = match remaining.find('{') {
            Some(p) => p,
            None => break,
        };

        let header = remaining[..open].trim();
        let after_open = &remaining[open + 1..];

        let close = match find_matching_brace(after_open) {
            Some(p) => p,
            None => break,
        };

        let body = &after_open[..close];
        remaining = &after_open[close + 1..];

        if header.starts_with("@media") || header.starts_with("@supports") {
            blocks.push(Block::Media {
                query: header.to_string(),
                children: parse_blocks(body),
            });
        } else if header.starts_with('@') {
            blocks.push(Block::AtRule {
                header: header.to_string(),
                body: body.to_string(),
            });
        } else if !header.is_empty() {
            blocks.push(Block::Rule {
                selector: header.to_string(),
                body: body.to_string(),
            });
        }
    }

    blocks
}

/// Finds the position of the matching `}` in a string that starts just after
/// an opening `{`. Tracks nesting and skips quoted strings.
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth: u32 = 1;
    let mut in_string = false;
    let mut string_char = '"';
    let mut escape_next = false;

    for (i, c) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape_next = true;
            } else if c == string_char {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// HTML token extraction
// ---------------------------------------------------------------------------

struct HtmlTokens {
    tags: HashSet<String>,
    classes: HashSet<String>,
    ids: HashSet<String>,
}

fn extract_html_tokens(html: &str) -> HtmlTokens {
    let mut tags = HashSet::new();
    let mut classes = HashSet::new();
    let mut ids = HashSet::new();
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' && i + 1 < len && bytes[i + 1].is_ascii_alphabetic() {
            i += 1;
            let tag_start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
                i += 1;
            }
            tags.insert(html[tag_start..i].to_ascii_lowercase());

            // Find end of opening tag
            let tag_end = html[i..].find('>').map_or(len, |p| i + p);
            let attrs = &html[i..tag_end];

            for val in find_attr_values(attrs, "class") {
                for c in val.split_whitespace() {
                    if !c.is_empty() {
                        classes.insert(c.to_string());
                    }
                }
            }
            for val in find_attr_values(attrs, "id") {
                let id = val.trim();
                if !id.is_empty() {
                    ids.insert(id.to_string());
                }
            }

            i = tag_end;
        } else {
            i += 1;
        }
    }

    HtmlTokens { tags, classes, ids }
}

/// Finds all values of a given attribute in an attribute string.
/// Handles word-boundary checking to avoid false matches (e.g. "myclass=" for "class=").
fn find_attr_values<'a>(attrs: &'a str, name: &str) -> Vec<&'a str> {
    let mut values = Vec::new();
    let needle = format!("{name}=");
    let mut remaining = attrs;

    loop {
        let pos = match remaining.find(&needle) {
            Some(p) => p,
            None => break,
        };

        // Word boundary: must be preceded by whitespace or be at start
        if pos > 0 && !remaining.as_bytes()[pos - 1].is_ascii_whitespace() {
            remaining = &remaining[pos + needle.len()..];
            continue;
        }

        remaining = &remaining[pos + needle.len()..];
        let q = match remaining.as_bytes().first() {
            Some(&b'"') => '"',
            Some(&b'\'') => '\'',
            _ => continue,
        };
        remaining = &remaining[1..];
        if let Some(end) = remaining.find(q) {
            values.push(&remaining[..end]);
            remaining = &remaining[end + 1..];
        } else {
            break;
        }
    }

    values
}

// ---------------------------------------------------------------------------
// Selector matching
// ---------------------------------------------------------------------------

#[cfg(test)]
fn selector_matches(selector: &str, tokens: &HtmlTokens) -> bool {
    selector
        .split(',')
        .any(|s| single_selector_matches(s.trim(), tokens))
}

fn single_selector_matches(selector: &str, tokens: &HtmlTokens) -> bool {
    let s = selector.trim();
    if s.is_empty() {
        return false;
    }
    if s == "*" || s == ":root" {
        return true;
    }

    // Split into simple selectors by combinators (space, >, +, ~)
    // ALL parts must match for the whole selector to match.
    for part in split_combinators(s) {
        if !simple_part_matches(part, tokens) {
            return false;
        }
    }
    true
}

/// Splits a compound selector into simple selector parts, stripping combinators.
fn split_combinators(selector: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut remaining = selector.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.starts_with('>')
            || remaining.starts_with('+')
            || remaining.starts_with('~')
        {
            remaining = remaining[1..].trim_start();
            continue;
        }

        let end = find_simple_selector_end(remaining);
        if end > 0 {
            parts.push(&remaining[..end]);
            remaining = &remaining[end..];
        } else {
            break;
        }
    }
    parts
}

/// Finds the end of a simple selector (stops at combinators: space, >, +, ~).
/// Respects parentheses and brackets.
fn find_simple_selector_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut paren: u32 = 0;
    let mut bracket: u32 = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'+' | b'~'
                if paren == 0 && bracket == 0 =>
            {
                return i;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Checks if a simple selector part (e.g. `div.card:hover`) matches the HTML tokens.
fn simple_part_matches(selector: &str, tokens: &HtmlTokens) -> bool {
    let mut remaining = selector;
    let mut need_tag: Option<String> = None;
    let mut need_classes: Vec<String> = Vec::new();
    let mut need_ids: Vec<String> = Vec::new();

    while !remaining.is_empty() {
        if remaining.starts_with("::") {
            break; // pseudo-element — ignore rest
        } else if remaining.starts_with(':') {
            remaining = &remaining[1..];
            remaining = skip_pseudo_name(remaining);
        } else if remaining.starts_with('.') {
            remaining = &remaining[1..];
            let end = ident_end(remaining);
            if end > 0 {
                need_classes.push(remaining[..end].to_string());
                remaining = &remaining[end..];
            } else {
                break;
            }
        } else if remaining.starts_with('#') {
            remaining = &remaining[1..];
            let end = ident_end(remaining);
            if end > 0 {
                need_ids.push(remaining[..end].to_string());
                remaining = &remaining[end..];
            } else {
                break;
            }
        } else if remaining.starts_with('[') {
            if let Some(end) = remaining.find(']') {
                remaining = &remaining[end + 1..];
            } else {
                break;
            }
        } else if remaining.starts_with('*') {
            remaining = &remaining[1..]; // universal — no constraint
        } else if remaining
            .as_bytes()
            .first()
            .map_or(false, |b| b.is_ascii_alphabetic())
        {
            let end = ident_end(remaining);
            if end > 0 {
                need_tag = Some(remaining[..end].to_ascii_lowercase());
                remaining = &remaining[end..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if let Some(ref tag) = need_tag {
        if !tokens.tags.contains(tag) {
            return false;
        }
    }
    for class in &need_classes {
        if !tokens.classes.contains(class) {
            return false;
        }
    }
    for id in &need_ids {
        if !tokens.ids.contains(id) {
            return false;
        }
    }

    true
}

fn ident_end(s: &str) -> usize {
    s.find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(s.len())
}

/// Skips a pseudo-class name (and any function arguments in parentheses).
fn skip_pseudo_name(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut paren: u32 = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'(' => paren += 1,
            b')' => {
                paren = paren.saturating_sub(1);
                if paren == 0 {
                    i += 1;
                    break;
                }
            }
            b'.' | b'#' | b':' | b'[' if paren == 0 => break,
            b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'+' | b'~' if paren == 0 => break,
            _ => {}
        }
        i += 1;
    }

    &s[i..]
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

fn emit_if_used(block: &Block, tokens: &HtmlTokens, out: &mut String) {
    match block {
        Block::AtRule { header, body } => {
            // Always keep @font-face, @keyframes, @charset, etc.
            out.push_str(header);
            out.push('{');
            out.push_str(body);
            out.push('}');
        }
        Block::Rule { selector, body } => {
            // For grouped selectors, keep only matching groups
            let matching: Vec<&str> = selector
                .split(',')
                .map(|s| s.trim())
                .filter(|s| single_selector_matches(s, tokens))
                .collect();

            if !matching.is_empty() {
                out.push_str(&matching.join(","));
                out.push('{');
                out.push_str(body);
                out.push('}');
            }
        }
        Block::Media { query, children } => {
            let mut inner = String::new();
            for child in children {
                emit_if_used(child, tokens, &mut inner);
            }
            if !inner.is_empty() {
                out.push_str(query);
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CSS compaction (collapse whitespace)
// ---------------------------------------------------------------------------

fn compact(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut last_was_ws = false;

    for c in css.chars() {
        if c.is_ascii_whitespace() {
            if !last_was_ws {
                out.push(' ');
                last_was_ws = true;
            }
        } else {
            out.push(c);
            last_was_ws = false;
        }
    }

    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- strip_comments -----------------------------------------------------

    #[test]
    fn strip_basic_comment() {
        assert_eq!(strip_comments("a /* x */ b"), "a  b");
    }

    #[test]
    fn strip_no_comment() {
        assert_eq!(strip_comments("body { color: red; }"), "body { color: red; }");
    }

    #[test]
    fn strip_multiple_comments() {
        assert_eq!(strip_comments("/* a */b/* c */d"), "bd");
    }

    #[test]
    fn strip_unterminated_comment() {
        assert_eq!(strip_comments("a /* never closed"), "a ");
    }

    // -- parse_blocks -------------------------------------------------------

    #[test]
    fn parse_single_rule() {
        let blocks = parse_blocks("body { color: red; }");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::Rule { selector, .. } if selector == "body"));
    }

    #[test]
    fn parse_at_rule() {
        let blocks = parse_blocks("@font-face { font-family: 'X'; }");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::AtRule { header, .. } if header == "@font-face"));
    }

    #[test]
    fn parse_media_query() {
        let css = "@media (max-width: 600px) { .hero { font-size: 1rem; } main { padding: 1rem; } }";
        let blocks = parse_blocks(css);
        assert_eq!(blocks.len(), 1);
        if let Block::Media { children, .. } = &blocks[0] {
            assert_eq!(children.len(), 2);
        } else {
            panic!("expected Media block");
        }
    }

    #[test]
    fn parse_multiple_rules() {
        let blocks = parse_blocks("a { color: red; } b { color: blue; }");
        assert_eq!(blocks.len(), 2);
    }

    // -- find_matching_brace ------------------------------------------------

    #[test]
    fn brace_simple() {
        assert_eq!(find_matching_brace("color: red; }"), Some(12));
    }

    #[test]
    fn brace_nested() {
        assert_eq!(find_matching_brace("a { b: c; } }"), Some(12));
    }

    #[test]
    fn brace_in_string() {
        assert_eq!(find_matching_brace(r#"content: "}"; }"#), Some(14));
    }

    #[test]
    fn brace_unmatched() {
        assert_eq!(find_matching_brace("color: red;"), None);
    }

    // -- extract_html_tokens ------------------------------------------------

    #[test]
    fn tokens_tags() {
        let t = extract_html_tokens("<div><p>hello</p></div>");
        assert!(t.tags.contains("div"));
        assert!(t.tags.contains("p"));
    }

    #[test]
    fn tokens_classes() {
        let t = extract_html_tokens(r#"<div class="card hero"><p class="sub">x</p></div>"#);
        assert!(t.classes.contains("card"));
        assert!(t.classes.contains("hero"));
        assert!(t.classes.contains("sub"));
    }

    #[test]
    fn tokens_ids() {
        let t = extract_html_tokens(r#"<div id="main"><span id="count">0</span></div>"#);
        assert!(t.ids.contains("main"));
        assert!(t.ids.contains("count"));
    }

    #[test]
    fn tokens_no_false_match_class() {
        // "myclass=" should NOT match as "class="
        let t = extract_html_tokens(r#"<div myclass="nope" class="yes">x</div>"#);
        assert!(t.classes.contains("yes"));
        assert!(!t.classes.contains("nope"));
    }

    #[test]
    fn tokens_skips_closing_tags() {
        let t = extract_html_tokens("</div>");
        assert!(!t.tags.contains("div"));
    }

    // -- selector_matches ---------------------------------------------------

    #[test]
    fn matches_tag() {
        let t = tokens(&["div", "p"], &[], &[]);
        assert!(selector_matches("div", &t));
        assert!(!selector_matches("span", &t));
    }

    #[test]
    fn matches_class() {
        let t = tokens(&[], &["card"], &[]);
        assert!(selector_matches(".card", &t));
        assert!(!selector_matches(".hero", &t));
    }

    #[test]
    fn matches_id() {
        let t = tokens(&[], &[], &["main"]);
        assert!(selector_matches("#main", &t));
        assert!(!selector_matches("#other", &t));
    }

    #[test]
    fn matches_universal() {
        let t = tokens(&[], &[], &[]);
        assert!(selector_matches("*", &t));
        assert!(selector_matches("*::before", &t));
    }

    #[test]
    fn matches_root() {
        let t = tokens(&[], &[], &[]);
        assert!(selector_matches(":root", &t));
    }

    #[test]
    fn matches_descendant() {
        let t = tokens(&["nav", "a"], &[], &[]);
        assert!(selector_matches("nav a", &t));

        let t2 = tokens(&["a"], &[], &[]);
        assert!(!selector_matches("nav a", &t2));
    }

    #[test]
    fn matches_child_combinator() {
        let t = tokens(&["p"], &["card"], &[]);
        assert!(selector_matches(".card > p", &t));
    }

    #[test]
    fn matches_pseudo_class() {
        let t = tokens(&["a"], &[], &[]);
        assert!(selector_matches("a:hover", &t));
    }

    #[test]
    fn matches_class_pseudo() {
        let t = tokens(&[], &["card"], &[]);
        assert!(selector_matches(".card:hover", &t));
    }

    #[test]
    fn matches_descendant_pseudo() {
        let t = tokens(&["p"], &["content"], &[]);
        assert!(selector_matches(".content p:first-of-type", &t));
    }

    #[test]
    fn matches_grouped_selectors() {
        let t = tokens(&["a"], &[], &[]);
        // "a" matches, "span" does not — grouped selector matches if ANY group matches
        assert!(selector_matches("a, span", &t));
    }

    #[test]
    fn no_match_grouped_all_miss() {
        let t = tokens(&["div"], &[], &[]);
        assert!(!selector_matches("a, span", &t));
    }

    #[test]
    fn matches_tag_class_combo() {
        let t = tokens(&["h1"], &["hero"], &[]);
        assert!(selector_matches(".hero h1", &t));

        let t2 = tokens(&["h1"], &[], &[]);
        assert!(!selector_matches(".hero h1", &t2));
    }

    // -- tree_shake end-to-end -----------------------------------------------

    #[test]
    fn shake_keeps_matching_rules() {
        let css = "body { color: red; } .hero { font-size: 2rem; }";
        let html = "<body><p>hello</p></body>";
        let result = tree_shake(css, html);
        assert!(result.contains("body"));
        assert!(result.contains("color: red"));
        assert!(!result.contains(".hero"));
    }

    #[test]
    fn shake_keeps_font_face() {
        let css = "@font-face { font-family: 'X'; } .unused { color: red; }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains("@font-face"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn shake_keeps_root() {
        let css = ":root { --fg: #fff; } .unused { color: red; }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains(":root"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn shake_keeps_universal() {
        let css = "*, *::before, *::after { box-sizing: border-box; }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains("box-sizing"));
    }

    #[test]
    fn shake_media_partial() {
        let css = "@media (max-width: 600px) { .hero h1 { font-size: 1.5rem; } main { padding: 1rem; } }";
        let html = "<main>hello</main>";
        let result = tree_shake(css, html);
        assert!(result.contains("@media"));
        assert!(result.contains("main"));
        assert!(!result.contains(".hero"));
    }

    #[test]
    fn shake_media_all_removed() {
        let css = "@media (max-width: 600px) { .hero { font-size: 1rem; } .grid { gap: 0; } }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(!result.contains("@media"));
    }

    #[test]
    fn shake_grouped_selector_partial() {
        let css = "a, span { color: red; }";
        let html = "<div><a href=\"/\">link</a></div>";
        let result = tree_shake(css, html);
        assert!(result.contains("a{") || result.contains("a {"));
        assert!(!result.contains("span"));
    }

    #[test]
    fn shake_safelist_keeps_class() {
        let css = "/* vanilo:keep .open */ .open { display: block; } .unused { color: red; }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains(".open"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn shake_safelist_multiple() {
        let css = "/* vanilo:keep .modal .active */ .modal { display: flex; } .active { opacity: 1; } .gone { color: red; }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains(".modal"));
        assert!(result.contains(".active"));
        assert!(!result.contains(".gone"));
    }

    #[test]
    fn shake_safelist_tag_and_id() {
        let css = "/* vanilo:keep dialog #overlay */ dialog { position: fixed; } #overlay { background: rgba(0,0,0,.5); }";
        let html = "<div>hello</div>";
        let result = tree_shake(css, html);
        assert!(result.contains("dialog"));
        assert!(result.contains("#overlay"));
    }

    #[test]
    fn shake_empty_css() {
        assert_eq!(tree_shake("", "<div>hello</div>"), "");
    }

    #[test]
    fn shake_comments_stripped() {
        let css = "/* comment */ body { color: red; }";
        let html = "<body>hello</body>";
        let result = tree_shake(css, html);
        assert!(result.contains("body"));
        assert!(!result.contains("comment"));
    }

    #[test]
    fn compact_collapses_whitespace() {
        assert_eq!(compact("a  {\n  color:  red;\n}"), "a { color: red; }");
    }

    // -- helpers ------------------------------------------------------------

    fn tokens(tag_list: &[&str], class_list: &[&str], id_list: &[&str]) -> HtmlTokens {
        HtmlTokens {
            tags: tag_list.iter().map(|s| s.to_string()).collect(),
            classes: class_list.iter().map(|s| s.to_string()).collect(),
            ids: id_list.iter().map(|s| s.to_string()).collect(),
        }
    }
}
