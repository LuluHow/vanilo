# simple

A static site generator with edge functions, written in Rust.

Zero config, zero frontend dependencies. Write HTML, declare components, `simple build` generates your site. Add JavaScript edge functions, `simple serve` runs everything.

**scaffold &rarr; dev &rarr; deploy.**

## Installation

```bash
cargo install --path .
```

## Quick start

```bash
simple init        # scaffold a new project
simple serve       # build + dev server on http://127.0.0.1:3000
```

## Commands

| Command | Description |
|---------|-------------|
| `simple init` | Scaffold the project structure (pages, components, functions, Dockerfile, compose.yaml, simple.toml) |
| `simple build` | Generate the `dist/` directory from sources |
| `simple serve [port]` | Build + start the dev server (default: port 3000) |

## Project structure

```
my-site/
  simple.toml            # configuration
  layout.html            # global layout (optional)
  pages/                 # your pages
    index.html
    about.html
  components/            # reusable components
    Header.html
    Card.html
  content/               # JSON content (CMS)
    site.json
    posts.json
  static/                # copied as-is into dist/
    style.css
    main.js
  functions/             # edge functions (JS)
    hello.js
  dist/                  # build output (generated)
  data.db                # SQLite database (created on first db call)
  Dockerfile
  compose.yaml
  .dockerignore
```

## Components

A component is an HTML file inside `components/`. The filename must be PascalCase.

### Basic component

```html
<!-- components/Card.html -->
<div class="card">
</div>
```

```html
<!-- pages/index.html -->
<Card>
    <h1>Title</h1>
    <p>Content goes here</p>
</Card>
```

Output:

```html
<div class="card">
    <h1>Title</h1>
    <p>Content goes here</p>
</div>
```

Child content is automatically inserted into the root element of the component.

### Props

Pass values through attributes using `{{name}}` placeholders:

```html
<!-- components/Header.html -->
<header>
    <h1>{{title}}</h1>
</header>
```

```html
<Header title="My Site" />
```

Output:

```html
<header>
    <h1>My Site</h1>
</header>
```

### Self-closing tags

Components without children can be self-closed:

```html
<Header title="Home" />
```

### Nesting

Components can contain other components. Resolution is recursive.

### Client-side rendering

At build time, every component is also emitted as a native `<template>` element in the HTML. A minimal runtime (`Simple`) lets you reuse components from JavaScript — no markup duplication, no `innerHTML` string concatenation.

**Render a list:**

```javascript
fetch('/api/messages').then(r => r.json()).then(function(messages) {
    Simple.list('#messages-list', 'MessageCard', messages);
});
```

**Render a single item:**

```javascript
Simple.put('#user-profile', 'UserCard', user);
```

**Get the HTML string (advanced):**

```javascript
var html = Simple.render('MessageCard', { author: "Alice", content: "Hello" });
```

**API:**

| Method | Description |
|--------|-------------|
| `Simple.list(selector, component, array)` | Render an array of items into a container |
| `Simple.put(selector, component, data)` | Render a single item into a container |
| `Simple.render(component, data)` | Return the HTML string (no DOM insertion) |
| `Simple.esc(string)` | HTML-escape a string |

All prop values are automatically HTML-escaped. Unreplaced `{{prop}}` placeholders are removed.

**Conditions** are plain JavaScript:

```javascript
if (user) Simple.put('#welcome', 'Welcome', user);

Simple.put('#status', user.premium ? 'PremiumBadge' : 'FreeBadge', user);
```

## Layout

`layout.html` wraps all pages. Each page's content is inserted before `</body>`.

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>{{title}}</title>
    <link rel="stylesheet" href="/style.css">
</head>
<body>
</body>
</html>
```

`{{title}}` in the layout is replaced by the value of the matching `<meta>` tag in the page:

```html
<meta name="title" content="Home">

<p>My content</p>
```

Any `<meta name="...">` tag in a page becomes a prop available in the layout.

## Clean URLs

`about.html` is built as `about/index.html` and served at `/about`.

## Content (CMS)

JSON files in `content/` act as a lightweight CMS. Edit them directly on GitHub &mdash; push triggers a rebuild, site updates.

### Values

Create a JSON file:

```json
// content/site.json
{
    "title": "My Site",
    "description": "Built with simple",
    "contact": {
        "email": "hello@example.com"
    }
}
```

Reference values in any page or layout with `{{@file.key}}`:

```html
<h1>{{@site.title}}</h1>
<p>{{@site.description}}</p>
<a href="mailto:{{@site.contact.email}}">Contact</a>
```

Nested paths work: `{{@site.contact.email}}` navigates `site.json` &rarr; `contact` &rarr; `email`.

Content references also work as component props:

```html
<Header title="{{@site.title}}" />
```

Unresolved references are removed silently.

### Collections

Use `<Each>` to iterate over a JSON array:

```json
// content/posts.json
[
    { "title": "First post", "slug": "first-post", "description": "Hello world" },
    { "title": "Second post", "slug": "second-post", "description": "Another one" }
]
```

```html
<Each content="posts">
    <article>
        <h2><a href="/blog/{{slug}}">{{title}}</a></h2>
        <p>{{description}}</p>
    </article>
</Each>
```

Inside `<Each>`, `{{key}}` is replaced with each item's properties. Unmatched placeholders pass through to the component system, so you can combine both:

```html
<Each content="posts">
    <Card title="{{title}}" href="/blog/{{slug}}" />
</Each>
```

`<Each>` also supports nested paths for arrays inside objects:

```html
<Each content="site.team">
    <p>{{name}} — {{role}}</p>
</Each>
```

### Workflow

1. Edit `content/*.json` on GitHub (web UI, API, or local clone)
2. Push triggers your build pipeline (`simple build`)
3. Site updates with new content &mdash; no code changes needed

## Edge functions

JavaScript files in `functions/` are executed server-side via an embedded QuickJS runtime.

Routing is automatic:

| File | URL |
|------|-----|
| `functions/hello.js` | `/api/hello` |
| `functions/users/list.js` | `/api/users/list` |
| `functions/users/index.js` | `/api/users` |

### Request / Response

Each file must export a `handler(req)` function:

```javascript
function handler(req) {
    // req.method  — GET, POST, PUT, PATCH, DELETE, HEAD
    // req.path    — /api/hello
    // req.body    — request body (string)
    // req.query   — query string (e.g. "foo=bar")

    return {
        status: 200,                              // optional, defaults to 200
        headers: { "content-type": "text/plain" }, // optional
        body: "Hello!"                             // string or object (auto JSON.stringify)
    }
}
```

### Database (SQLite)

A global `db` object is available in every function. The database file (`data.db`) is created automatically on first use. WAL mode is enabled by default.

```javascript
function handler(req) {
    db.exec("CREATE TABLE IF NOT EXISTS visits (count INTEGER)");

    var row = db.query("SELECT count FROM visits");
    if (row.length === 0) {
        db.exec("INSERT INTO visits VALUES (1)");
    } else {
        db.exec("UPDATE visits SET count = count + 1");
    }

    var result = db.query("SELECT count FROM visits");
    return {
        body: { visits: result[0].count }
    }
}
```

**API:**

| Method | Returns | Description |
|--------|---------|-------------|
| `db.query(sql, params?)` | `Array<Object>` | Run a SELECT query, returns rows as objects |
| `db.exec(sql, params?)` | `{ changes: number }` | Run an INSERT/UPDATE/DELETE, returns affected row count |

**Parameterized queries** (prevents SQL injection):

```javascript
db.query("SELECT * FROM users WHERE id = ?", [userId]);
db.exec("INSERT INTO users (name) VALUES (?)", [name]);
```

### fetch()

Outbound HTTP calls from edge functions:

```javascript
function handler(req) {
    var resp = fetch("https://api.example.com/data", {
        method: "POST",
        body: JSON.stringify({ key: "value" }),
        headers: { "content-type": "application/json" }
    });

    return { body: resp.body }
}
```

`fetch()` returns `{ status, body }`.

## Configuration

All configuration lives in `simple.toml` at the project root. Every field is optional &mdash; defaults are applied when omitted.

### Override priority

Settings are resolved in this order (highest priority first):

| Priority | Source | Example |
|----------|--------|---------|
| 1 | CLI arguments | `simple serve 8080` |
| 2 | Environment variables | `PORT=8080`, `HOST=0.0.0.0` |
| 3 | `simple.toml` | `port = 8080` |
| 4 | Built-in defaults | `port = 3000`, `host = "127.0.0.1"` |

### Full reference

```toml
# ── Network ──────────────────────────────────────────────
port = 3000                 # server port
host = "127.0.0.1"          # bind address

# ── Server limits ────────────────────────────────────────
max_body = 1                # request body limit (MB)
max_connections = 128       # maximum concurrent connections
rate_limit = 60             # requests per window on /api/*
rate_window = 60            # rate limit window (seconds)

# ── JavaScript runtime ──────────────────────────────────
timeout = 5                 # JS execution timeout (seconds)
memory = 32                 # JS runtime memory limit (MB)
fetch_timeout = 10          # outbound HTTP timeout (seconds)

# ── Security headers ────────────────────────────────────
# All headers below can be overridden. Set to "" to disable.
[security_headers]
content_security_policy = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"
strict_transport_security = "max-age=63072000; includeSubDomains"
x_frame_options = "DENY"
referrer_policy = "strict-origin-when-cross-origin"
permissions_policy = "camera=(), microphone=(), geolocation=()"
cross_origin_opener_policy = "same-origin"
cross_origin_resource_policy = "same-origin"
# x_content_type_options is always "nosniff" and cannot be overridden.
```

### Configuration examples

**Allow Stripe in CSP:**

```toml
[security_headers]
content_security_policy = "default-src 'self'; script-src 'self' https://js.stripe.com; frame-src https://js.stripe.com"
```

**Disable HSTS in development:**

```toml
[security_headers]
strict_transport_security = ""
```

**Allow iframe embedding from same origin:**

```toml
[security_headers]
x_frame_options = "SAMEORIGIN"
```

**Increase limits for a heavier workload:**

```toml
max_body = 5
max_connections = 256
rate_limit = 120
timeout = 10
memory = 64
```

**Bind to all interfaces (for Docker or remote access):**

```toml
host = "0.0.0.0"
```

Or via environment variable:

```bash
HOST=0.0.0.0 simple serve
```

## Security

The server ships with security defaults enabled out of the box.

### Built-in protections

| Protection | Details |
|------------|---------|
| **Path traversal** | Percent-encoding decoded before validation; `..` and null bytes rejected; symlinks outside `dist/` blocked |
| **Blocked file extensions** | `.db`, `.sqlite`, `.env`, `.env.*`, `.envrc`, `.key`, `.pem`, `.sh`, `.sql`, `.log`, `.htaccess`, `.bak`, `.swp`, `.DS_Store`, `.gitignore` &mdash; never served or copied into `dist/` |
| **Rate limiting** | Per-IP, per-window on `/api/*` (default: 60 req / 60s, configurable) |
| **Content-Type enforcement** | POST/PUT/PATCH to `/api/*` with a body require `application/json` (returns 415 otherwise) |
| **SSRF protection** | `fetch()` in edge functions blocks private IPs, localhost, link-local, and numeric IP encodings |
| **Request smuggling** | `Transfer-Encoding` header rejected; `Content-Length` required |
| **SQL injection** | Parameterized queries supported; SQLite authorizer whitelist blocks VACUUM, ATTACH, triggers, views, and `load_extension()` |
| **JS sandbox** | Memory cap (default 32 MB), stack limit (1 MB), execution timeout (default 5s); uncaught errors return 400 with details logged server-side only |

### Security headers

Sent on **all** responses (static files and API):

| Header | Default | Configurable |
|--------|---------|:------------:|
| `Content-Security-Policy` | `default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'` | Yes |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` | Yes |
| `X-Frame-Options` | `DENY` | Yes |
| `X-Content-Type-Options` | `nosniff` | No |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Yes |
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=()` | Yes |
| `Cross-Origin-Opener-Policy` | `same-origin` | Yes |
| `Cross-Origin-Resource-Policy` | `same-origin` | Yes |

Set any header to an empty string `""` in `simple.toml` to disable it.

## Static files

Everything inside `static/` is copied as-is into `dist/`, preserving directory structure. Files with blocked extensions (see Security) are excluded.

Cache headers:
- HTML files: `Cache-Control: no-cache`
- Other files: `Cache-Control: public, max-age=86400`

## CSS tree-shaking

At build time, CSS is automatically tree-shaken and inlined per page.

For each HTML page, the build:
1. Parses the CSS file into individual rules
2. Extracts all tags, classes, and IDs from the page's HTML
3. Keeps only the rules whose selectors match elements actually present in the page
4. Replaces the `<link rel="stylesheet">` with an inline `<style>` containing only the used rules
5. Removes the original CSS file from `dist/` (no longer needed)

Rules that are always kept regardless of the page content:
- `@font-face` declarations
- `@keyframes` animations
- `:root` and `*` selectors

`@media` and `@supports` blocks are kept only if they contain at least one matching inner rule. Grouped selectors (`a, .card, span`) are pruned individually &mdash; only the matching groups are emitted.

**Example:** a page with `<main>`, `<p>`, and `<footer>` will not include rules for `.hero`, `.grid`, `.card`, or `.counter` even if those rules exist in the source CSS.

The inlined CSS is compacted (whitespace collapsed) and then benefits from the same gzip/brotli pre-compression as the rest of the HTML.

## Docker deployment

### Build the base image (once)

From this repository:

```bash
docker build -t simple .
```

This builds the `simple` binary into a minimal Debian image.

### Full workflow

```bash
# Local development
mkdir my-site && cd my-site
simple init
simple serve

# Build & push the site image
docker build -t user/my-site .
docker push user/my-site

# On your VPS
scp compose.yaml user@vps:~/my-site/
ssh user@vps "cd my-site && docker compose up -d"
```

Point a DNS A record to your VPS IP. Done.

### Generated Dockerfile

`simple init` creates this Dockerfile for your site:

```dockerfile
FROM simple
WORKDIR /app
COPY . .
RUN simple build
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["simple", "serve"]
```

No Rust compilation at this stage &mdash; just copy files and build HTML.

### Generated compose.yaml

```yaml
services:
  app:
    build: .
    ports:
      - "127.0.0.1:3000:3000"
    volumes:
      - ./data.db:/app/data.db
    restart: unless-stopped
```

The app listens on `127.0.0.1:3000` &mdash; only reachable through a reverse proxy, not directly from the internet. The volume persists `data.db` on the host.

### Reverse proxy

You need a reverse proxy in front for HTTPS and domain routing.

**Nginx** (if already on your VPS):

```nginx
server {
    server_name example.com;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

```bash
certbot --nginx -d example.com
```

**Caddy** (zero-config automatic HTTPS):

Add to `compose.yaml`:

```yaml
services:
  caddy:
    image: caddy:2
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
    restart: unless-stopped

volumes:
  caddy_data:
```

`Caddyfile`:

```
example.com {
    reverse_proxy app:3000
}
```

When using Caddy in Docker Compose, change `127.0.0.1:3000:3000` to `expose: ["3000"]` so the app is only reachable through Docker's internal network.

### Registry

Any Docker registry works:

```bash
# Docker Hub
docker build -t user/my-site .
docker push user/my-site

# GitHub Container Registry
docker build -t ghcr.io/user/my-site .
docker push ghcr.io/user/my-site
```

On the VPS, replace `build: .` with the registry image in `compose.yaml`:

```yaml
services:
  app:
    image: user/my-site
    # ...
```
