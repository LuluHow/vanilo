# vanilo

HTML, CSS, JS. No JSX, no virtual DOM, no 45-second builds, no 800 MB node_modules.

A static site generator with edge functions, written in Rust. You write HTML, run `vanilo build`, done. Need server-side logic? Drop a JS file in `functions/` and you've got an API.

**For:** devs who know how to write HTML and don't need a framework to do it for them.

## Quick start

```bash
cargo install --path .
vanilo init
vanilo serve
```

Your site runs at `http://127.0.0.1:3000`. Edit, reload, that's it.

## What you get

| Feature | One liner |
|---------|-----------|
| **Components** | HTML files in `components/`. PascalCase. Props, children, nesting. No syntax to learn. |
| **Layout** | One `layout.html`, all your pages wrapped in it. `{{title}}` comes from each page's `<meta>` tags. |
| **Clean URLs** | `about.html` → `/about`. Automatic. |
| **Content (CMS)** | JSON in `content/`. `{{@site.title}}` in HTML. `<Each content="posts">` to iterate. |
| **Edge functions** | Server-side JS via QuickJS. `functions/hello.js` → `/api/hello`. |
| **SQLite** | `db.query()`, `db.exec()` — available in every function. Nothing to configure. |
| **CSS tree-shaking** | Only the CSS rules used by each page get inlined. The rest is gone. |
| **Client-side rendering** | `Vanilo.put()`, `Vanilo.list()` — reuse your components client-side without innerHTML. |
| **Security** | CSP, HSTS, rate limiting, SSRF protection, path traversal blocking — on by default. |
| **Deploy** | `docker build && docker compose up`. DNS A record. That's it. |

## Project structure

```
my-site/
  vanilo.toml          # config
  layout.html          # global layout
  pages/               # your pages
  components/          # reusable components
  content/             # JSON (CMS)
  static/              # copied as-is
  functions/           # edge functions (JS)
  dist/                # output (generated)
```

## Components

An HTML file in `components/`, named in PascalCase. That's a component.

```html
<!-- components/Card.html -->
<div class="card">
    <h3>{{title}}</h3>
</div>
```

```html
<!-- usage -->
<Card title="Hello" />

<Card>
    <p>Child content goes into the root element.</p>
</Card>
```

Components nest inside each other. Resolution is recursive.

### Client-side

Every component is also emitted as a `<template>`. A minimal runtime lets you reuse them from JS:

```javascript
Vanilo.list('#messages', 'MessageCard', messages);   // array → DOM
Vanilo.put('#profile', 'UserCard', user);             // object → DOM
Vanilo.render('Card', { title: "Hi" });               // → HTML string
```

Props are auto-escaped. Unresolved placeholders are removed.

## Content (CMS)

```json
// content/site.json
{ "title": "My Site", "contact": { "email": "hello@example.com" } }
```

```html
<h1>{{@site.title}}</h1>
<a href="mailto:{{@site.contact.email}}">Contact</a>
```

Collections with `<Each>`:

```html
<Each content="posts">
    <Card title="{{title}}" href="/blog/{{slug}}" />
</Each>
```

## Edge functions

A JS file in `functions/`, a `handler(req)` function. That's it.

```javascript
// functions/hello.js
function handler(req) {
    // req.method, req.path, req.body, req.query

    db.exec("CREATE TABLE IF NOT EXISTS visits (count INTEGER)");
    var row = db.query("SELECT count FROM visits");

    return {
        status: 200,
        body: { visits: row[0].count }   // object → auto JSON.stringify
    }
}
```

| File | URL |
|------|-----|
| `functions/hello.js` | `/api/hello` |
| `functions/users/list.js` | `/api/users/list` |

`fetch()` is available for outbound HTTP calls. `db.query()` and `db.exec()` support parameterized queries (`?`) to prevent SQL injection.

## Deploy

```bash
docker build -t my-site .
docker push user/my-site

# On your VPS
docker compose up -d
```

`vanilo init` generates the `Dockerfile` and `compose.yaml`. Put a reverse proxy in front (Caddy, Nginx) for HTTPS. Point DNS. You're live.

## Configuration

Everything in `vanilo.toml`. Everything is optional.

```toml
port = 3000
host = "127.0.0.1"

max_body = 1            # MB
max_connections = 128
rate_limit = 60         # req/window on /api/*
rate_window = 60        # seconds

timeout = 5             # JS execution, seconds
memory = 32             # JS runtime, MB
fetch_timeout = 10      # outbound HTTP, seconds

[security_headers]
content_security_policy = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"
# strict_transport_security = "max-age=63072000; includeSubDomains"
# x_frame_options = "DENY"
# referrer_policy = "strict-origin-when-cross-origin"
# permissions_policy = "camera=(), microphone=(), geolocation=()"
```

Priority: CLI args > env vars (`PORT`, `HOST`) > `vanilo.toml` > defaults.

Set a header to `""` to disable it.

## Security

On by default. No config needed.

- **Path traversal** — `..`, null bytes, symlinks outside `dist/` blocked
- **Blocked extensions** — `.db`, `.env`, `.key`, `.pem`, `.sql`, `.log` etc. never served
- **Rate limiting** — per-IP on `/api/*`
- **SSRF** — `fetch()` blocks private IPs, localhost, link-local
- **SQL injection** — parameterized queries, restrictive SQLite authorizer
- **JS sandbox** — capped memory, timeout, stack limit
- **Headers** — CSP, HSTS, X-Frame-Options, nosniff — all enabled

## Commands

```
vanilo init              # scaffold the project
vanilo build             # generate dist/
vanilo serve [port]      # build + dev server
```
