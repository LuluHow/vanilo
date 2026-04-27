# Vanilo — Technical Documentation

> Static site generator with edge functions. One Rust binary, zero runtime dependencies.

---

## Table of contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Source modules](#3-source-modules)
4. [CLI and commands](#4-cli-and-commands)
5. [Project structure](#5-project-structure)
6. [Configuration (vanilo.toml)](#6-configuration-vanilotoml)
7. [Component system](#7-component-system)
8. [Template system](#8-template-system)
9. [Content CMS (JSON)](#9-content-cms-json)
10. [Edge functions (JS)](#10-edge-functions-js)
11. [Client-side runtime (Vanilo.*)](#11-client-side-runtime-vanilo)
12. [Server blocks (partial SSR)](#12-server-blocks-partial-ssr)
13. [CSS tree-shaking](#13-css-tree-shaking)
14. [Build pipeline](#14-build-pipeline)
15. [HTTP server](#15-http-server)
16. [Security](#16-security)
17. [Performance](#17-performance)
18. [Webhook (auto-rebuild)](#18-webhook-auto-rebuild)
19. [Deployment](#19-deployment)
20. [Rust dependencies](#20-rust-dependencies)
21. [Tests](#21-tests)
22. [Limits and non-goals](#22-limits-and-non-goals)

---

## 1. Overview

Vanilo is a full-stack web framework packed into a single Rust binary. It combines:

- **Static site generator**: HTML pages, reusable components, global layout
- **File-based CMS**: JSON data in `content/`, accessible from templates
- **Edge functions**: server-side JavaScript logic (embedded QuickJS)
- **Database**: built-in SQLite, accessible from edge functions
- **HTTP server**: dev + production server with compression, caching, security headers

**Philosophy**: zero external runtime dependencies. Everything is compiled into the binary.

**Tech stack**:
- Language: Rust (edition 2024)
- JS runtime: QuickJS via `rquickjs`
- Database: SQLite via `rusqlite` (bundled)
- Compression: gzip (`flate2`) + brotli (`brotli`)
- Outbound HTTP: `ureq`
- Crypto: `hmac` + `sha2` (webhooks)

---

## 2. Architecture

```
vanilo (binary)
  |
  |-- main.rs          CLI entry point
  |-- config.rs        Load vanilo.toml + env vars + defaults
  |-- builder.rs       Full build pipeline (init + build)
  |-- parser.rs        Resolve component tags in HTML
  |-- component.rs     Load, render, escape components
  |-- content.rs       JSON parser, CMS, <Each>, {{@...}} placeholders
  |-- css.rs           CSS tree-shaking (selectors vs HTML analysis)
  |-- functions.rs     QuickJS runtime, SQLite, fetch(), SSRF protection
  |-- lint.rs          Build-time security analysis (XSS, SQL injection)
  |-- server.rs        HTTP server, rate limiting, compression, SSR
  |-- scaffold/        Embedded templates (style.css, main.js)
```

**Data flow**:

```
vanilo build:
  content/*.json  -->  content::load()
  components/*.html -->  component::load_components()
  pages/*.html  -->  extract_meta() --> content::expand_each()
                -->  content::resolve_placeholders()
                -->  replace_server_blocks()
                -->  parser::resolve()
                -->  component::render_layout()
                -->  inject_templates_block()
                -->  inline_critical_css()
                -->  minify_html()
                -->  hash_static_assets()
                -->  precompress()
                -->  dist/

vanilo serve:
  build() --> server::serve()
    /api/*  -->  functions::execute() (QuickJS + SQLite)
    /*      -->  dist/ (static files + SSR server blocks)
```

---

## 3. Source modules

### 3.1 main.rs

Entry point. Parses CLI arguments and dispatches:

| Command | Action |
|---------|--------|
| `vanilo build` | `builder::build()` |
| `vanilo init` | `builder::init()` |
| `vanilo serve [port]` | `builder::build()`, spawn file watcher, then `server::serve(cfg, build_lock)` |

Port can be overridden as argument: `vanilo serve 8080`.

The `build_lock` (`Arc<Mutex<()>>`) is created in `main()` and shared between the watcher thread and the server (for webhook rebuilds) to prevent concurrent builds.

### 3.2 config.rs

**`Config` struct** — 27 fields covering:
- Network: `port`, `host`
- Server limits: `max_body`, `max_connections`, `rate_limit`, `rate_window`
- JS runtime: `timeout`, `memory`, `fetch_timeout`
- Security headers: CSP, HSTS, X-Frame-Options, Referrer-Policy, Permissions-Policy, COOP, CORP
- CORS: `api_cors`
- Per-function overrides: `functions` (`HashMap<String, FunctionConfig>`)
- Proxy: `trusted_proxy`
- Build: `minify_js`
- Webhook: `webhook_path`, `webhook_secret`, `webhook_rate_limit`, `webhook_rate_window`

**`FunctionConfig` struct** — per-function overrides (all optional, `None` = use global):
- `cors`: CORS policy override
- `rate_limit`: rate limit override
- `rate_window`: rate window override

**Configuration priority**: CLI args > env vars (`PORT`, `HOST`) > `vanilo.toml` > defaults.

**TOML parsing**: uses the `toml` crate with serde deserialization. A `RawConfig` struct with `#[derive(Deserialize)]` handles both flat keys and nested tables (`[security_headers]`, `[webhook]`, `[functions.*]`). Full TOML is supported: nested tables, arrays, multiline strings, inline comments. When both a flat key and a nested table key exist for the same field, the nested table takes precedence.

**Per-function config helpers**:
- `fn_cors(fn_name)`: returns per-function CORS or falls back to `api_cors`
- `fn_rate_limit(fn_name)`: returns per-function rate limit or falls back to `rate_limit`
- `fn_rate_window(fn_name)`: returns per-function rate window or falls back to `rate_window`
- `has_fn_rate_config(fn_name)`: whether the function has its own rate bucket

**Defaults**:
```
port = 3000
host = "127.0.0.1"
max_body = 1 MB
max_connections = 128
rate_limit = 60 req/60s
timeout = 5s (JS)
memory = 32 MB (JS)
fetch_timeout = 10s
CSP = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"
HSTS = "max-age=63072000; includeSubDomains"
X-Frame-Options = "DENY"
...
```

**`Config::security_headers()`**: generates the HTTP headers block. Strips `\r`, `\n`, `\0` characters to prevent header injection. `X-Content-Type-Options: nosniff` is always on.

### 3.3 builder.rs (2282 lines)

The framework's core. Two public functions:

#### `init()` — Scaffolding

Creates the full project structure:
- Directories: `pages/`, `components/`, `static/`, `functions/`, `content/`
- Layout: `layout.html` with `{{title}}` and `{{@site.title}}`
- Components: `Header.html`, `Footer.html`, `Card.html`, `Tag.html`
- Pages: `index.html` (bookmarks), `notes.html` (dynamic CRUD), `docs.html`
- Edge functions: `notes.js` (SQLite CRUD), `github.js` (external fetch)
- Content: `site.json`, `bookmarks.json`
- Config: `vanilo.toml` with randomly generated webhook_path
- Deploy: `Dockerfile`, `compose.yaml`, `.dockerignore`
- Static: `style.css`, `main.js` (embedded via `include_str!()`)

#### `build()` — Build pipeline

Sequential steps with atomic build:

1. **Preparation**: create `dist_tmp/`, clean up previous
2. **Loading**: components (`components/*.html`), layout (`layout.html`), content (`content/*.json`)
3. **Security lint**: `lint::check()` — blocks the build on errors
4. **Client runtime detection**: scans `static/**/*.js` for `Vanilo.` — only injects the runtime if used
5. **Build templates block**: generates `<template id="tpl-Name">` sorted alphabetically + runtime script
6. **Page processing** (recursive in `pages/`):
   - `extract_meta()`: extracts `<meta name="..." content="...">` as props, removes them from body
   - `content::expand_each()`: expands `<Each content="...">` loops
   - `content::resolve_placeholders()`: replaces `{{@path}}` (escaped) and `{{{@path}}}` (raw)
   - `replace_server_blocks()`: converts `<Server ... />` into `<!--vanilo:server ...-->` HTML markers
   - `parser::resolve()`: resolves component tags (recursive)
   - `component::render_layout()`: inserts content into layout before `</body>`
   - Second pass `resolve_placeholders()` for layout-level references
   - `inject_before_body_close()`: injects templates + JS runtime
   - Clean URLs: `about.html` → `about/index.html`
   - `minify_html()`: HTML minification (collapse whitespace, strip comments, preserve `<pre>/<code>/<script>/<style>/<template>`)
7. **Copy static**: recursive, filters blocked extensions (`.db`, `.env`, `.key`, `.pem`, `.sh`, `.sql`, `.log`, etc.)
8. **CSS tree-shaking**: inline used CSS per page, remove original CSS files
9. **JS minification** (optional, `minify_js = true`): strip comments, trim whitespace, join safe lines
10. **Asset hashing**: `style.css` → `style.a1b2c3d4.css` (FNV-1a 32-bit, 8 hex chars), rewrite HTML references
11. **Pre-compression**: gzip (best) + brotli (quality 11) for files > 256 bytes
12. **Atomic swap**: `dist_tmp` → `dist` with recovery on failure

**HTML minification**: preserves content of `<pre>`, `<code>`, `<script>`, `<style>`, `<textarea>`, `<template>`. Preserves `<!--vanilo:server ...-->` comments for runtime SSR. Strips all other HTML comments.

**JS minification**: `strip_js_comments()` respects string literals (`"`, `'`, `` ` ``). `js_line_continues()` determines when to join lines (after `{`, `(`, `[`, `=`, `,`, `?`, `:`, operators) but not after `++`/`--`.

**Content hash**: FNV-1a truncated to 32 bits, 8 hex characters. Deterministic by content (not filename). Hashed extensions: `.css`, `.js`.

**Pre-compression**: compressible extensions: `.html`, `.css`, `.js`, `.json`, `.svg`, `.xml`, `.txt`. Minimum threshold: 256 bytes. Does not re-compress existing `.gz`/`.br` files. Detects real extensions through hashes (`style.a1b2c3d4.css` → real extension `css`).

### 3.4 parser.rs (249 lines)

Recursive component tag resolution in HTML.

**`resolve(html, components)`**: walks the HTML, detects known PascalCase tags, replaces them with the rendered template.

**Detection**: `find_component_tag()` looks for `<` followed by an uppercase letter. Checks that the name is a known component. Parses attributes with `parse_attributes()`.

**Supported formats**:
```html
<Card title="Hello" />                    <!-- self-closing -->
<Card><p>Children content</p></Card>      <!-- block with children -->
<Card title="Hello"><Tag label="new" /></Card>  <!-- nested -->
```

**Nested resolution**:
1. Children content is resolved recursively first
2. The rendered template is also resolved recursively
3. Handles arbitrary depth of components within components

**`parse_attributes()`**: supports quoted (`"`, `'`), unquoted, and boolean (no value) attributes.

**`find_matching_close()`**: handles same-name nested tags with a depth counter.

### 3.5 component.rs (287 lines)

**`Component` struct**: contains only `template: String`.

**`load_components(dir)`**: loads all `.html` files from the `components/` directory. The filename (without extension) becomes the component name.

**`render(template, props, children)`**:
1. Replaces `{{{prop}}}` with raw (unescaped) value — BEFORE double braces
2. Replaces `{{prop}}` with HTML-escaped value
3. Cleans up unresolved placeholders (silently removed)
4. Inserts children before the root element's closing tag

**`render_layout(template, props, children)`**: same logic but inserts children before `</body>` instead of the root tag.

**`escape_html()`**: escapes `&`, `<`, `>`, `"`, `'` (→ `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#x27;`).

**`clean_placeholders()`**: removes unresolved `{{...}}` and `{{{...}}}`.

**`insert_children()`**: detects the root tag (`find_root_tag()`), finds the LAST occurrence of its closing tag, inserts children just before it.

### 3.6 content.rs (972 lines)

#### JSON parser

Full recursive descent JSON parser. Supports: strings (with Unicode escapes `\uXXXX`), numbers (integers, decimals, scientific notation), booleans, null, arrays, objects.

**`enum Value`**: `Null`, `Bool(bool)`, `Number(f64)`, `Str(String)`, `Array(Vec<Value>)`, `Object(Vec<(String, Value)>)`.

**`Value::get(path)`**: dot-separated path navigation. `"contact.email"` navigates Object → Object. Numeric indices navigate Arrays. E.g.: `"items.0.name"`.

**`Value::as_str()`**: converts to display string for template interpolation. Integers display without decimals. Objects/arrays are serialized to JSON.

#### Content CMS

**`load(dir)`**: loads all `*.json` from `content/`. The filename (without `.json`) becomes the key. E.g.: `content/site.json` → key `"site"`.

**`resolve_ref(content, path)`**: resolves a reference like `"site.contact.email"`. The first segment is the filename, the rest is the path within the value.

**`resolve_placeholders(html, content)`**:
- `{{@site.title}}` → HTML-escaped value (safe default)
- `{{{@site.title}}}` → raw value (opt-in for trusted HTML)
- Unresolved references → empty string
- Preserves `{{prop}}` (without `@`) for component resolution

**`expand_each(html, content)`**:
```html
<Each content="posts">
    <Card title="{{title}}" />
</Each>
```
- Iterates over the array referenced by `content="path"`
- For each object, replaces `{{key}}` (escaped) and `{{{key}}}` (raw) in the inner template
- Placeholders not matched by the current object are preserved (for later component resolution)
- Supports nested paths: `content="site.team"`

### 3.7 css.rs (876 lines)

Build-time CSS tree-shaking.

**`tree_shake(css, html)`**:
1. Extracts the safelist from `/* vanilo:keep .cls #id tag */` comments
2. Strips CSS comments
3. Parses CSS blocks (rules, at-rules, @media/@supports)
4. Extracts HTML tokens (tags, classes, IDs)
5. Merges safelist into tokens
6. Emits only rules whose selectors match the tokens
7. Compacts the CSS (collapse whitespace)

**CSS data model**:
- `Block::Rule { selector, body }` — standard rule
- `Block::AtRule { header, body }` — `@font-face`, `@keyframes`, `@charset` (always preserved)
- `Block::Media { query, children }` — `@media`, `@supports` (preserved if at least one child matches)

**Selector matching**:
- Tags: `div`, `p`, `main`
- Classes: `.card`, `.hero`
- IDs: `#main`
- Universal: `*`, `*::before` (always match)
- `:root` (always match)
- Combinators: descendant (space), child (`>`), adjacent sibling (`+`), general sibling (`~`)
- Pseudo-classes: `:hover`, `:first-of-type`, `:nth-child(...)` (ignored — the base selector matches if the tag/class/id exists)
- Pseudo-elements: `::before`, `::after` (ignored)
- Grouped selectors (`a, span`): each group is evaluated independently, only matching groups are emitted
- Combinations: `div.card:hover`, `.hero h1`, `.content p:first-of-type`

**Safelist**: `/* vanilo:keep .open .modal #overlay dialog */`
- Classes (`.prefix`) → added to detected classes
- IDs (`#prefix`) → added to detected IDs
- Tags (no prefix) → added to detected tags
- Multiple entries in a single comment
- Multiple safelist comments supported

**HTML analysis**: `extract_html_tokens()` scans opening tags, extracts tag names, `class=""` values (split by whitespace), and `id=""` values. Word-boundary check for `class=` (avoids false positives with `myclass=`).

### 3.8 functions.rs

JavaScript/TypeScript edge function execution runtime.

#### Resolution

**`resolve_function(api_path)`**: resolves API paths to function files. Resolution order:
1. `functions/{name}.js`
2. `functions/{name}.ts`
3. `functions/{name}/index.js`
4. `functions/{name}/index.ts`

When both `.js` and `.ts` exist, `.js` takes priority. Supports subdirectories: `/api/users/list` → `functions/users/list.js` (or `.ts`). Blocks path traversal (`..`).

#### Execution

**`execute(file_path, req, config)`**: if the file is `.ts`, it is first transpiled to JavaScript via `typescript::strip_types()` (oxc parser + transformer) before execution.

1. **QuickJS runtime creation**:
   - Memory limit: `config.memory` (default 32 MB)
   - Max stack size: 1 MB
   - Timeout: interrupt handler based on `config.timeout`

2. **Global injection**:
   - `db.query(sql, params?)` → returns an array of JSON objects
   - `db.exec(sql, params?)` → returns `{changes: N}`
   - `fetch(url, opts?)` → returns `{status, body}`

3. **JS wrapper**: the source code is wrapped in a script that:
   - Creates the `db` object with JS methods that call Rust functions
   - Creates the global `fetch()` function
   - Creates the `__req` object with `method`, `path`, `body`, `query`, `headers`
   - Executes `handler(__req)` and serializes the result to JSON
   - Catches errors (returns status 400 with server-side log)

4. **Request headers**: HTTP headers are passed to functions via `req.headers`. Sensitive headers are filtered out: `host`, `content-length`, `transfer-encoding`, `connection`, `upgrade`, `keep-alive`, `te`, `trailer`, `accept-encoding`. Names are normalized to lowercase.

#### SQLite

**Lazy opening**: connection is only opened on first `db.*` call. File: `data.db`.

**Pragmas**: `journal_mode=WAL`, `max_page_count=65536`.

**SQLite authorizer** — operation whitelist:
- **Allowed**: `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `DROP TABLE`, `CREATE INDEX`, `DROP INDEX`, temp tables, transactions, savepoints, recursive CTEs
- **Allowed pragmas**: `table_info`, `index_list`, `foreign_keys`, `busy_timeout`
- **Functions**: all except `load_extension`
- **Blocked**: `ATTACH`, `DETACH`, `CREATE TRIGGER`, `DROP TRIGGER`, `CREATE VIEW`, `DROP VIEW`, `ALTER TABLE`, `VACUUM`, `REINDEX`, `ANALYZE`

**Additional SQL blocking**: `is_blocked_sql()` detects `VACUUM` even through SQL comments (`-- bypass\nVACUUM`, `/* bypass */VACUUM`).

**Parameters**: `parse_json_params()` parses the JSON parameter array. Supported types: string, integer (i64), float (f64), null, boolean (true→1, false→0).

#### fetch()

**SSRF protection** (Server-Side Request Forgery):

1. **Scheme**: only `http://` and `https://`
2. **Hostname blocking**:
   - `localhost`, `*.local`, `*.internal`
   - `metadata.google.internal` (cloud metadata)
   - `169.254.169.254` (AWS/cloud metadata)
3. **Private IP blocking**:
   - Loopback: `127.0.0.0/8`, `::1`
   - RFC 1918: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
   - Link-local: `169.254.0.0/16`, `fe80::/10`
   - CGNAT: `100.64.0.0/10`
   - Unspecified: `0.0.0.0`, `::`
   - IPv6 unique local: `fc00::/7`
   - IPv6-mapped IPv4: `::ffff:x.x.x.x` (re-checks the underlying IPv4)
   - IPv4-compatible: `::x.x.x.x`
4. **Numeric IP blocking**: decimal (`2130706433`), hex (`0x7f000001`), octal (`0177.0.0.1`)
5. **DNS pre-resolution**: `check_resolved_ips()` resolves the hostname BEFORE the request, validates resolved IPs, and returns a validated IP
6. **Anti DNS-rebinding**: for HTTP (not HTTPS), the URL is rewritten with the resolved IP + original `Host:` header. HTTPS is protected by TLS certificate validation (SNI mismatch = handshake failure)
7. **Blocked outbound headers**: `Host`, `Transfer-Encoding`, `Content-Length`, `Connection`, `Upgrade`, `Proxy-Authorization`, `TE`, `Trailer`
8. **Limits**: configurable timeout, max response 10 MB, no redirects (`max_redirects = 0`)

**Supported methods**: GET, HEAD, DELETE, POST, PUT, PATCH.

#### JSON helpers

The module includes a mini JSON parser to deserialize QuickJS responses: `find_value_for_key()` (structured navigation), `extract_json_string()`, `extract_json_number()`, `extract_json_object()`. Correctly handles quoted strings (does not match keys inside string values).

### 3.9 lint.rs (780 lines)

Build-time security analysis.

**`check(pages_dir, components_dir, functions_dir)`**: returns `Err` (blocks the build) if errors are found. Warnings are printed but don't block.

#### HTML rules (pages + components)

| Rule | Severity | Detection |
|------|----------|-----------|
| `{{prop}}` in event handler (`onclick`, `onload`, etc.) | Error | Looks for `on*=` with `{{...}}` in the value |
| `{{prop}}` in `javascript:` URI | Error | `javascript:` + `{{` on the same line |
| `{{prop}}` in `<script>` (inline) | Error | `<script>` tag with `{{...}}` |
| `{{prop}}` in `<script>` (multi-line block) | Error | Scans between `<script>` and `</script>` |
| `{{prop}}` in `style=""` | Error | `style=` with `{{...}}` in the value |
| `<img>` without `alt` | Warn | Accessibility + Lighthouse |
| `<a target="_blank">` without `rel="noopener"` | Warn | Security (reverse tabnabbing) |

#### JS rules (edge functions)

| Rule | Severity | Detection |
|------|----------|-----------|
| SQL string concatenation | Error | `db.query("..." + var)` or `` db.exec(`...${var}`) `` in the first SQL argument (ignores params array) |
| `eval()` | Error | Global `eval(` call (not `.eval(` or `someeval(`) |
| `new Function()` | Error | Dynamic code construction |
| `document.write()` / `document.writeln()` | Error | XSS sink |
| `setTimeout("string", ...)` | Error | Equivalent to eval |
| `innerHTML = dynamic` | Warn | Not flagged for pure string literals |
| `outerHTML = dynamic` | Warn | XSS sink |
| `req.body`/`req.query` concatenated into response | Warn | `"..." + req.body` or `` `...${req.body}` `` |

**SQL subtlety**: `find_params_separator()` parses quoted strings so it only checks for concatenation in the first SQL argument, not in the parameters array. `db.query("SELECT ?", ["%" + q + "%"])` is **safe**.

### 3.10 server.rs (1738 lines)

Minimal multi-threaded HTTP server.

#### Main loop

`serve(config)`:
1. Binds TCP on `host:port`
2. Accepts connections in a loop
3. Checks `max_connections` (503 if exceeded)
4. Spawns one thread per connection with `ConnectionGuard` (atomic decrement on drop, even on panic)
5. Loads components once (Arc shared) for runtime SSR
6. Shares the SSR cache (Arc<Mutex<ServerCache>>)

#### HTTP parsing

- Reads headers until `\r\n\r\n` (max 64 KB)
- Parses Content-Length + Content-Type
- Reads body (bounded by `max_body`)
- Rejects `Transfer-Encoding` (no chunked, prevents request smuggling)
- URL percent-decodes paths
- Path traversal protection: blocks `..` and `\0` in the decoded path

#### Routing

1. **Webhook**: if `webhook_path` + `webhook_secret` configured and path matches → HMAC-SHA256 verification, background rebuild
2. **API** (`/api/*`):
   - `OPTIONS`: CORS preflight (responds 204, no rate limiting)
   - Per-IP rate limiting (sliding window)
   - Body size check
   - Content-Type enforcement (`application/json` required for POST/PUT/PATCH with body)
   - `HEAD` treated as `GET` (body suppressed in response)
   - JS function execution
   - On-the-fly compression (brotli quality 4 or gzip fast)
   - CORS headers if configured
3. **Static files**:
   - Checks if the HTML page contains `<!--vanilo:server ...-->` markers (SSR)
   - If SSR: executes server blocks, compresses on-the-fly, `Cache-Control: no-cache`
   - If static: serves pre-compressed variants (`.br`, `.gz`) if accepted
   - Clean URLs: `/about` → `dist/about/index.html`
   - ETag (FNV-1a 64-bit) + `304 Not Modified`
   - Cache-Control: HTML = `no-cache`, hashed files = `immutable, 1 year`, others = `1 day`

#### CORS

Configured via `api_cors` in `vanilo.toml` (global) or `[functions.<name>]` (per-function):
- Empty (default): no CORS headers = same-origin only
- `"*"`: `Access-Control-Allow-Origin: *` (no `Vary: Origin`)
- Specific: `"https://example.com"` or multiple `"https://a.com, https://b.com"`
- Checks the request Origin against the allowed list (case-insensitive)
- Responds with the specific origin + `Vary: Origin` (for caching)
- Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS
- Allowed headers: Content-Type, Authorization, X-Requested-With
- `Access-Control-Max-Age: 86400` (24h)
- Per-function override: `[functions.i] cors = "*"` overrides the global `api_cors` for `/api/i`

#### Rate limiting

- Per-IP on `/api/*`
- Sliding window: `rate_limit` requests per `rate_window` seconds
- Per-function override: `[functions.i] rate_limit = 30` creates a separate rate bucket for `/api/i` (keyed by `ip:fn_name`). Functions without overrides share the global bucket (keyed by `ip`)
- Behind a reverse proxy: if `trusted_proxy` matches the peer IP, uses `X-Forwarded-For` (first IP = client)
- Automatic pruning of stale entries when the map exceeds 100 entries (uses max window across all configs)
- Separate rate limit for webhook (default 5/min)

#### Compression

**Static files** (build time): brotli quality 11 + gzip best. Served directly from `.br`/`.gz` files.

**API responses** (runtime): brotli quality 4 + gzip fast. Compressible types: `text/*`, `application/javascript`, `application/json`, `image/svg+xml`, `*xml`. Threshold: > 256 bytes.

**Vary header**: combines `Origin` (CORS) + `Accept-Encoding` (compression) when both apply.

#### Content types

16 MIME types supported: HTML, CSS, JS, JSON, PNG, JPEG, SVG, GIF, ICO, WOFF2, WOFF, TTF, WebP, MP4, WebM + `application/octet-stream` fallback.

Clean URLs (no extension) resolve to `text/html`.

#### Server cache (SSR)

`ServerCache`: in-memory HashMap for server blocks with TTL.
- Max capacity: 1024 entries
- Eviction: oldest entry when full
- Lazy expiration: checked on `get()`
- Key: `"{function}:{component}:{params}"`

---

## 4. CLI and commands

```bash
vanilo init              # Scaffold a new project
vanilo build             # Generate dist/
vanilo serve [port]      # Build + start dev server
```

`vanilo serve` runs `build()`, spawns a file watcher thread, then `serve()`. The watcher monitors `pages/`, `components/`, `static/`, `functions/`, `content/`, `layout.html`, and `vanilo.toml` for changes. On change, it debounces (300ms), acquires the `build_lock`, and calls `builder::build()`. The server keeps serving the old `dist/` during rebuild — the atomic swap makes the transition seamless. If the lock is already held (webhook rebuild in progress), the watcher skips.

---

## 5. Project structure

```
my-site/
  vanilo.toml          # Configuration
  layout.html          # Global layout (optional)
  pages/               # HTML pages (one page = one route)
    index.html         # → /
    about.html         # → /about
    blog/
      index.html       # → /blog
      post.html        # → /blog/post
  components/          # Reusable components (PascalCase)
    Header.html
    Card.html
  content/             # JSON data (CMS)
    site.json
    posts.json
  static/              # Static files (copied as-is)
    style.css
    main.js
    images/
  functions/           # Edge functions (JS/TS)
    hello.ts           # → /api/hello
    users/
      list.js          # → /api/users/list
  dist/                # Generated output (do not edit)
  data.db              # SQLite (created on first db.* call)
```

**Clean URLs**: `pages/about.html` → `dist/about/index.html` → served as `/about`. `pages/index.html` stays as `dist/index.html`.

---

## 6. Configuration (vanilo.toml)

```toml
# Network
port = 3000
host = "127.0.0.1"

# Server limits
max_body = 1            # MB (default: 1)
max_connections = 128
rate_limit = 60         # requests per window on /api/*
rate_window = 60        # seconds

# JS runtime
timeout = 5             # seconds (JS execution)
memory = 32             # MB (QuickJS runtime)
fetch_timeout = 10      # seconds (outbound HTTP)

# Proxy
# trusted_proxy = "127.0.0.1"   # or "*" to trust any peer

# Build
# minify_js = true               # Opt-in, doesn't handle regex literals

# CORS for /api/*
# api_cors = ""                  # Empty = same-origin (default)
# api_cors = "*"                 # Allow all
# api_cors = "https://a.com, https://b.com"   # Specific

# Per-function overrides: [functions.<name>] for /api/<name>
# Overrides CORS and/or rate limiting for a specific function.
# Omitted fields fall back to the global config.
# [functions.i]
# cors = "*"                     # Override CORS for /api/i
# rate_limit = 30                # Own rate bucket: 30 req/60s
# rate_window = 60

[security_headers]
content_security_policy = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"
# strict_transport_security = "max-age=63072000; includeSubDomains"
# x_frame_options = "DENY"
# referrer_policy = "strict-origin-when-cross-origin"
# permissions_policy = "camera=(), microphone=(), geolocation=()"
# cross_origin_opener_policy = "same-origin"
# cross_origin_resource_policy = "same-origin"

[webhook]
# webhook_path = "/_hook/a3f8c91e..."   # Unique per project
# webhook_secret = "your-secret"
# webhook_rate_limit = 5
# webhook_rate_window = 60
```

To disable a header: set it to empty string (`""`).

Environment variables `PORT` and `HOST` override `vanilo.toml`.

---

## 7. Component system

### Definition

An `.html` file in `components/`, named in PascalCase:

```html
<!-- components/Card.html -->
<div class="card">
    <h3>{{title}}</h3>
    <p>{{description}}</p>
</div>
```

### Usage

```html
<!-- Self-closing -->
<Card title="Hello" description="World" />

<!-- With children -->
<Card title="Hello">
    <Tag label="new" />
    <p>More content</p>
</Card>
```

### Props

- `{{prop}}`: HTML-escaped (safe default). `<script>` → `&lt;script&gt;`
- `{{{prop}}}`: raw, unescaped (opt-in for trusted HTML)
- Unresolved props: silently removed
- Standard HTML attributes: double quotes, single quotes, or unquoted
- Boolean attributes supported (no value)

### Children (slot)

Children are inserted before the root element's closing tag:

```html
<!-- Template: <div class="card"></div> -->
<!-- Children: <p>Hello</p> -->
<!-- Result: <div class="card"><p>Hello</p></div> -->
```

### Nesting

Components can contain other components. Resolution is recursive:

```html
<Card title="Hello">
    <Tag label="new" />     <!-- component inside a component -->
</Card>
```

### Layout

`layout.html` is a special component. Props come from the pages' `<meta>` tags. Page content is inserted before `</body>`.

```html
<!-- layout.html -->
<!DOCTYPE html>
<html>
<head><title>{{title}}</title></head>
<body>
<!-- page content is inserted here -->
</body>
</html>
```

```html
<!-- pages/about.html -->
<meta name="title" content="About">
<h1>About</h1>
```

---

## 8. Template system

### Placeholders

| Syntax | Context | Escaping |
|--------|---------|----------|
| `{{prop}}` | Component props | HTML-escaped |
| `{{{prop}}}` | Component props | Raw |
| `{{@site.title}}` | Content CMS | HTML-escaped |
| `{{{@site.title}}}` | Content CMS | Raw |
| `{{title}}` in `<Each>` | Current item | HTML-escaped |
| `{{{html}}}` in `<Each>` | Current item | Raw |

### Resolution order

1. `<Each content="...">`: loop expansion (content CMS)
2. `{{@...}}`: content placeholder resolution (body)
3. `<Server ... />`: conversion to SSR markers
4. `<Component ... />`: component resolution (recursive)
5. Layout wrapping
6. `{{@...}}`: content placeholder resolution (layout, second pass)
7. Client runtime injection (templates + script)

### Meta tags as props

`<meta name="..." content="...">` tags at the top of a page are extracted as props and passed to the layout:

```html
<!-- pages/about.html -->
<meta name="title" content="About Us">
<meta name="description" content="Our story">

<h1>About</h1>
```

The layout receives `{{title}}` = "About Us" and `{{description}}` = "Our story".

---

## 9. Content CMS (JSON)

### Files

Each `content/*.json` file is accessible by its name (without extension):

```json
// content/site.json
{
    "title": "My Site",
    "contact": { "email": "hello@example.com" }
}
```

```html
<h1>{{@site.title}}</h1>
<a href="mailto:{{@site.contact.email}}">Contact</a>
```

### Dot-path navigation

`{{@site.contact.email}}`:
1. `site` → file key for `content/site.json`
2. `contact` → object key
3. `email` → final value

Numeric indices navigate arrays: `{{@posts.0.title}}`.

### Loops with `<Each>`

```html
<Each content="posts">
    <Card title="{{title}}" />
</Each>
```

- Iterates over each object in the `content/posts.json` array
- `{{key}}` is replaced with the HTML-escaped value from the current object
- `{{{key}}}` for the raw value
- Unmatched placeholders from the current object are preserved (passed to components)

### Value types

| JSON type | Template display |
|-----------|-----------------|
| String | As-is |
| Number (integer) | No decimal (`3000` not `3000.0`) |
| Number (decimal) | With decimal (`3.14`) |
| Boolean | `"true"` / `"false"` |
| Null | Empty string |
| Object/Array | Serialized as JSON |

---

## 10. Edge functions (JS/TS)

### Convention

`functions/hello.js` → `/api/hello`
`functions/hello.ts` → `/api/hello`
`functions/users/list.js` → `/api/users/list`

Both `.js` and `.ts` files are supported. When both exist for the same route, `.js` takes priority. TypeScript files are transpiled at runtime via oxc (parser → semantic analysis → transformer → codegen). Type annotations, interfaces, type aliases, enums, and generics are stripped to produce valid JavaScript.

### Interface

```typescript
function handler(req) {
    // req.method  : "GET", "POST", etc.
    // req.path    : "/api/hello"
    // req.body    : request body (string)
    // req.query   : query string without the "?"
    // req.headers : {name: value} object (lowercase names, sensitive headers filtered)

    return {
        status: 200,              // optional (default: 200)
        headers: {                // optional
            "content-type": "application/json",
            "location": "/other"  // custom headers are forwarded
        },
        body: "response"          // string or object (auto JSON.stringify)
    };
}
```

### SQLite

```javascript
// Create a table
db.exec("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)");

// Insert with parameters
db.exec("INSERT INTO users (name) VALUES (?)", ["Alice"]);

// Query with parameters
var rows = db.query("SELECT * FROM users WHERE name LIKE ?", ["%" + search + "%"]);
// → [{id: 1, name: "Alice"}, ...]

// Exec result
var result = db.exec("DELETE FROM users WHERE id = ?", [1]);
// → {changes: 1}
```

Parameterized queries (`?`) are **required**. SQL concatenation is blocked by the linter.

### Outbound fetch()

```javascript
var res = fetch("https://api.example.com/data", {
    method: "POST",
    headers: { "Authorization": "Bearer token" },
    body: JSON.stringify({ key: "value" })
});
// res.status : HTTP status code
// res.body   : response body (string)
```

### JS sandbox

- Limited memory (default 32 MB)
- Execution timeout (default 5s)
- Max stack 1 MB
- No filesystem access
- No `require()` / `import`
- TypeScript supported (transpiled via oxc)
- fetch() with full SSRF protection
- SQLite with restrictive authorizer

---

## 11. Client-side runtime (Vanilo.*)

### Conditional injection

The runtime is only injected if a JS file in `static/` contains `Vanilo.`. This avoids bloating sites that don't use client-side rendering.

### Templates

Each component is emitted as `<template id="tpl-Name">` before `</body>`. Order is alphabetical (deterministic).

### API

```javascript
// Render a list of components into a container
Vanilo.list("#results", "Card", [
    { title: "First", description: "Hello" },
    { title: "Second", description: "World" }
]);

// Render a single component
Vanilo.put("#profile", "UserCard", user);

// Get HTML string without inserting into the DOM
var html = Vanilo.render("Card", { title: "Preview" });

// HTML escaping
var safe = Vanilo.esc("<script>");  // → "&lt;script&gt;"
```

### Escaping

Props are escaped via DOM (`textContent`) in the client-side runtime. This is automatic for `Vanilo.render()`, `Vanilo.put()`, and `Vanilo.list()`.

### Runtime (minified)

```javascript
(function(){
var S=window.Vanilo={};
S.esc=function(s){var d=document.createElement("div");d.appendChild(document.createTextNode(s));return d.innerHTML};
S.render=function(n,p){var t=document.getElementById("tpl-"+n);if(!t)return"";var h=t.innerHTML;if(p){var k=Object.keys(p);for(var i=0;i<k.length;i++){var v=p[k[i]]!=null?String(p[k[i]]):"";h=h.split("{{"+k[i]+"}}").join(S.esc(v))}}return h.replace(/\{\{[^}]+\}\}/g,"")};
S.put=function(sel,n,p){var el=typeof sel==="string"?document.querySelector(sel):sel;if(el)el.innerHTML=S.render(n,p)};
S.list=function(sel,n,arr){var el=typeof sel==="string"?document.querySelector(sel):sel;if(!el||!arr)return;el.innerHTML=arr.map(function(p){return S.render(n,p)}).join("")};
})();
```

---

## 12. Server blocks (partial SSR)

### Concept

Partial SSR lets you render dynamic content (edge function + component) at HTTP request time, directly within a static HTML page. Useful for SEO: content is in the initial HTML.

### Syntax (build time)

```html
<Server function="articles" component="Card" />
<Server function="articles" component="Card" cache="3600" />
<Server function="articles" component="Card" category="rust" limit="10" />
```

**Reserved attributes**:
- `function`: edge function name (in `functions/`)
- `component`: component name for rendering
- `cache`: TTL in seconds (optional)

**Custom attributes**: everything else becomes query-string parameters passed to the function (`params="category=rust&limit=10"`, sorted alphabetically).

### Build-time transformation

`<Server function="articles" component="Card" cache="3600" />`
→ `<!--vanilo:server fn="articles" comp="Card" cache="3600"-->`

The marker is preserved by HTML minification (`<!--vanilo:server` comments are not stripped).

### Runtime rendering

When the server serves an HTML page containing a marker:

1. Parses the marker (function, component, TTL, params)
2. Checks the cache (key = `"fn:comp:params"`)
3. If cache miss: executes the edge function with `method=GET`, `query=params`
4. Parses the JSON response
5. If array: renders the component for each object
6. If single object: renders the component once
7. Resolves nested components in the output
8. Stores in cache if TTL is set
9. Replaces the marker with the rendered HTML

### Cache

- In-memory, HashMap with TTL
- Max 1024 entries
- Eviction: oldest when full
- Lazy expiration on `get()`
- Pages with server blocks: `Cache-Control: no-cache`
- ETag recomputed after assembly

---

## 13. CSS tree-shaking

### How it works

At build time, each HTML page gets its CSS inlined: only rules used by that page's static HTML are included.

1. Detects `<link rel="stylesheet" href="...">` in the HTML
2. Parses the referenced CSS file
3. Scans the HTML to extract tags, classes, IDs
4. Keeps only rules whose selectors match
5. Replaces the `<link>` with an inline `<style>`
6. Removes the original CSS file

### Safelist

For classes added dynamically by JavaScript:

```css
/* vanilo:keep .open .modal .active */
```

Tags, classes (`.prefix`), and IDs (`#prefix`) are supported. Multiple entries per comment, multiple safelist comments per file.

### Automatically preserved

- `@font-face`: always preserved
- `@keyframes`: always preserved
- `@charset`: always preserved
- `:root`: always preserved
- `*` / `*::before` / `*::after`: always preserved
- `@media` / `@supports`: preserved if at least one child rule matches

### Grouped selectors

```css
a, span { color: red; }
```

If only `<a>` is present in the HTML, the emitted selector is `a { color: red; }` — `span` is stripped.

---

## 14. Build pipeline

Full pipeline summary in order:

```
1.  Create dist_tmp/
2.  Load components/*.html
3.  Load layout.html (optional)
4.  Load content/*.json
5.  Security lint (pages + components + functions)
    → Blocks the build on errors
6.  Detect Vanilo.* usage in static/**/*.js
7.  Build templates block + runtime (if used)
8.  For each page in pages/ (recursive):
    a. Extract <meta> as props
    b. Expand <Each content="...">
    c. Resolve {{@...}} placeholders
    d. Replace <Server .../> with <!--vanilo:server--> markers
    e. Resolve components (recursive)
    f. Wrap in layout
    g. Second pass {{@...}} for layout
    h. Inject templates + runtime before </body>
    i. Clean URLs (about.html → about/index.html)
    j. Minify HTML
9.  Copy static/ (filter blocked extensions)
10. CSS tree-shaking (inline per page)
11. JS minification (optional, minify_js = true)
12. Hash CSS/JS assets (style.a1b2c3d4.css)
13. Rewrite HTML references to hashed files
14. Pre-compress gzip + brotli (files > 256 bytes)
15. Atomic swap: dist_tmp → dist (with recovery)
```

---

## 15. HTTP server

### Architecture

- **One thread per connection** (no async, no pool)
- **`ConnectionGuard`**: RAII with `AtomicUsize` for the active connections counter
- **Rate limiting**: `HashMap<IP, (count, timestamp)>` behind `Mutex`
- **Build lock**: `Mutex<()>` to serialize webhook rebuilds

### Response headers

Every response includes:
- `Content-Type`, `Content-Length`
- Configurable security headers (CSP, HSTS, etc.)
- `X-Content-Type-Options: nosniff` (always on)
- `ETag` (for static files and SSR)
- `Cache-Control` (adapted to file type)
- `Content-Encoding` + `Vary: Accept-Encoding` (if compressed)
- CORS headers (if configured, for `/api/*` only)
- Custom headers from edge functions (e.g. `Location` for redirects). Server-managed headers (`Content-Type`, `Content-Length`, `Content-Encoding`, `Connection`, `Transfer-Encoding`, `Keep-Alive`) cannot be overridden by functions.

### Cache-Control

| Type | Policy |
|------|--------|
| HTML | `no-cache` (revalidate with ETag) |
| Hashed file (`*.a1b2c3d4.css`) | `public, max-age=31536000, immutable` |
| Other static | `public, max-age=86400` |
| API | `no-store` |
| SSR pages | `no-cache` |

### Blocked extensions

Never served or copied to dist: `.db`, `.sqlite`, `.sqlite3`, `.env`, `.env.*`, `.key`, `.pem`, `.p12`, `.pfx`, `.sh`, `.bash`, `.sql`, `.log`, `.envrc`, `.htaccess`, `.bak`, `.swp`, `.swo`, `.DS_Store`, `.gitignore`, `.gitmodules`.

---

## 16. Security

### Defense in depth

| Layer | Protection |
|-------|-----------|
| **Build (lint)** | Detects XSS in templates, SQL injection in functions, eval(), document.write() |
| **Build (escaping)** | `{{prop}}` and `{{@content}}` are HTML-escaped by default |
| **Server (path)** | Blocks `..`, `\0`, symlinks outside dist/, sensitive extensions |
| **Server (headers)** | CSP, HSTS, X-Frame-Options, COOP, CORP, nosniff, Referrer-Policy, Permissions-Policy |
| **Server (rate limit)** | Per-IP on /api/* (global or per-function buckets), dedicated for webhook |
| **Server (body)** | Configurable max size, Content-Type enforced for body-bearing methods |
| **Server (CORS)** | Opt-in, Origin verification, preflight handling |
| **Runtime (JS)** | Sandboxed QuickJS: memory, timeout, stack |
| **Runtime (SQL)** | Restrictive SQLite authorizer, no ATTACH, no triggers, no load_extension |
| **Runtime (fetch)** | Full SSRF protection: scheme, hostname, IP, DNS pre-resolution, anti-rebinding |
| **Runtime (headers)** | Sensitive header filtering (inbound and outbound), CRLF/null sanitization |

### Header injection prevention

Security header values and content-types returned by functions are filtered: `\r`, `\n`, `\0` characters are removed.

### Webhook security

- Endpoint does not exist if path + secret are not configured (404)
- Random path per project (cannot be guessed)
- HMAC-SHA256 verification (constant-time via `hmac` crate)
- Dedicated rate limit (5/min default)
- Concurrent rebuilds are deduplicated (try_lock)

---

## 17. Performance

### Build time

- **HTML minification**: collapse whitespace, strip comments, preserve `<pre>/<code>/<script>/<style>/<template>`
- **JS minification** (opt-in): strip comments, trim indentation, join lines when safe
- **CSS tree-shaking**: inline only rules used by each page
- **Content hashing**: FNV-1a 32-bit for cache busting
- **Pre-compression**: gzip (best) + brotli (quality 11) at build time

### Runtime

- **Pre-compressed serving**: serves `.br`/`.gz` directly (zero CPU)
- **On-the-fly compression**: brotli quality 4 / gzip fast for API responses
- **ETag**: FNV-1a 64-bit, 304 Not Modified
- **Immutable cache**: hashed files cached for 1 year by the browser
- **SSR cache**: in-memory with TTL for server blocks
- **Compression threshold**: 256 bytes minimum (no overhead for small responses)

---

## 18. Webhook (auto-rebuild)

### Configuration

```toml
[webhook]
webhook_path = "/_hook/a3f8c91e..."   # Generated by vanilo init
webhook_secret = "your-secret"
webhook_rate_limit = 5                 # Max rebuilds per window
webhook_rate_window = 60               # Seconds
```

### Flow

1. Push to GitHub/Gitea → webhook POST to `/_hook/...`
2. Vanilo checks rate limit (dedicated, independent from API)
3. Verifies HMAC-SHA256 signature (`X-Hub-Signature-256: sha256=...`)
4. Responds `200 OK` immediately
5. Runs `builder::build()` in a background thread
6. If a rebuild is already in progress → skip (try_lock)
7. Atomic build guarantees zero downtime (swap dist_tmp → dist)

---

## 19. Deployment

### Docker

`vanilo init` generates a multi-stage `Dockerfile`:

```dockerfile
# Stage 1: build vanilo from source
FROM rust:1-bookworm AS toolchain
RUN apt-get update && apt-get install -y libclang-dev
WORKDIR /build
RUN cargo install vanilo

# Stage 2: build site + serve
FROM debian:bookworm-slim
RUN useradd -r -s /usr/sbin/nologin vanilo
COPY --from=toolchain /usr/local/cargo/bin/vanilo /usr/local/bin/vanilo
WORKDIR /app
COPY . .
RUN vanilo build
USER vanilo
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["vanilo", "serve"]
```

### compose.yaml

```yaml
services:
  app:
    build: .
    ports:
      - "127.0.0.1:3000:3000"
    # volumes:
    #   - ./data.db:/app/data.db    # Persist SQLite
    restart: unless-stopped
```

### Reverse proxy

Vanilo is designed to run behind Caddy or Nginx for HTTPS. Set `trusted_proxy` so rate limiting uses the real client IP:

```toml
trusted_proxy = "127.0.0.1"   # Reverse proxy IP
# or
trusted_proxy = "*"            # Trust any peer (less secure)
```

---

## 20. Rust dependencies

```toml
[dependencies]
brotli = "7"                                          # Brotli compression
flate2 = "1"                                          # Gzip compression
rquickjs = { version = "0.11.0", features = ["bindgen"] }  # QuickJS JS runtime
rusqlite = { version = "0.39.0", features = ["bundled", "hooks"] }  # SQLite
ureq = "3.3.0"                                        # HTTP client (for fetch())
hmac = "0.12"                                         # HMAC (webhook)
sha2 = "0.10"                                         # SHA-256 (webhook)
serde = { version = "1", features = ["derive"] }      # Serialization (config)
toml = "0.8"                                          # TOML parser (config)
notify = "7"                                          # File system watcher (hot reload)
oxc_allocator = "0.127"                               # OXC memory arena (TypeScript)
oxc_parser = "0.127"                                  # OXC parser (TypeScript)
oxc_codegen = "0.127"                                 # OXC code generator (TypeScript)
oxc_span = "0.127"                                    # OXC source spans (TypeScript)
oxc_transformer = "0.127"                             # OXC transformer (TypeScript)
oxc_semantic = "0.127"                                # OXC semantic analysis (TypeScript)
```

**All heavy dependencies are bundled**: SQLite is compiled into the binary (`bundled`), QuickJS is compiled via bindgen. The final binary is self-contained.

**libclang-dev** is required at build time for QuickJS bindgen.

---

## 21. Tests

The project contains exhaustive unit tests in every module:

| Module | Tests | Coverage |
|--------|-------|----------|
| `config.rs` | 24 tests | TOML parsing (flat, nested, merge, multiline), security headers, CORS, per-function config |
| `parser.rs` | 4 tests | Self-closing, block, nesting, mixed HTML |
| `component.rs` | 10 tests | Props, escaping, raw, children, layout |
| `builder.rs` | 32 tests | Meta extraction, hashing, compression, CSS inline, JS minification, templates, server blocks, HTML minification |
| `server.rs` | 31 tests | Cache-control, content hash, encoding, content type, compression, ETag, URL decode, blocked paths, CORS, headers, server markers, SSR cache |
| `functions.rs` | 22 tests | SSRF, private IPs, URL parsing, SQL blocking, params, JSON, headers, TS resolution |
| `css.rs` | 24 tests | Tree-shaking, selectors, safelist, media queries, grouped |
| `content.rs` | 24 tests | JSON parser, CMS, placeholders, `<Each>`, escaping |
| `lint.rs` | 15 tests | XSS, SQL injection, eval, innerHTML, integration |
| `typescript.rs` | 10 tests | Type stripping, interfaces, enums, generics, casts, error handling |
| `watcher.rs` | 4 tests | Debounce coalescing, separate batches, lock skip, watched dirs |

**Total: 317 unit tests.**

Run tests:
```bash
cargo test
```

---

## 22. Limits and non-goals

- **Single instance**: SQLite is local, rate limiting is per-process. No horizontal scaling.
- **Basic JS minification**: doesn't handle regex literals. Off by default.
- **Static CSS tree-shaking**: scans HTML at build time. JS-added classes need `/* vanilo:keep */`.
- **No browser live-reload**: `vanilo serve` rebuilds on file changes but does not inject a live-reload script.
- **No JS bundling**: no module system, no JS tree-shaking.
- **No incremental SSG**: every build regenerates everything.
- **HTTP/1.1 only**: the server does not support HTTP/2.
- **No async**: one thread per connection.
