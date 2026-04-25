# simple

Générateur de sites statiques en Rust. Zéro dépendance.

Tu écris du HTML, tu déclares des composants, `simple build` génère ton site.

## Installation

```bash
cargo install --path .
```

## Commandes

```bash
simple init     # crée la structure du projet
simple build    # génère dist/
```

## Structure du projet

```
mon-site/
  layout.html           # layout global (optionnel)
  pages/                # tes pages
    index.html
    about.html
  components/           # composants réutilisables
    Header.html
    Card.html
  static/               # copié tel quel dans dist/
    style.css
    main.js
```

## Composants

Un composant = un fichier HTML dans `components/`.

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

## Layout

`layout.html` enveloppe toutes les pages. Le contenu de chaque page est inséré dans `<body>`.

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

`{{title}}` dans le layout est remplacé par la valeur du `<meta>` correspondant dans la page :

```html
<meta name="title" content="Accueil">

<p>Mon contenu</p>
```

## Fichiers statiques

Tout ce qui est dans `static/` est copié tel quel dans `dist/`.
