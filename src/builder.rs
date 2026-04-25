use std::fs;
use std::path::Path;

use crate::component;
use crate::parser;

const PAGES_DIR: &str = "pages";
const COMPONENTS_DIR: &str = "components";
const STATIC_DIR: &str = "static";
const DIST_DIR: &str = "dist";
const LAYOUT_FILE: &str = "layout.html";

/// Scaffolds a new project structure.
pub fn init() -> Result<(), String> {
    create_dir(PAGES_DIR)?;
    create_dir(COMPONENTS_DIR)?;
    create_dir(STATIC_DIR)?;

    // Default layout
    if !Path::new(LAYOUT_FILE).exists() {
        let layout = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{{title}}</title>
    <link rel="stylesheet" href="/style.css">
    <script src="/main.js" defer></script>
</head>
<body>
</body>
</html>"#;
        write_file(LAYOUT_FILE, layout)?;
    }

    // Example component
    let header_path = format!("{COMPONENTS_DIR}/Header.html");
    if !Path::new(&header_path).exists() {
        let header = r#"<header>
    <h1>{{title}}</h1>
</header>"#;
        write_file(&header_path, header)?;
    }

    // Example page
    let index_path = format!("{PAGES_DIR}/index.html");
    if !Path::new(&index_path).exists() {
        let index = r#"<meta name="title" content="Home">

<Header title="My Site">
    <a href="/">Home</a>
    <a href="/about">About</a>
</Header>

<main>
    <h2>Welcome</h2>
    <p>Edit pages/ and components/, then run <code>simple build</code>.</p>
</main>"#;
        write_file(&index_path, index)?;
    }

    // Empty CSS/JS
    if !Path::new(&format!("{STATIC_DIR}/style.css")).exists() {
        write_file(&format!("{STATIC_DIR}/style.css"), "/* your styles */\n")?;
    }
    if !Path::new(&format!("{STATIC_DIR}/main.js")).exists() {
        write_file(&format!("{STATIC_DIR}/main.js"), "// your scripts\n")?;
    }

    println!("project initialized:");
    println!("  {LAYOUT_FILE}");
    println!("  {PAGES_DIR}/index.html");
    println!("  {COMPONENTS_DIR}/Header.html");
    println!("  {STATIC_DIR}/style.css");
    println!("  {STATIC_DIR}/main.js");
    println!();
    println!("run `simple build` to generate dist/");
    Ok(())
}

/// Builds the site: resolves components, wraps in layout, outputs to dist/.
pub fn build() -> Result<(), String> {
    // Clean dist
    if Path::new(DIST_DIR).exists() {
        fs::remove_dir_all(DIST_DIR).map_err(|e| format!("clean dist: {e}"))?;
    }
    fs::create_dir_all(DIST_DIR).map_err(|e| format!("create dist: {e}"))?;

    // Load components
    let components = component::load_components(Path::new(COMPONENTS_DIR))?;
    println!("loaded {} component(s)", components.len());

    // Load layout (optional)
    let layout = if Path::new(LAYOUT_FILE).exists() {
        Some(fs::read_to_string(LAYOUT_FILE).map_err(|e| format!("read layout: {e}"))?)
    } else {
        None
    };

    // Process pages
    let pages_path = Path::new(PAGES_DIR);
    if !pages_path.exists() {
        return Err(format!("{PAGES_DIR}/ directory not found"));
    }

    let page_count = process_dir(pages_path, pages_path, Path::new(DIST_DIR), &components, &layout)?;
    println!("built {page_count} page(s)");

    // Copy static files
    let static_path = Path::new(STATIC_DIR);
    if static_path.exists() {
        let copied = copy_dir_recursive(static_path, Path::new(DIST_DIR))?;
        println!("copied {copied} static file(s)");
    }

    println!("-> {DIST_DIR}/");
    Ok(())
}

/// Recursively processes a directory of pages.
fn process_dir(
    dir: &Path,
    pages_root: &Path,
    dist_root: &Path,
    components: &std::collections::HashMap<String, component::Component>,
    layout: &Option<String>,
) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();

        if path.is_dir() {
            count += process_dir(&path, pages_root, dist_root, components, layout)?;
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;

        // Extract <meta> tags as props, remove them from body
        let (props, body) = extract_meta(&content);

        // Resolve components in page body
        let resolved = parser::resolve(&body, components);

        // Wrap in layout if available
        let final_html = if let Some(layout_tpl) = layout {
            component::render_layout(layout_tpl, &props, &resolved)
        } else {
            resolved
        };

        // Compute output path with clean URLs:
        // about.html -> about/index.html (served as /about/)
        // index.html stays index.html
        let rel = path
            .strip_prefix(pages_root)
            .map_err(|e| format!("strip prefix: {e}"))?;
        let out_path = if rel.file_stem().and_then(|s| s.to_str()) == Some("index") {
            dist_root.join(rel)
        } else {
            let without_ext = rel.with_extension("");
            dist_root.join(without_ext).join("index.html")
        };

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }

        fs::write(&out_path, final_html)
            .map_err(|e| format!("write {}: {e}", out_path.display()))?;

        count += 1;
    }

    Ok(count)
}

/// Extracts `<meta name="key" content="value">` tags from the page content.
/// Returns the props and the body with meta tags removed.
fn extract_meta(content: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut props = std::collections::HashMap::new();
    let mut body = String::with_capacity(content.len());

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(meta) = parse_meta_tag(trimmed) {
            props.insert(meta.0, meta.1);
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }

    // Remove trailing newline added by the loop
    if body.ends_with('\n') && !content.ends_with('\n') {
        body.pop();
    }

    (props, body)
}

/// Parses a single `<meta name="..." content="...">` tag.
fn parse_meta_tag(line: &str) -> Option<(String, String)> {
    let line = line.strip_prefix("<meta ")?;
    let line = line.strip_suffix('>')?.strip_suffix('/').unwrap_or(line);
    let line = line.trim();

    let mut name = None;
    let mut content = None;

    // Simple attribute parsing for name="..." and content="..."
    let mut remaining = line;
    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if let Some(rest) = remaining.strip_prefix("name=") {
            let (val, r) = parse_quoted_value(rest)?;
            name = Some(val);
            remaining = r;
        } else if let Some(rest) = remaining.strip_prefix("content=") {
            let (val, r) = parse_quoted_value(rest)?;
            content = Some(val);
            remaining = r;
        } else {
            // Skip unknown attribute
            let end = remaining.find(|c: char| c.is_ascii_whitespace()).unwrap_or(remaining.len());
            remaining = &remaining[end..];
        }
    }

    Some((name?, content?))
}

fn parse_quoted_value(input: &str) -> Option<(String, &str)> {
    let input = input.trim_start();
    let quote = input.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &input[1..];
    let end = rest.find(quote)?;
    let value = rest[..end].to_string();
    Some((value, &rest[end + 1..]))
}

/// Recursively copies a directory's contents into a destination.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        let dest = dst.join(path.file_name().unwrap());

        if path.is_dir() {
            fs::create_dir_all(&dest)
                .map_err(|e| format!("create dir {}: {e}", dest.display()))?;
            count += copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).map_err(|e| {
                format!("copy {} -> {}: {e}", path.display(), dest.display())
            })?;
            count += 1;
        }
    }

    Ok(count)
}

fn create_dir(path: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("create {path}: {e}"))
}

fn write_file(path: &str, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("write {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_extraction() {
        let input = "<meta name=\"title\" content=\"Hello\">\n<p>body</p>";
        let (props, body) = extract_meta(input);
        assert_eq!(props.get("title").unwrap(), "Hello");
        assert_eq!(body.trim(), "<p>body</p>");
    }

    #[test]
    fn multiple_meta() {
        let input = "<meta name=\"title\" content=\"Hi\">\n<meta name=\"lang\" content=\"fr\">\n<p>ok</p>";
        let (props, body) = extract_meta(input);
        assert_eq!(props.get("title").unwrap(), "Hi");
        assert_eq!(props.get("lang").unwrap(), "fr");
        assert_eq!(body.trim(), "<p>ok</p>");
    }

    #[test]
    fn no_meta() {
        let input = "<p>just html</p>";
        let (props, body) = extract_meta(input);
        assert!(props.is_empty());
        assert_eq!(body.trim(), "<p>just html</p>");
    }
}
