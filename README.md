# simple

Générateur de sites statiques avec edge functions, en Rust.

Zéro config, zéro dépendance frontend. Tu écris du HTML, tu déclares des composants, `simple build` génère ton site. Tu ajoutes des edge functions en JS, `simple serve` lance le tout.

**scaffold &rarr; dev &rarr; deploy.**

## Installation

```bash
cargo install --path .
```

## Démarrage rapide

```bash
simple init        # scaffold le projet
simple serve       # build + serveur de dev sur http://127.0.0.1:3000
```

## Commandes

| Commande | Description |
|----------|------------|
| `simple init` | Crée la structure du projet (pages, composants, fonctions, Dockerfile...) |
| `simple build` | Génère `dist/` |
| `simple serve [port]` | Build + serveur de dev (défaut: port 3000) |

## Structure du projet

```
mon-site/
  simple.toml            # configuration (port, host, limites)
  layout.html            # layout global (optionnel)
  pages/                 # tes pages
    index.html
    about.html
  components/            # composants réutilisables
    Header.html
    Card.html
  static/                # copié tel quel dans dist/
    style.css
    main.js
  functions/             # edge functions JS
    hello.js
  Dockerfile             # image du site
  compose.yaml           # déploiement VPS
  .dockerignore
```

## Composants

Un composant = un fichier HTML dans `components/`. Nom en PascalCase.

### Composant simple

```html
<!-- components/Card.html -->
<div class="card">
</div>
```

```html
<!-- pages/index.html -->
<Card>
    <h1>Titre</h1>
    <p>Contenu</p>
</Card>
```

Résultat :

```html
<div class="card">
    <h1>Titre</h1>
    <p>Contenu</p>
</div>
```

Le contenu enfant va automatiquement dans l'élément racine du composant.

### Props

Passe des valeurs via les attributs avec `{{nom}}` :

```html
<!-- components/Header.html -->
<header>
    <h1>{{title}}</h1>
</header>
```

```html
<Header title="Mon Site" />
```

### Self-closing

Sans contenu enfant, ferme directement :

```html
<Header title="Accueil" />
```

### Imbrication

Les composants peuvent contenir d'autres composants. La résolution est récursive.

## Layout

`layout.html` enveloppe toutes les pages. Le contenu de chaque page est inséré avant `</body>`.

```html
<!DOCTYPE html>
<html lang="fr">
<head>
    <meta charset="UTF-8">
    <title>{{title}}</title>
    <link rel="stylesheet" href="/style.css">
</head>
<body>
</body>
</html>
```

`{{title}}` dans le layout est remplacé par la valeur du `<meta>` correspondant dans la page :

```html
<meta name="title" content="Accueil">

<p>Mon contenu</p>
```

## Clean URLs

`about.html` est généré en `about/index.html` et servi sur `/about`.

## Edge functions

Les fichiers JS dans `functions/` sont exécutés côté serveur via QuickJS.

Le routage est automatique :

| Fichier | URL |
|---------|-----|
| `functions/hello.js` | `/api/hello` |
| `functions/users/list.js` | `/api/users/list` |
| `functions/users/index.js` | `/api/users` |

### Request / Response

Chaque fichier doit exporter une fonction `handler(req)` :

```javascript
function handler(req) {
    // req.method  — GET, POST, etc.
    // req.path    — /api/hello
    // req.body    — corps de la requête
    // req.query   — query string (ex: "foo=bar")

    return {
        status: 200,                              // optionnel, défaut 200
        headers: { "content-type": "text/plain" }, // optionnel
        body: "Hello!"                             // string ou objet (auto JSON.stringify)
    }
}
```

### Base de données (SQLite)

Un objet `db` est disponible dans chaque fonction. La base (`data.db`) est créée automatiquement au premier appel.

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

**Requêtes paramétrées** (anti-injection SQL) :

```javascript
db.query("SELECT * FROM users WHERE id = ?", [userId]);
db.exec("INSERT INTO users (name) VALUES (?)", [name]);
```

### fetch()

Appels HTTP sortants depuis les edge functions :

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

## Configuration

Fichier `simple.toml` à la racine du projet :

```toml
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
# Voir section Sécurité > Headers de sécurité pour les clés disponibles
```

Les variables d'environnement `PORT` et `HOST` prennent le dessus sur le fichier. L'argument CLI `simple serve [port]` a la priorité la plus haute sur le port.

**Priorité :** CLI > env vars > simple.toml > défauts

## Déploiement Docker

### Setup (une fois)

Build l'image de base `simple` depuis ce repo :

```bash
docker build -t simple .
```

### Flow complet

```bash
# Dev
mkdir mon-site && cd mon-site
simple init
simple serve

# Build & push
docker build -t user/mon-site .
docker push user/mon-site

# VPS
scp compose.yaml user@vps:~/mon-site/
ssh user@vps "cd mon-site && docker compose up -d"
```

DNS : A record vers l'IP du VPS. C'est en ligne.

### Le Dockerfile du site (généré par init)

```dockerfile
FROM simple
WORKDIR /app
COPY . .
RUN simple build
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["simple", "serve"]
```

Léger : pas de compilation Rust, juste copier les fichiers et builder le HTML.

### compose.yaml (généré par init)

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

L'app écoute sur `127.0.0.1:3000` — accessible uniquement via le reverse proxy, pas directement depuis l'extérieur. Le volume persiste `data.db` sur le host.

### Reverse proxy

L'app a besoin d'un reverse proxy devant elle pour le HTTPS et le domaine.

**Nginx** (si déjà installé sur le VPS) :

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

**Caddy** (alternative zéro config, HTTPS automatique) :

Ajoute au `compose.yaml` :

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

`Caddyfile` :

```
example.com {
    reverse_proxy app:3000
}
```

Avec Caddy, change `127.0.0.1:3000:3000` en `expose: ["3000"]` dans le compose (réseau Docker interne).

### Registry

N'importe quel registry Docker fonctionne :

```bash
# Docker Hub
docker build -t user/mon-site .
docker push user/mon-site

# GitHub Container Registry
docker build -t ghcr.io/user/mon-site .
docker push ghcr.io/user/mon-site
```

Sur le VPS, dans `compose.yaml`, remplace `build: .` par l'image du registry :

```yaml
services:
  app:
    image: user/mon-site
    # ...
```

## Sécurité

Le serveur intègre des protections par défaut :

- **Path traversal** : décodage percent-encoding, blocage de `..` et des symlinks hors `dist/`
- **Extensions bloquées** : `.db`, `.sqlite`, `.env`, `.env.*`, `.envrc`, `.key`, `.pem`, `.sh`, `.sql`, `.log`, `.htaccess`, `.bak`, `.swp`, `.DS_Store`, `.gitignore` — jamais servies ni copiées dans `dist/`
- **Rate limiting** : 60 requêtes / 60s par IP sur `/api/*`
- **Content-Type** : POST/PUT/PATCH vers `/api/*` avec body requièrent `application/json` (415 sinon)
- **SSRF** : les `fetch()` en edge function bloquent les IPs privées, localhost, et les encodages numériques
- **Request smuggling** : `Transfer-Encoding` rejeté, `Content-Length` requis
- **SQLite** : authorizer whitelist (pas de VACUUM, ATTACH, triggers, views, load_extension)
- **JS sandbox** : mémoire 32 MB, stack 1 MB, timeout 5s, erreurs non catchées → 400 (détails loggés côté serveur, jamais exposés au client)

### Headers de sécurité

Envoyés sur toutes les réponses (statiques et API) :

| Header | Défaut |
|--------|--------|
| `Content-Security-Policy` | `default-src 'self'; style-src 'self' 'unsafe-inline'` |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` |
| `X-Frame-Options` | `DENY` |
| `X-Content-Type-Options` | `nosniff` (toujours actif, non configurable) |
| `Referrer-Policy` | `strict-origin-when-cross-origin` |
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=()` |
| `Cross-Origin-Opener-Policy` | `same-origin` |
| `Cross-Origin-Resource-Policy` | `same-origin` |

Chaque header est configurable dans `simple.toml`. Une valeur vide désactive le header.

```toml
[security_headers]
# Autoriser Stripe
content_security_policy = "default-src 'self'; script-src 'self' https://js.stripe.com; frame-src https://js.stripe.com"

# Désactiver HSTS en dev
strict_transport_security = ""

# Autoriser l'intégration en iframe par un domaine spécifique
x_frame_options = "SAMEORIGIN"
```

## Fichiers statiques

Tout ce qui est dans `static/` est copié tel quel dans `dist/`.
