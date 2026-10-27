# Toolbox Website Checklist — Resolution

The site fixes were published from commit `30f31da` to the GitHub Pages `main/docs` source. The Pages build completed successfully.

## Completed

1. Updated release references to Tauri 1.0.11 and verified the macOS release asset and Microsoft Store destination.
2. Added and linked a Terms of Use page; disclosed the website's saved theme preference in the Privacy Policy.
3. Added page-specific Open Graph and Twitter metadata to the homepage, PDF, images, Privacy, and Terms pages.
4. Replaced the old PNG social card with a platform-neutral 1200×630 JPEG at `docs/og.jpg` (133,528 bytes).
5. Fixed dark-theme CTA contrast: 4.85:1 in the default state and 6.38:1 on hover/focus.
6. Corrected screenshot alternative text and removed a stale app-version claim from its caption.
7. Updated the tool matrix and release notes to reflect the synced main-branch registry and release boundary.
8. Expanded `docs/check-docs.mjs` to verify metadata, release consistency, social image dimensions/size, CTA contrast, local links, and assets.

## Verified live

- All five public pages and the social image return HTTP 200; HTTP redirects to HTTPS.
- Browser checks at 375px and 1440px found no horizontal overflow, missing image alt attributes, or JavaScript errors.
- The Pages build for commit `30f31da` completed. The `[skip ci]` marker prevented the app-release and Microsoft Store workflows from running.

## Still open

- PageSpeed Insights returned HTTP 429. No PageSpeed, Lighthouse, or field Core Web Vitals score is available.
- The Terms and Privacy pages are factual product summaries, not a jurisdiction-specific legal review.
