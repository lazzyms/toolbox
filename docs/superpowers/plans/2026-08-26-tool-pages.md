# Toolbox Feature Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add dedicated PDF and image tool pages and turn the homepage tools section into a concise category gateway.

**Architecture:** Keep the site static and dependency-free. Reuse the existing stylesheet, icon sprite, theme behavior, download resolver, and page chrome across `index.html`, `pdf.html`, and `images.html`; use data attributes only where existing JavaScript already supports them.

**Tech Stack:** Static HTML, hand-written CSS, plain ES2020 JavaScript, SVG symbols.

**Spec:** `docs/superpowers/specs/2026-08-26-tool-pages-design.md`

## Global Constraints

- No framework, bundler, external font, or runtime dependency.
- Preserve the current visual system and responsive behavior.
- Keep all processing claims accurate to the native app.
- Preserve download resolution, theme toggle, accessibility affordances, and external links.

### Task 1: Create shared category page structure

**Files:**
- Create: `docs/pdf.html`
- Create: `docs/images.html`
- Modify: `docs/index.html`

**Interfaces:**
- Consumes: Existing SVG symbol IDs and current registry copy in `docs/index.html`.
- Produces: Three linked pages with consistent header, hero, category cards, FAQ link, footer, and download CTAs.

- [ ] **Step 1: Add the PDF page**

  Copy the existing document shell and icon sprite into `docs/pdf.html`; give it a PDF-specific title, description, canonical URL, navigation links, and a hero with a download CTA. Render the 17 current PDF utilities as individual cards using the existing SVG symbols.

- [ ] **Step 2: Add the image page**

  Copy the same shell into `docs/images.html`; give it an image-specific title, description, canonical URL, navigation links, and a hero with a download CTA. Render the 14 current image utilities as individual cards using the existing SVG symbols.

- [ ] **Step 3: Replace the homepage tool lists with category gateways**

  In `docs/index.html`, replace the long PDF and image checklists with two category cards containing counts, short descriptions, representative capabilities, and links to `pdf.html` and `images.html`.

### Task 2: Add the category-card presentation

**Files:**
- Modify: `docs/assets/styles.css`

**Interfaces:**
- Consumes: Existing `.card`, `.grid`, `.kicker`, `.section`, `.btn`, and theme tokens.
- Produces: Responsive `.category-grid`, `.feature-grid`, `.category-card`, `.feature-card`, and category page hero styles.

- [ ] **Step 1: Add category gateway styles**

  Style the homepage cards with a larger icon, tool count, feature preview list, and full-width text link while preserving keyboard focus and hover states.

- [ ] **Step 2: Add feature card styles**

  Style dedicated-page cards with consistent icon, title, one-sentence description, and a small capability label. Use responsive columns that collapse cleanly below 760px.

- [ ] **Step 3: Add category page hero and CTA styles**

  Give category pages a left-aligned hero on wide screens, a compact back-to-overview link, and a final download panel without introducing new layout dependencies.

### Task 3: Wire page behavior and verify navigation

**Files:**
- Modify: `docs/assets/app.js`

**Interfaces:**
- Consumes: Existing download buttons marked with `data-dl`, theme toggle, and navigation selectors.
- Produces: Correct download resolution and active navigation on all three pages.

- [ ] **Step 1: Make download fallback page-neutral**

  Ensure a missing GitHub release points to the current page's source/release action without referencing removed homepage anchors.

- [ ] **Step 2: Verify each document directly**

  Run `node --check docs/assets/app.js`, serve `docs/` with `python3 -m http.server`, fetch `/`, `/pdf.html`, and `/images.html`, and assert each contains its expected title, navigation links, and download CTA.

- [ ] **Step 3: Check for stale anchors and formatting errors**

  Run `rg` for removed `#install`, `#build`, and `#tools` references where inappropriate, then run `git diff --check`.
