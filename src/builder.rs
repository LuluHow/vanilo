use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::component;
use crate::content;
use crate::css;
use crate::lint;
use crate::parser;

const PAGES_DIR: &str = "pages";
const COMPONENTS_DIR: &str = "components";
const STATIC_DIR: &str = "static";
const FUNCTIONS_DIR: &str = "functions";
const CONTENT_DIR: &str = "content";
const DIST_DIR: &str = "dist";
const LAYOUT_FILE: &str = "layout.html";

/// Minimal client-side runtime for rendering components from <template> elements.
const TEMPLATES_RUNTIME: &str = r#"<script>(function(){
var S=window.Simple={};
S.esc=function(s){var d=document.createElement("div");d.appendChild(document.createTextNode(s));return d.innerHTML};
S.render=function(n,p){var t=document.getElementById("tpl-"+n);if(!t)return"";var h=t.innerHTML;if(p){var k=Object.keys(p);for(var i=0;i<k.length;i++){var v=p[k[i]]!=null?String(p[k[i]]):"";h=h.split("{{"+k[i]+"}}").join(S.esc(v))}}return h.replace(/\{\{[^}]+\}\}/g,"")};
S.put=function(sel,n,p){var el=typeof sel==="string"?document.querySelector(sel):sel;if(el)el.innerHTML=S.render(n,p)};
S.list=function(sel,n,arr){var el=typeof sel==="string"?document.querySelector(sel):sel;if(!el||!arr)return;el.innerHTML=arr.map(function(p){return S.render(n,p)}).join("")};
})();</script>"#;

/// Scaffolds a new project structure.
pub fn init() -> Result<(), String> {
    create_dir(PAGES_DIR)?;
    create_dir(COMPONENTS_DIR)?;
    create_dir(STATIC_DIR)?;
    create_dir(FUNCTIONS_DIR)?;
    create_dir(CONTENT_DIR)?;

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

    // Header component
    let header_path = format!("{COMPONENTS_DIR}/Header.html");
    if !Path::new(&header_path).exists() {
        let header = r#"<header>
    <a href="/" class="logo">{{title}}</a>
</header>"#;
        write_file(&header_path, header)?;
    }

    // Footer component
    let footer_path = format!("{COMPONENTS_DIR}/Footer.html");
    if !Path::new(&footer_path).exists() {
        write_file(&footer_path, r#"<footer>
    <p>Built with simple</p>
</footer>"#)?;
    }

    // Index page
    let index_path = format!("{PAGES_DIR}/index.html");
    if !Path::new(&index_path).exists() {
        let index = r#"<meta name="title" content="Home">

<Header title="simple">
    <nav>
        <a href="/">Home</a>
        <a href="/about">About</a>
    </nav>
</Header>

<main>
    <section class="hero">
        <p class="badge">simple init</p>
        <h1>Your project is&nbsp;ready.</h1>
        <p class="sub">Static site generator with edge functions. Edit <code>pages/</code> and <code>components/</code>, then reload.</p>
    </section>

    <hr>

    <section class="grid">
        <div class="card">
            <h3>Components</h3>
            <p>Reusable HTML with props and children.</p>
            <pre><code>&lt;Header title="simple"&gt;
  &lt;nav&gt;...&lt;/nav&gt;
&lt;/Header&gt;</code></pre>
        </div>
        <div class="card">
            <h3>Client-side rendering</h3>
            <p>Reuse components from JS. No innerHTML.</p>
            <pre><code>Simple.list('#el', 'Card', items)
Simple.put('#el', 'Card', item)</code></pre>
        </div>
        <div class="card">
            <h3>Edge functions</h3>
            <p>Server-side JS via QuickJS on /api/*.</p>
            <pre><code>function handler(req) {
  return { body: "hello" }
}</code></pre>
        </div>
        <div class="card">
            <h3>SQLite</h3>
            <p>Built-in database in every function.</p>
            <pre><code>db.query("SELECT * FROM t")
db.exec("INSERT INTO t ...")</code></pre>
        </div>
        <div class="card">
            <h3>Deploy</h3>
            <p>Docker image, one command to ship.</p>
            <pre><code>docker build -t my-site .
docker compose up -d</code></pre>
        </div>
    </section>

    <hr>

    <section class="counter">
        <p>This page has been viewed <strong id="visit-count">-</strong> times</p>
        <p class="note">Powered by <code>/api/hello</code> — an edge function backed by SQLite.</p>
    </section>
</main>

<Footer />"#;
        write_file(&index_path, index)?;
    }

    // About page
    let about_path = format!("{PAGES_DIR}/about.html");
    if !Path::new(&about_path).exists() {
        let about = r#"<meta name="title" content="About">

<Header title="simple">
    <nav>
        <a href="/">Home</a>
        <a href="/about">About</a>
    </nav>
</Header>

<main>
    <div class="content">
        <h1>About</h1>
        <p>simple is a static site generator with edge functions, written in Rust.</p>
        <p>Zero config, zero frontend dependencies. Write HTML and components, build your site. Add JavaScript functions for server-side logic backed by SQLite.</p>
        <p>This page lives at <code>pages/about.html</code> and is served at <code>/about</code> — clean URLs are automatic.</p>
    </div>
</main>

<Footer />"#;
        write_file(&about_path, about)?;
    }

    // Example edge function with db
    let fn_path = format!("{FUNCTIONS_DIR}/hello.js");
    if !Path::new(&fn_path).exists() {
        let hello = r#"function handler(req) {
    db.exec("CREATE TABLE IF NOT EXISTS visits (count INTEGER)");

    var row = db.query("SELECT count FROM visits");
    if (row.length === 0) {
        db.exec("INSERT INTO visits VALUES (1)");
    } else {
        db.exec("UPDATE visits SET count = count + 1");
    }

    var result = db.query("SELECT count FROM visits");
    return {
        status: 200,
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ visits: result[0].count })
    }
}"#;
        write_file(&fn_path, hello)?;
    }

    // Starter CSS/JS
    if !Path::new(&format!("{STATIC_DIR}/style.css")).exists() {
        write_file(&format!("{STATIC_DIR}/style.css"), include_str!("scaffold/style.css"))?;
    }
    if !Path::new(&format!("{STATIC_DIR}/main.js")).exists() {
        write_file(&format!("{STATIC_DIR}/main.js"), include_str!("scaffold/main.js"))?;
    }

    // Config
    if !Path::new("simple.toml").exists() {
        let config = r#"# simple.toml — project configuration
# Override port/host with env vars: PORT, HOST

port = 3000
host = "127.0.0.1"

max_body = 1            # request body limit, MB
max_connections = 128
rate_limit = 60         # requests per window on /api/*
rate_window = 60        # rate limit window, seconds

timeout = 5             # JS execution timeout, seconds
memory = 32             # JS runtime memory limit, MB
fetch_timeout = 10      # outbound HTTP timeout, seconds

[security_headers]
content_security_policy = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; font-src 'self' https://cdn.jsdelivr.net"
# Uncomment to override other defaults. Empty string disables a header.
# strict_transport_security = "max-age=63072000; includeSubDomains"
# x_frame_options = "DENY"
# referrer_policy = "strict-origin-when-cross-origin"
# permissions_policy = "camera=(), microphone=(), geolocation=()"
# cross_origin_opener_policy = "same-origin"
# cross_origin_resource_policy = "same-origin"
"#;
        write_file("simple.toml", config)?;
    }

    // Dockerfile
    if !Path::new("Dockerfile").exists() {
        let dockerfile = r#"FROM simple
WORKDIR /app
COPY . .
RUN simple build
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["simple", "serve"]
"#;
        write_file("Dockerfile", dockerfile)?;
    }

    // Example content
    let site_json = format!("{CONTENT_DIR}/site.json");
    if !Path::new(&site_json).exists() {
        let content_example = r#"{
    "title": "My Site",
    "description": "Built with simple"
}"#;
        write_file(&site_json, content_example)?;
    }

    // .dockerignore
    if !Path::new(".dockerignore").exists() {
        let ignore = "dist/\ndata.db\n.git/\n*.db\n*.sqlite*\n*.env\n*.log\ntarget/\n";
        write_file(".dockerignore", ignore)?;
    }

    // compose.yaml
    if !Path::new("compose.yaml").exists() {
        let compose = r#"services:
  app:
    build: .
    ports:
      - "127.0.0.1:3000:3000"
    volumes:
      - ./data.db:/app/data.db
    restart: unless-stopped
"#;
        write_file("compose.yaml", compose)?;
    }

    println!("project initialized:");
    println!("  {LAYOUT_FILE}");
    println!("  {PAGES_DIR}/index.html");
    println!("  {PAGES_DIR}/about.html");
    println!("  {COMPONENTS_DIR}/Header.html");
    println!("  {COMPONENTS_DIR}/Footer.html");
    println!("  {FUNCTIONS_DIR}/hello.js");
    println!("  {CONTENT_DIR}/site.json");
    println!("  {STATIC_DIR}/style.css");
    println!("  {STATIC_DIR}/main.js");
    println!("  simple.toml");
    println!("  Dockerfile");
    println!("  compose.yaml");
    println!();
    println!("run `simple serve` to start dev server");
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

    // Load content (JSON CMS)
    let content_data = content::load(Path::new(CONTENT_DIR))?;
    if !content_data.is_empty() {
        println!("loaded {} content file(s)", content_data.len());
    }

    // Security lint check
    lint::check(
        Path::new(PAGES_DIR),
        Path::new(COMPONENTS_DIR),
        Path::new(FUNCTIONS_DIR),
    )?;

    // Build component templates block only if JS uses the client-side runtime
    let templates_block = if uses_client_runtime(Path::new(STATIC_DIR)) {
        build_templates_block(&components)
    } else {
        String::new()
    };

    // Process pages
    let pages_path = Path::new(PAGES_DIR);
    if !pages_path.exists() {
        return Err(format!("{PAGES_DIR}/ directory not found"));
    }

    let page_count = process_dir(pages_path, pages_path, Path::new(DIST_DIR), &components, &layout, &templates_block, &content_data)?;
    println!("built {page_count} page(s)");

    // Copy static files
    let static_path = Path::new(STATIC_DIR);
    if static_path.exists() {
        let copied = copy_dir_recursive(static_path, Path::new(DIST_DIR))?;
        println!("copied {copied} static file(s)");
    }

    // CSS tree-shaking: inline only used rules per page
    let inlined = inline_critical_css(Path::new(DIST_DIR))?;
    if inlined > 0 {
        println!("inlined CSS for {inlined} page(s)");
    }

    // Minify JS files in dist/
    let minified_js = minify_js_files(Path::new(DIST_DIR))?;
    if minified_js > 0 {
        println!("minified {minified_js} JS file(s)");
    }

    // Content-hash CSS/JS filenames for cache busting
    let renames = hash_static_assets(Path::new(DIST_DIR))?;
    if !renames.is_empty() {
        rewrite_html_refs(Path::new(DIST_DIR), &renames)?;
        println!("hashed {} asset(s)", renames.len());
    }

    // Pre-compress with gzip + brotli
    let compressed = precompress_dir(Path::new(DIST_DIR))?;
    if compressed > 0 {
        println!("pre-compressed {compressed} file(s)");
    }

    println!("-> {DIST_DIR}/");
    Ok(())
}

/// Returns true if any JS file in the static directory references `Simple.`,
/// indicating the project uses the client-side rendering runtime.
fn uses_client_runtime(static_dir: &Path) -> bool {
    if !static_dir.exists() {
        return false;
    }
    scan_js_for_simple(static_dir)
}

fn scan_js_for_simple(dir: &Path) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if scan_js_for_simple(&path) {
                return true;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("js") {
            if let Ok(content) = fs::read_to_string(&path) {
                if content.contains("Simple.") {
                    return true;
                }
            }
        }
    }
    false
}

/// Builds `<template>` elements for all components + the runtime script.
/// Injected before `</body>` so components are reusable from client-side JS.
fn build_templates_block(
    components: &std::collections::HashMap<String, component::Component>,
) -> String {
    if components.is_empty() {
        return String::new();
    }

    let mut block = String::new();

    // Sort for deterministic output
    let mut names: Vec<&String> = components.keys().collect();
    names.sort();

    for name in &names {
        let comp = &components[*name];
        block.push_str("<template id=\"tpl-");
        block.push_str(name);
        block.push_str("\">");
        block.push_str(comp.template.trim());
        block.push_str("</template>");
    }

    block.push_str(TEMPLATES_RUNTIME);
    block
}

/// Inserts content just before `</body>`, or appends if no `</body>` found.
fn inject_before_body_close(html: &str, content: &str) -> String {
    if content.is_empty() {
        return html.to_string();
    }
    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + content.len());
        result.push_str(&html[..pos]);
        result.push_str(content);
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{html}{content}")
    }
}

/// Recursively processes a directory of pages.
fn process_dir(
    dir: &Path,
    pages_root: &Path,
    dist_root: &Path,
    components: &std::collections::HashMap<String, component::Component>,
    layout: &Option<String>,
    templates_block: &str,
    content_data: &std::collections::HashMap<String, content::Value>,
) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();

        if path.is_dir() {
            count += process_dir(&path, pages_root, dist_root, components, layout, templates_block, content_data)?;
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }

        let content =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;

        // Extract <meta> tags as props, remove them from body
        let (props, body) = extract_meta(&content);

        // Expand <Each content="..."> directives
        let body = content::expand_each(&body, content_data);

        // Resolve {{@...}} content placeholders (before components, so they work as props)
        let body = content::resolve_placeholders(&body, content_data);

        // Resolve components in page body
        let resolved = parser::resolve(&body, components);

        // Wrap in layout if available
        let final_html = if let Some(layout_tpl) = layout {
            component::render_layout(layout_tpl, &props, &resolved)
        } else {
            resolved
        };

        // Resolve {{@...}} in layout (second pass for layout-level references)
        let final_html = content::resolve_placeholders(&final_html, content_data);

        // Inject component templates for runtime JS rendering
        let final_html = inject_before_body_close(&final_html, templates_block);

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

        let minified = minify_html(&final_html);
        fs::write(&out_path, minified)
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

/// File extensions that must never be copied to dist/ or served.
const BLOCKED_EXTENSIONS: &[&str] = &[
    ".db", ".sqlite", ".sqlite3",
    ".env",
    ".key", ".pem", ".p12", ".pfx",
    ".sh", ".bash",
    ".sql",
    ".log",
    ".envrc", ".htaccess",
    ".bak", ".swp", ".swo",
];

const BLOCKED_FILENAMES: &[&str] = &[
    ".ds_store", ".gitignore", ".gitmodules",
];

fn is_blocked_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    if BLOCKED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }
    // Block .env.* variants (e.g., .env.local, .env.production)
    if lower.starts_with(".env.") {
        return true;
    }
    BLOCKED_FILENAMES.iter().any(|n| lower == *n)
}

/// Recursively copies a directory's contents into a destination.
/// Skips files with sensitive extensions.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if is_blocked_file(file_name) {
            println!("  skipped {}: blocked extension", path.display());
            continue;
        }

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

// ---------------------------------------------------------------------------
// CSS tree-shaking (inline critical CSS per page)
// ---------------------------------------------------------------------------

/// Inlines tree-shaken CSS into each HTML page.
/// Replaces `<link rel="stylesheet" href="...">` with `<style>` containing only used rules.
/// Removes CSS files from dist/ after inlining.
fn inline_critical_css(dist: &Path) -> Result<usize, String> {
    let css_files = collect_css_files(dist, dist)?;
    if css_files.is_empty() {
        return Ok(0);
    }

    // Read CSS file contents
    let mut css_map: Vec<(String, std::path::PathBuf, String)> = Vec::new();
    for (url, path) in &css_files {
        let content =
            fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        css_map.push((url.clone(), path.clone(), content));
    }

    // Inline into every HTML file
    let mut count = 0;
    let mut used_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    inline_css_walk(dist, &css_map, &mut count, &mut used_urls)?;

    // Remove CSS files that were inlined
    for (url, path, _) in &css_map {
        if used_urls.contains(url) {
            let _ = fs::remove_file(path);
        }
    }

    Ok(count)
}

fn collect_css_files(
    dir: &Path,
    root: &Path,
) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_css_files(&path, root)?);
        } else if path.extension().and_then(|e| e.to_str()) == Some("css") {
            let rel = path.strip_prefix(root).map_err(|e| format!("strip: {e}"))?;
            let url = format!("/{}", rel.display()).replace('\\', "/");
            files.push((url, path));
        }
    }
    Ok(files)
}

fn inline_css_walk(
    dir: &Path,
    css_map: &[(String, std::path::PathBuf, String)],
    count: &mut usize,
    used: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            inline_css_walk(&path, css_map, count, used)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }

        let mut html =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut changed = false;

        for (url, _, css_content) in css_map {
            if let Some(new_html) = replace_link_with_style(&html, url, css_content) {
                html = new_html;
                changed = true;
                used.insert(url.clone());
            }
        }

        if changed {
            fs::write(&path, &html).map_err(|e| format!("write {}: {e}", path.display()))?;
            *count += 1;
        }
    }
    Ok(())
}

/// Finds a `<link rel="stylesheet" href="URL">` tag and replaces it with
/// `<style>` containing tree-shaken CSS. Returns None if no matching link found.
fn replace_link_with_style(html: &str, css_url: &str, css_content: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(start) = html[search_from..].find("<link ") {
        let abs_start = search_from + start;
        let rest = &html[abs_start..];
        let tag_end = rest.find('>')? + 1;
        let tag = &rest[..tag_end];

        let is_stylesheet =
            tag.contains("rel=\"stylesheet\"") || tag.contains("rel='stylesheet'");
        let has_href = tag.contains(&format!("href=\"{css_url}\""))
            || tag.contains(&format!("href='{css_url}'"));

        if is_stylesheet && has_href {
            let pruned = css::tree_shake(css_content, html);

            let mut result = String::with_capacity(html.len() + pruned.len());
            result.push_str(&html[..abs_start]);
            if !pruned.is_empty() {
                result.push_str("<style>");
                result.push_str(&pruned);
                result.push_str("</style>");
            }
            result.push_str(&html[abs_start + tag_end..]);
            return Some(result);
        }

        search_from = abs_start + tag_end;
    }

    None
}

// ---------------------------------------------------------------------------
// JS minification
// ---------------------------------------------------------------------------

/// Minifies all `.js` files in dist/.
fn minify_js_files(dist: &Path) -> Result<usize, String> {
    let mut count = 0;
    minify_js_walk(dist, &mut count)?;
    Ok(count)
}

fn minify_js_walk(dir: &Path, count: &mut usize) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            minify_js_walk(&path, count)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        let content =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let minified = minify_js(&content);
        if minified.len() < content.len() {
            fs::write(&path, &minified)
                .map_err(|e| format!("write {}: {e}", path.display()))?;
            *count += 1;
        }
    }
    Ok(())
}

/// Minifies JavaScript: strips comments, trims lines, joins where safe.
fn minify_js(js: &str) -> String {
    let stripped = strip_js_comments(js);
    let lines: Vec<&str> = stripped
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(js.len());
    out.push_str(lines[0]);

    for i in 1..lines.len() {
        if !js_line_continues(lines[i - 1]) {
            out.push('\n');
        }
        out.push_str(lines[i]);
    }

    out
}

/// Returns true when a line ends with a token that guarantees the expression
/// continues on the next line (ASI cannot insert a semicolon here).
fn js_line_continues(line: &str) -> bool {
    let bytes = line.trim_end().as_bytes();
    if bytes.is_empty() {
        return false;
    }

    let last = bytes[bytes.len() - 1];

    // ++ and -- are complete postfix expressions — don't join
    if bytes.len() >= 2 {
        let prev = bytes[bytes.len() - 2];
        if (last == b'+' && prev == b'+') || (last == b'-' && prev == b'-') {
            return false;
        }
    }

    matches!(
        last,
        b'{' | b'(' | b'[' | b',' | b'=' | b'?' | b':' | b'+' | b'-' | b'*'
            | b'/' | b'%' | b'~' | b'^' | b'&' | b'|' | b'<' | b'>' | b'!'
    )
}

/// Strips `//` and `/* */` comments from JavaScript, respecting string literals.
fn strip_js_comments(js: &str) -> String {
    let mut out = String::with_capacity(js.len());
    let mut chars = js.chars().peekable();

    // 0 = normal, 1 = single-quote, 2 = double-quote, 3 = backtick
    let mut string_state: u8 = 0;

    while let Some(c) = chars.next() {
        if string_state != 0 {
            out.push(c);
            if c == '\\' {
                // Escape: pass next char through
                if let Some(nc) = chars.next() {
                    out.push(nc);
                }
            } else if (string_state == 1 && c == '\'')
                || (string_state == 2 && c == '"')
                || (string_state == 3 && c == '`')
            {
                string_state = 0;
            }
            continue;
        }

        match c {
            '\'' => {
                string_state = 1;
                out.push(c);
            }
            '"' => {
                string_state = 2;
                out.push(c);
            }
            '`' => {
                string_state = 3;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some(&'/') => {
                    chars.next();
                    for nc in chars.by_ref() {
                        if nc == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some(&'*') => {
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
                }
                _ => out.push('/'),
            },
            _ => out.push(c),
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Content-hash filenames
// ---------------------------------------------------------------------------

/// Extensions that get content-hash filenames for cache busting.
const HASHABLE_EXTENSIONS: &[&str] = &["css", "js"];

/// Extensions that benefit from pre-compression.
const COMPRESSIBLE_EXTENSIONS: &[&str] = &["html", "css", "js", "json", "svg", "xml", "txt"];

/// Renames CSS/JS files with content hash: style.css → style.a1b2c3d4.css
fn hash_static_assets(dist: &Path) -> Result<Vec<(String, String)>, String> {
    let mut renames = Vec::new();
    collect_hashable(dist, dist, &mut renames)?;
    Ok(renames)
}

fn collect_hashable(
    dir: &Path,
    dist_root: &Path,
    renames: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_hashable(&path, dist_root, renames)?;
            continue;
        }

        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_string(),
            None => continue,
        };

        if !HASHABLE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let data = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let hash = content_hash(&data);

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let new_name = format!("{stem}.{hash}.{ext}");
        let new_path = path.with_file_name(&new_name);

        fs::rename(&path, &new_path)
            .map_err(|e| format!("rename {}: {e}", path.display()))?;

        let old_rel = path
            .strip_prefix(dist_root)
            .map_err(|e| format!("strip: {e}"))?;
        let new_rel = new_path
            .strip_prefix(dist_root)
            .map_err(|e| format!("strip: {e}"))?;

        // Use forward slashes for URL paths
        let old_url = format!("/{}", old_rel.display()).replace('\\', "/");
        let new_url = format!("/{}", new_rel.display()).replace('\\', "/");

        renames.push((old_url, new_url));
    }

    Ok(())
}

/// FNV-1a hash truncated to 8 hex chars.
fn content_hash(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", hash as u32)
}

/// Rewrites references in HTML files to use hashed filenames.
fn rewrite_html_refs(dir: &Path, renames: &[(String, String)]) -> Result<(), String> {
    if renames.is_empty() {
        return Ok(());
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rewrite_html_refs(&path, renames)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }

        let mut content =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;

        for (old_url, new_url) in renames {
            content = content.replace(old_url.as_str(), new_url.as_str());
            // Also match without leading slash (relative refs)
            if let Some(old_no_slash) = old_url.strip_prefix('/') {
                if let Some(new_no_slash) = new_url.strip_prefix('/') {
                    content = content.replace(old_no_slash, new_no_slash);
                }
            }
        }

        fs::write(&path, content)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-compression (gzip + brotli)
// ---------------------------------------------------------------------------

/// Pre-compresses all compressible files with gzip and brotli at build time.
fn precompress_dir(dir: &Path) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += precompress_dir(&path)?;
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext == "gz" || ext == "br" {
            continue;
        }

        // For hashed files like style.a1b2c3d4.css, check the real extension
        let real_ext = real_extension(&path);
        if !COMPRESSIBLE_EXTENSIONS.contains(&real_ext.as_str()) {
            continue;
        }

        let data = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if data.len() < 256 {
            continue;
        }

        // Skip if already pre-compressed
        let path_str = path.display().to_string();
        if Path::new(&format!("{path_str}.gz")).exists()
            && Path::new(&format!("{path_str}.br")).exists()
        {
            continue;
        }

        // Gzip (best compression for static files)
        let gz_path = format!("{}.gz", path.display());
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&data)
            .map_err(|e| format!("gzip: {e}"))?;
        let gz_data = encoder.finish().map_err(|e| format!("gzip: {e}"))?;
        fs::write(&gz_path, &gz_data).map_err(|e| format!("write {gz_path}: {e}"))?;

        // Brotli (quality 11 = max, for static assets)
        let br_path = format!("{}.br", path.display());
        let mut br_data = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut br_data, 4096, 11, 22);
            writer
                .write_all(&data)
                .map_err(|e| format!("brotli: {e}"))?;
        }
        fs::write(&br_path, &br_data).map_err(|e| format!("write {br_path}: {e}"))?;

        count += 1;
    }

    Ok(count)
}

/// Gets the real extension, skipping content-hash segments.
/// e.g. "style.a1b2c3d4.css" → "css", "main.js" → "js"
fn real_extension(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 1].to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// HTML minification
// ---------------------------------------------------------------------------

/// Minifies HTML: strips comments, collapses inter-tag whitespace.
/// Preserves whitespace inside <pre>, <code>, <script>, <style>, <textarea>.
fn minify_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut remaining = html;

    while !remaining.is_empty() {
        // Check for preserved blocks
        if let Some(block) = find_preserved_block(remaining) {
            // Collapse whitespace in the part before the block
            let before = &remaining[..block.start];
            collapse_whitespace(before, &mut out);
            // Copy the preserved block as-is
            out.push_str(&remaining[block.start..block.end]);
            remaining = &remaining[block.end..];
            continue;
        }

        // No more preserved blocks — collapse the rest
        collapse_whitespace(remaining, &mut out);
        break;
    }

    out
}

struct PreservedBlock {
    start: usize,
    end: usize,
}

fn find_preserved_block(html: &str) -> Option<PreservedBlock> {
    let tags = ["<pre", "<code", "<script", "<style", "<textarea", "<template"];
    let mut earliest: Option<(usize, &str)> = None;

    for tag in &tags {
        if let Some(pos) = html.to_lowercase().find(tag) {
            if earliest.is_none() || pos < earliest.unwrap().0 {
                earliest = Some((pos, &tag[1..])); // strip '<' for closing tag
            }
        }
    }

    let (start, tag_name) = earliest?;
    let closing = format!("</{tag_name}>");
    let after_open = start + tag_name.len() + 1;
    let close_pos = html[after_open..]
        .to_lowercase()
        .find(&closing)
        .map(|p| after_open + p + closing.len())?;

    Some(PreservedBlock {
        start,
        end: close_pos,
    })
}

fn collapse_whitespace(html: &str, out: &mut String) {
    // Strip HTML comments
    let mut remaining = html;
    while let Some(start) = remaining.find("<!--") {
        out.push_str(&collapse_inter_tag(&remaining[..start]));
        if let Some(end) = remaining[start..].find("-->") {
            remaining = &remaining[start + end + 3..];
        } else {
            remaining = "";
        }
    }
    out.push_str(&collapse_inter_tag(remaining));
}

/// Collapses runs of whitespace between tags into a single space.
fn collapse_inter_tag(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            last_was_space = false;
            result.push(c);
        } else if c == '>' {
            in_tag = false;
            last_was_space = false;
            result.push(c);
        } else if in_tag {
            result.push(c);
        } else if c.is_ascii_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            last_was_space = false;
            result.push(c);
        }
    }

    result
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
    use std::fs;

    // -----------------------------------------------------------------------
    // Meta extraction (existing)
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Content hash
    // -----------------------------------------------------------------------

    #[test]
    fn content_hash_deterministic() {
        let data = b"body { color: red; }";
        let h1 = content_hash(data);
        let h2 = content_hash(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let h1 = content_hash(b"body { color: red; }");
        let h2 = content_hash(b"body { color: blue; }");
        assert_ne!(h1, h2);
    }

    // -----------------------------------------------------------------------
    // Hash static assets + rewrite
    // -----------------------------------------------------------------------

    #[test]
    fn hash_renames_css_and_js() {
        let dir = tempdir("hash_renames");
        fs::write(dir.join("style.css"), "body{}").unwrap();
        fs::write(dir.join("main.js"), "console.log(1)").unwrap();
        fs::write(dir.join("photo.png"), "fakepng").unwrap();

        let renames = hash_static_assets(&dir).unwrap();

        // CSS and JS should be renamed
        assert_eq!(renames.len(), 2);
        // PNG should NOT be renamed
        assert!(dir.join("photo.png").exists());
        // Original CSS/JS should be gone
        assert!(!dir.join("style.css").exists());
        assert!(!dir.join("main.js").exists());

        // Hashed files should exist
        for (_, new_url) in &renames {
            let rel = new_url.strip_prefix('/').unwrap();
            assert!(dir.join(rel).exists(), "missing: {new_url}");
        }

        cleanup(&dir);
    }

    #[test]
    fn rewrite_updates_html_references() {
        let dir = tempdir("rewrite_refs");
        fs::write(
            dir.join("index.html"),
            r#"<link rel="stylesheet" href="/style.css"><script src="/main.js"></script>"#,
        )
        .unwrap();
        fs::write(dir.join("style.css"), "body{}").unwrap();
        fs::write(dir.join("main.js"), "x()").unwrap();

        let renames = hash_static_assets(&dir).unwrap();
        rewrite_html_refs(&dir, &renames).unwrap();

        let html = fs::read_to_string(dir.join("index.html")).unwrap();
        // Should no longer contain unhashed refs
        assert!(!html.contains("/style.css\""), "still has /style.css");
        assert!(!html.contains("/main.js\""), "still has /main.js");
        // Should contain hashed refs
        for (_, new_url) in &renames {
            assert!(html.contains(new_url), "missing ref: {new_url}");
        }

        cleanup(&dir);
    }

    #[test]
    fn hash_is_content_based_not_name_based() {
        let dir = tempdir("hash_content");
        fs::write(dir.join("a.css"), "same-content").unwrap();
        let renames_a = hash_static_assets(&dir).unwrap();

        // Recreate with different filename, same content
        cleanup(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.css"), "same-content").unwrap();
        let renames_b = hash_static_assets(&dir).unwrap();

        // Extract hash from both renames
        let hash_a = extract_hash_from_url(&renames_a[0].1);
        let hash_b = extract_hash_from_url(&renames_b[0].1);
        assert_eq!(hash_a, hash_b, "same content should produce same hash");

        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // Pre-compression
    // -----------------------------------------------------------------------

    #[test]
    fn precompress_creates_gz_and_br() {
        let dir = tempdir("precompress");
        // Must be > 256 bytes to trigger compression
        let big_css = "a{color:red;}".repeat(50);
        fs::write(dir.join("style.css"), &big_css).unwrap();
        fs::write(dir.join("photo.png"), "small").unwrap(); // too small + not compressible ext

        let count = precompress_dir(&dir).unwrap();
        assert_eq!(count, 1); // only CSS

        assert!(dir.join("style.css.gz").exists(), "missing .gz");
        assert!(dir.join("style.css.br").exists(), "missing .br");
        assert!(!dir.join("photo.png.gz").exists(), "should not compress png");

        cleanup(&dir);
    }

    #[test]
    fn precompressed_files_are_smaller() {
        let dir = tempdir("precompress_smaller");
        let big_html = "<div>hello world</div>\n".repeat(100);
        fs::write(dir.join("page.html"), &big_html).unwrap();

        precompress_dir(&dir).unwrap();

        let original_size = fs::metadata(dir.join("page.html")).unwrap().len();
        let gz_size = fs::metadata(dir.join("page.html.gz")).unwrap().len();
        let br_size = fs::metadata(dir.join("page.html.br")).unwrap().len();

        assert!(gz_size < original_size, "gzip should be smaller");
        assert!(br_size < original_size, "brotli should be smaller");
        assert!(br_size <= gz_size, "brotli should be <= gzip for text");

        cleanup(&dir);
    }

    #[test]
    fn precompress_skips_small_files() {
        let dir = tempdir("precompress_skip");
        fs::write(dir.join("tiny.css"), "a{}").unwrap(); // < 256 bytes

        let count = precompress_dir(&dir).unwrap();
        assert_eq!(count, 0);
        assert!(!dir.join("tiny.css.gz").exists());

        cleanup(&dir);
    }

    #[test]
    fn precompress_skips_already_compressed() {
        let dir = tempdir("precompress_double");
        let big = "x".repeat(300);
        fs::write(dir.join("app.js"), &big).unwrap();

        let count1 = precompress_dir(&dir).unwrap();
        assert_eq!(count1, 1);

        // Running again should not create .gz.gz or .br.br
        let count2 = precompress_dir(&dir).unwrap();
        assert_eq!(count2, 0, "should not re-compress .gz/.br files");
        assert!(!dir.join("app.js.gz.gz").exists());

        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // HTML minification
    // -----------------------------------------------------------------------

    #[test]
    fn minify_collapses_whitespace() {
        let input = "<div>   \n   <p>hello</p>   \n   </div>";
        let result = minify_html(input);
        assert_eq!(result, "<div> <p>hello</p> </div>");
    }

    #[test]
    fn minify_strips_comments() {
        let input = "<div><!-- comment --><p>ok</p></div>";
        let result = minify_html(input);
        assert_eq!(result, "<div><p>ok</p></div>");
    }

    #[test]
    fn minify_preserves_pre_content() {
        let input = "<pre>  multiple   spaces  </pre>";
        let result = minify_html(input);
        assert_eq!(result, "<pre>  multiple   spaces  </pre>");
    }

    #[test]
    fn minify_preserves_code_content() {
        let input = "<p>before</p>\n<code>  keep   this  </code>\n<p>after</p>";
        let result = minify_html(input);
        assert!(result.contains("  keep   this  "), "code content should be preserved");
    }

    #[test]
    fn minify_preserves_script_content() {
        let input = "<script>\n  if (x > 1) {\n    console.log(x);\n  }\n</script>";
        let result = minify_html(input);
        assert!(result.contains("if (x > 1)"), "script content should be preserved");
    }

    #[test]
    fn minify_handles_nested_preserved_blocks() {
        let input = "<div>\n  <pre>\n    hello\n  </pre>\n  <p>world</p>\n</div>";
        let result = minify_html(input);
        // pre content preserved, surrounding whitespace collapsed
        assert!(result.contains("<pre>\n    hello\n  </pre>"));
        assert!(result.contains("<p>world</p>"));
    }

    #[test]
    fn minify_empty_input() {
        assert_eq!(minify_html(""), "");
    }

    #[test]
    fn minify_no_html_tags() {
        let input = "just   some    text";
        let result = minify_html(input);
        assert_eq!(result, "just some text");
    }

    // -----------------------------------------------------------------------
    // real_extension helper
    // -----------------------------------------------------------------------

    #[test]
    fn real_extension_plain() {
        assert_eq!(real_extension(Path::new("style.css")), "css");
        assert_eq!(real_extension(Path::new("main.js")), "js");
    }

    #[test]
    fn real_extension_hashed() {
        assert_eq!(real_extension(Path::new("style.a1b2c3d4.css")), "css");
        assert_eq!(real_extension(Path::new("app.deadbeef.js")), "js");
    }

    #[test]
    fn real_extension_no_ext() {
        assert_eq!(real_extension(Path::new("Makefile")), "");
    }

    // -----------------------------------------------------------------------
    // Blocked files
    // -----------------------------------------------------------------------

    #[test]
    fn blocks_sensitive_extensions() {
        assert!(is_blocked_file(".env"));
        assert!(is_blocked_file("data.db"));
        assert!(is_blocked_file("key.pem"));
        assert!(is_blocked_file("deploy.sh"));
        assert!(is_blocked_file("dump.sql"));
        assert!(is_blocked_file(".env.local"));
        assert!(is_blocked_file(".env.production"));
    }

    #[test]
    fn allows_normal_files() {
        assert!(!is_blocked_file("style.css"));
        assert!(!is_blocked_file("main.js"));
        assert!(!is_blocked_file("photo.png"));
        assert!(!is_blocked_file("index.html"));
    }

    // -----------------------------------------------------------------------
    // Component templates + runtime
    // -----------------------------------------------------------------------

    #[test]
    fn templates_block_contains_all_components() {
        let mut comps = std::collections::HashMap::new();
        comps.insert(
            "Card".to_string(),
            component::Component {
                template: "<div class=\"card\">{{title}}</div>".to_string(),
            },
        );
        comps.insert(
            "Badge".to_string(),
            component::Component {
                template: "<span>{{label}}</span>".to_string(),
            },
        );
        let block = build_templates_block(&comps);
        assert!(block.contains("<template id=\"tpl-Card\">"));
        assert!(block.contains("<template id=\"tpl-Badge\">"));
        assert!(block.contains("{{title}}"));
        assert!(block.contains("{{label}}"));
        assert!(block.contains("Simple"));
    }

    #[test]
    fn templates_block_empty_when_no_components() {
        let comps = std::collections::HashMap::new();
        let block = build_templates_block(&comps);
        assert!(block.is_empty());
    }

    #[test]
    fn templates_block_sorted_deterministic() {
        let mut comps = std::collections::HashMap::new();
        comps.insert("Zebra".to_string(), component::Component { template: "<z></z>".to_string() });
        comps.insert("Alpha".to_string(), component::Component { template: "<a></a>".to_string() });
        let block = build_templates_block(&comps);
        let alpha_pos = block.find("tpl-Alpha").unwrap();
        let zebra_pos = block.find("tpl-Zebra").unwrap();
        assert!(alpha_pos < zebra_pos, "components should be sorted alphabetically");
    }

    #[test]
    fn inject_before_body() {
        let html = "<html><body><p>hi</p></body></html>";
        let result = inject_before_body_close(html, "<template>x</template>");
        assert_eq!(result, "<html><body><p>hi</p><template>x</template></body></html>");
    }

    #[test]
    fn inject_appends_without_body() {
        let html = "<div>no body tag</div>";
        let result = inject_before_body_close(html, "<template>x</template>");
        assert_eq!(result, "<div>no body tag</div><template>x</template>");
    }

    #[test]
    fn inject_noop_when_empty() {
        let html = "<html><body></body></html>";
        let result = inject_before_body_close(html, "");
        assert_eq!(result, html);
    }

    #[test]
    fn minify_preserves_template_content() {
        let input = "<template id=\"tpl-Card\"><div>  {{title}}  </div></template>";
        let result = minify_html(input);
        assert!(result.contains("  {{title}}  "), "template content should be preserved");
    }

    // -----------------------------------------------------------------------
    // Client runtime detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_simple_usage_in_js() {
        let dir = tempdir("runtime_yes");
        fs::write(dir.join("app.js"), "Simple.list('#el', 'Card', items);").unwrap();
        assert!(uses_client_runtime(&dir));
        cleanup(&dir);
    }

    #[test]
    fn no_runtime_without_simple() {
        let dir = tempdir("runtime_no");
        fs::write(dir.join("app.js"), "fetch('/api/hello').then(r => r.json());").unwrap();
        assert!(!uses_client_runtime(&dir));
        cleanup(&dir);
    }

    #[test]
    fn no_runtime_empty_dir() {
        let dir = tempdir("runtime_empty");
        assert!(!uses_client_runtime(&dir));
        cleanup(&dir);
    }

    #[test]
    fn no_runtime_missing_dir() {
        assert!(!uses_client_runtime(Path::new("/tmp/does_not_exist_simple_rt")));
    }

    // -----------------------------------------------------------------------
    // JS minification
    // -----------------------------------------------------------------------

    #[test]
    fn minify_js_strips_line_comments() {
        let input = "var x = 1; // comment\nvar y = 2;";
        let result = minify_js(input);
        assert!(result.contains("var x = 1;"));
        assert!(result.contains("var y = 2;"));
        assert!(!result.contains("comment"));
    }

    #[test]
    fn minify_js_strips_block_comments() {
        let input = "var x = /* value */ 1;";
        let result = minify_js(input);
        assert!(result.contains("var x ="));
        assert!(result.contains("1;"));
        assert!(!result.contains("value"));
    }

    #[test]
    fn minify_js_preserves_strings() {
        let input = "var x = \"hello // world\";";
        let result = minify_js(input);
        assert!(result.contains("hello // world"));
    }

    #[test]
    fn minify_js_preserves_backtick_strings() {
        let input = "var x = `hello /* world */`;";
        let result = minify_js(input);
        assert!(result.contains("hello /* world */"));
    }

    #[test]
    fn minify_js_trims_indentation() {
        let input = "    var x = 1;\n    var y = 2;";
        let result = minify_js(input);
        assert!(!result.starts_with(' '));
        assert!(result.contains("var x = 1;"));
    }

    #[test]
    fn minify_js_removes_blank_lines() {
        let input = "var x = 1;\n\n\nvar y = 2;";
        let result = minify_js(input);
        assert_eq!(result, "var x = 1;\nvar y = 2;");
    }

    #[test]
    fn minify_js_joins_after_operator() {
        let input = "var x =\n    5;";
        let result = minify_js(input);
        assert_eq!(result, "var x =5;");
    }

    #[test]
    fn minify_js_joins_after_open_brace() {
        let input = "if (true) {\n    x = 1;\n}";
        let result = minify_js(input);
        assert!(result.contains("{x = 1;"), "should join after {{");
    }

    #[test]
    fn minify_js_preserves_return_newline() {
        let input = "return\nx";
        let result = minify_js(input);
        assert!(result.contains('\n'), "newline after return must be preserved");
    }

    #[test]
    fn minify_js_no_join_after_postfix() {
        let input = "x++\ny = 1";
        let result = minify_js(input);
        assert!(result.contains("x++\n"), "should not join after ++");
    }

    #[test]
    fn minify_js_empty() {
        assert_eq!(minify_js(""), "");
    }

    #[test]
    fn minify_js_comment_only() {
        assert_eq!(minify_js("// just a comment\n"), "");
    }

    #[test]
    fn minify_js_files_end_to_end() {
        let dir = tempdir("minify_js");
        let js = "// Comment\nvar x = 1;\n\n// Another\nvar y = 2;\n";
        fs::write(dir.join("app.js"), js).unwrap();
        fs::write(dir.join("photo.png"), "fake").unwrap();

        let count = minify_js_files(&dir).unwrap();
        assert_eq!(count, 1);

        let result = fs::read_to_string(dir.join("app.js")).unwrap();
        assert!(!result.contains("Comment"));
        assert!(result.contains("var x = 1;"));
        assert!(result.contains("var y = 2;"));

        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // CSS tree-shaking / inline
    // -----------------------------------------------------------------------

    #[test]
    fn replace_link_inlines_style() {
        let html = r#"<html><head><link rel="stylesheet" href="/style.css"></head><body><div class="card">hi</div></body></html>"#;
        let css = ".card { color: red; } .unused { display: none; }";
        let result = replace_link_with_style(html, "/style.css", css).unwrap();
        assert!(result.contains("<style>"), "should have <style> tag");
        assert!(!result.contains("<link"), "link tag should be removed");
        assert!(result.contains(".card"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn replace_link_no_match() {
        let html = r#"<html><head><link rel="stylesheet" href="/other.css"></head><body></body></html>"#;
        let css = "body { color: red; }";
        assert!(replace_link_with_style(html, "/style.css", css).is_none());
    }

    #[test]
    fn inline_critical_css_end_to_end() {
        let dir = tempdir("inline_css");
        let html = r#"<html><head><link rel="stylesheet" href="/style.css"></head><body><main><p>hello</p></main></body></html>"#;
        let css = "main { padding: 1rem; } .card { border: 1px solid; } p { margin: 0; }";

        fs::write(dir.join("index.html"), html).unwrap();
        fs::write(dir.join("style.css"), css).unwrap();

        let count = inline_critical_css(&dir).unwrap();
        assert_eq!(count, 1);

        let result = fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(result.contains("<style>"));
        assert!(result.contains("main"));
        assert!(result.contains("p{") || result.contains("p {"));
        assert!(!result.contains(".card"));
        assert!(!dir.join("style.css").exists(), "CSS file should be removed");

        cleanup(&dir);
    }

    #[test]
    fn inline_css_preserves_non_stylesheet_links() {
        let dir = tempdir("inline_preserve");
        let html = r#"<html><head><link rel="icon" href="/favicon.ico"><link rel="stylesheet" href="/style.css"></head><body><div>hi</div></body></html>"#;
        fs::write(dir.join("index.html"), html).unwrap();
        fs::write(dir.join("style.css"), "div { color: red; }").unwrap();

        inline_critical_css(&dir).unwrap();

        let result = fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(result.contains("rel=\"icon\""), "non-stylesheet link should remain");
        assert!(result.contains("<style>"));

        cleanup(&dir);
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("simple_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn extract_hash_from_url(url: &str) -> String {
        let filename = url.rsplit('/').next().unwrap();
        let parts: Vec<&str> = filename.split('.').collect();
        parts[parts.len() - 2].to_string()
    }
}
