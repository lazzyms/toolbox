# Toolbox Feature Pages Design

## Goal

Make Toolbox easier to browse by moving the complete PDF and image tool catalogs into dedicated pages with concise, benefit-led feature cards.

## Architecture

The site remains a dependency-free GitHub Pages site. `index.html` becomes a product overview with two category cards, while `pdf.html` and `images.html` provide the full catalog for each category. All pages share `assets/styles.css`, the existing icon sprite pattern, and `assets/app.js` for theme and download behavior.

## Content and interaction

- Homepage navigation exposes Overview, PDF tools, Image tools, and FAQ.
- The homepage previews each category with a short description and a link to its dedicated page.
- Each category page contains a hero, a scannable responsive grid of feature cards, trust notes, and a download CTA.
- Cards use existing SVG symbols and do not claim capabilities beyond the app registry and current README.
- Existing download resolution, theme toggle, accessibility affordances, and external links remain functional.

## Constraints

- No framework, bundler, external font, or runtime dependency.
- Preserve the current visual system and responsive behavior.
- Keep all processing claims accurate to the native app.
