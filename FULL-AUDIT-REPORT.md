# Toolbox Website Checklist Review

**Reviewed and published:** 2026-09-23  
**Scope:** Public GitHub Pages site and checked-in pages in `docs/`. Published in commit `30f31da` from the repository's `main/docs` source.

## Status

| Checklist item | Result | Evidence / remaining limit |
|---|---|---|
| Privacy Policy | Updated | Discloses the browser theme preference and distinguishes website storage from app analytics and update checks. |
| Terms & Conditions | Added | `docs/terms.html` describes the site, downloads, and MIT-licensed app, and is linked from public pages. It is not a jurisdiction-specific legal review. |
| Enforce HTTPS | Pass | HTTP redirects to HTTPS. |
| Cookie consent banner | No banner added | Website code has no analytics/ad scripts or tracking cookies. The theme choice uses local storage and is disclosed. This describes observed behavior, not a legal determination. |
| Meta titles/descriptions | Pass | Each of the five pages has page-specific title, description, and canonical URL metadata. |
| Social preview image | Pass | All five pages serve Open Graph and Twitter large-image metadata. `og.jpg` is 1200×630 and returns HTTP 200 as JPEG (133,528 bytes). |
| Image alt text | Pass | Live pages have no images without alt attributes. Screenshot descriptions no longer claim an incorrect app version; decorative SVG icons stay hidden from assistive technology. |
| Image compression | Improved | Replaced the 561,838-byte PNG with a 133,528-byte JPEG, about 76% smaller. Product screenshots remain lazy-loaded PNGs. |
| Page load speed | Unverified | A fresh PageSpeed Insights request returned HTTP 429. No Lighthouse or field Core Web Vitals score is claimed. |
| Color contrast | Fixed | Live browser measured 4.85:1 for the dark CTA default state and 6.38:1 on hover/focus. |
| Mobile responsiveness | Pass | All five live pages checked at 375px and 1440px with no horizontal overflow or JavaScript errors. Primary CTAs fit the mobile content width. |
| Broken links | Pass for checked paths | `node docs/check-docs.mjs` verifies local links, anchors, screenshots, and release destinations. All five page routes and `og.jpg` return HTTP 200; the macOS release URL resolves and the Microsoft Store listing was checked in a browser. This is not a crawl of every external link. |
| Single clear CTA | Pass | Download remains the filled primary action; View source is visually secondary. Category pages retain one prominent download action. |

## Release references

The site points to Tauri 1.0.11, published September 9, 2026. Its macOS DMG target resolves to the release asset, and Windows buttons route to the Microsoft Store listing. The public site was checked after publication.

## Verification

- `node docs/check-docs.mjs`
- `node --check docs/assets/app.js`
- `git diff --check`
- UI suite: 50 passed.
- Live browser checks at 375px and 1440px across `/`, `/pdf.html`, `/images.html`, `/privacy.html`, and `/terms.html`; no missing image alt attributes or JavaScript errors.
- Live HTTPS redirect, page routes, social metadata/image, and dark CTA contrast verified.
- GitHub Pages reports the build for commit `30f31da` as built. No GitHub Actions workflows ran for this commit, so no app release or Store publication was triggered.

## Remaining limit

PageSpeed Insights returned HTTP 429 after publication. Terms and Privacy copy is a factual product summary, not a jurisdiction-specific legal review.
