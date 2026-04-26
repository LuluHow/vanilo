# vanilo

A static site generator with edge functions — one Rust binary, zero dependencies.

Write HTML, run `vanilo build`, done. Need server-side logic? Drop a JS file in `functions/` and you've got an API. SQLite, CSS tree-shaking, security headers, Docker deploy — all built in.

**Status:** pre-1.0. API may change. Not yet tested at scale.

## Quick start

```bash
cargo install --git https://github.com/LuluHow/vanilo.git
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
| **CSS tree-shaking** | CSS rules matching each page's static HTML get inlined. Use `/* vanilo:keep .cls */` for classes added by JS. |
| **Client-side rendering** | `Vanilo.put()`, `Vanilo.list()` — reuse your components client-side. Props are auto-escaped via DOM. |
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

### Escaping

`{{prop}}` is **HTML-escaped** at build time (server-side) and at render time (client-side). This prevents XSS when values come from content or user input.

To output trusted raw HTML, use triple braces:

```html
<div>{{{bio}}}</div>   <!-- raw: <em>hello</em> stays as HTML -->
<p>{{bio}}</p>          <!-- escaped: &lt;em&gt;hello&lt;/em&gt; -->
```

The same applies to content placeholders: `{{@site.title}}` is escaped, `{{{@site.title}}}` is raw.

Unresolved placeholders are removed.

### Client-side

Every component is also emitted as a `<template>`. A minimal runtime lets you reuse them from JS:

```javascript
Vanilo.list('#messages', 'MessageCard', messages);   // array → DOM
Vanilo.put('#profile', 'UserCard', user);             // object → DOM
Vanilo.render('Card', { title: "Hi" });               // → HTML string
```

Client-side props are escaped via DOM (`textContent`).

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

A JS or TS file in `functions/`, a `handler(req)` function. That's it.

```typescript
// functions/hello.ts
interface Req { method: string; path: string; body: string; query: string; }

function handler(req: Req) {
    db.exec("CREATE TABLE IF NOT EXISTS visits (count INTEGER)");
    var row = db.query("SELECT count FROM visits");

    return {
        status: 200,
        body: { visits: row[0].count }   // object → auto JSON.stringify
    }
}
```

TypeScript is supported out of the box — type annotations, interfaces, enums, and generics are stripped at runtime via [oxc](https://oxc.rs). Plain `.js` files work as before. When both `hello.js` and `hello.ts` exist, `.js` takes priority.

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

`vanilo init` generates the `Dockerfile` and `compose.yaml`. The Dockerfile is self-contained — it builds vanilo from source in a multi-stage build.

Put a reverse proxy in front (Caddy, Nginx) for HTTPS. When using a proxy, set `trusted_proxy` in `vanilo.toml` so rate limiting uses the real client IP (see Configuration).

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

# When behind a reverse proxy, set this so rate limiting uses X-Forwarded-For.
# trusted_proxy = "127.0.0.1"    # or "*" to trust any peer

# JS minification (strips comments, trims whitespace). Off by default —
# the minifier does not handle regex literals. Enable for simple JS only.
# minify_js = true

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

- **HTML escaping** — `{{prop}}` and `{{@content}}` are HTML-escaped at build time. Use `{{{triple}}}` to opt out
- **Build linter** — blocks `{{prop}}` in `onclick`, `javascript:`, `<script>`, `style=` attributes. Blocks `eval()`, SQL concat, `document.write()` in edge functions
- **Path traversal** — `..`, null bytes, symlinks outside `dist/` blocked
- **Blocked extensions** — `.db`, `.env`, `.key`, `.pem`, `.sql`, `.log` etc. never served
- **Rate limiting** — per-IP on `/api/*` (set `trusted_proxy` when behind a reverse proxy)
- **SSRF** — `fetch()` blocks private IPs, localhost, link-local, cloud metadata, numeric-encoded IPs. DNS pre-resolved to prevent rebinding
- **SQL injection** — parameterized queries required. Restrictive SQLite authorizer (no ATTACH, no triggers, no load_extension)
- **JS sandbox** — QuickJS with capped memory, timeout, stack limit
- **Headers** — CSP, HSTS, X-Frame-Options, COOP, CORP, nosniff — all enabled

## CSS tree-shaking

At build time, each HTML page's `<link rel="stylesheet">` is replaced by an inline `<style>` containing only the CSS rules that match elements in that page. Unused rules are dropped.

**Limitation:** the tree-shaker scans static HTML only. Classes added dynamically by JS (e.g. `el.classList.add('open')`) are not detected. To preserve those rules, add a safelist comment in your CSS:

```css
/* vanilo:keep .open .modal .active */
```

Multiple selectors can be listed in a single comment. Tags, classes, and IDs are supported.

## Webhook (auto-rebuild on push)

Edit content in `content/`, push, site rebuilds. Zero config beyond two lines in `vanilo.toml`:

```toml
[webhook]
webhook_path = "/_hook/a3f8c91e..."   # generated by vanilo init, unique to your project
webhook_secret = "your-secret"
```

Then add the URL as a webhook in your GitHub/Gitea repo settings (Content type: `application/json`). On every push, vanilo verifies the HMAC-SHA256 signature and rebuilds the site atomically — visitors never see a broken page.

**Security:**
- No path + secret configured → endpoint doesn't exist (404)
- Path is random per project — can't be guessed
- HMAC-SHA256 signature verification (constant-time)
- Dedicated rate limit (5/min default), independent from API rate limit
- Concurrent rebuilds are deduplicated

## Commands

```
vanilo init              # scaffold the project
vanilo build             # generate dist/
vanilo serve [port]      # build + dev server (with hot reload)
```

`vanilo serve` watches `pages/`, `components/`, `static/`, `functions/`, `content/`, `layout.html`, and `vanilo.toml` for changes and rebuilds automatically (300ms debounce). No browser live-reload — refresh manually.

## Limits & non-goals

vanilo ships sites, not infrastructure. These are the current trade-offs:

- **Single instance.** SQLite is local. Rate limiting is per-process. This is not designed for horizontal scaling or load balancers.
- **JS minification is basic.** When enabled (`minify_js = true`), it strips comments and collapses whitespace, but does not understand regex literals. Off by default.
- **CSS tree-shaking is static.** It scans HTML at build time. JS-added classes need a `/* vanilo:keep */` safelist comment.
- **No browser live-reload.** `vanilo serve` rebuilds on file changes but does not inject a live-reload script. Refresh manually.
- **No JS bundling.** No module system, no JS tree-shaking.
- **No incremental SSG.** Every build regenerates everything.
- **HTTP/1.1 only.** The server does not support HTTP/2.
