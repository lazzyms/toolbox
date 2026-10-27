# Shadcn component migration

## Frame

- [x] Read the Principles section of `poteto-mode`.
- [x] Ground the app, component calls, current UI E2E coverage, and live preview.
- [x] Define completion as removal of the old spec and preview, no app imports from the custom component system, shadcn as the only shared UI component source, a neutral theme, a passing build and UI E2E suite, and direct inspection of the live app.
- [x] Scope the work at about ten shared-shell and feature views, the shared controls they call, the theme CSS, and existing Playwright coverage. Estimate two to four hours because one shared control API currently serves all workspaces.
- [x] Set high rigor because a shared component change can affect every workflow. The live Vite preview already answers on port 1420. The project uses React 18 and Tailwind 4. Confirm current CLI compatibility before installation.

## Design the workflow

- [x] Remove the rejected custom component spec and preview fixture before adding the replacement.
- [x] Add a shadcn E2E contract before the app migration. It first failed on the custom controls, then checked the neutral theme in both modes.
- [x] Confirm Vite and Tailwind 4 compatibility with the shadcn CLI, then add the `new-york` components with the neutral base under `src/components/ui/`.
- [x] Migrate the shared shell, settings, workspace controls, security, media, conversion, image editing, and PDF editing views.
- [x] Add the shadcn-only rule to `AGENTS.md` and a source check for legacy classes, raw controls, configuration, and neutral palette tokens.
- [x] Remove the obsolete design-system source, preview fixture, and color-check command after app callers moved to shadcn.
- [x] Run the production build and UI E2E suite. Inspect the live app and light/dark screenshots.

## Run the loop

The first integrated UI run found 12 regressions in search semantics, theme aliases, PDF thumbnails, slider labels, and retained copy. Focused navigation and editor E2E checks passed after those fixes. Independent audits then found a hand-built settings dialog, a click-only PDF page surface, a focus restoration gap, and reusable panels still drawn with raw layout tags. Settings now uses shadcn Dialog; the button trigger forwards its DOM ref so Radix restores focus; PDF page selection uses a shadcn Label with a keyboard-operable Checkbox. Workspace panels, previews, the command rail, PDF thumbnails, empty states, and file selection lists now use shadcn Card. The source checker blocks those known reusable-surface classes on raw layout tags. E2E covers focus containment, Escape, focus restoration, panel surfaces, and clicking the page surface.

The final full suite passed 109 tests with 3 optional OCR and vision resource cases skipped. The card-surface E2E waits for the theme transition to settle before comparing the rendered background to its neutral token or saving a viewport screenshot. Its theme token check reads from the themed body so both light and dark scopes are actually verified.

## Keep the audit trail

The append-only decision log is `.audit/shadcn-component-migration.tsv`. This file records the workflow, not a component specification.

## Verify and hand back

`npm run check:shadcn-components`, `git diff --check`, and `npm run build` pass. The final full UI suite reports 109 passed and 3 skipped; the focused neutral theme and Card-surface E2E checks pass. The live Vite preview responds on port 1420 and was visually inspected. Native Tauri build and native command E2E were not run for this UI migration.

## Follow-up: remove residual legacy styling

The baseline is behaviorally green, but the component contract is still violated. The live app has custom CSS on Card roots, and the source checker does not inspect component styling. The root cause is retained `index.css` surface/button styling and unused aliases in `styles/theme.css`.

### Frame and workflow

- [x] Read the `poteto-mode` Principles section and the relevant leaf skills.
- [x] Capture the pre-change build, component check, UI suite, and live component computed styles.
- [x] Define done as generated Card/Button appearance coming from shadcn classes and variants, with no retired design-system files or shared-surface CSS overrides; feature layout and PDF/image canvas geometry remain intact.
- [x] Quantify scope as a 1,728-line app stylesheet, a 228-line mixed theme file, and the shared shell plus editor/workspace components. Baseline build and UI checks pass; visual style assertions are the missing gate.
- [x] Design the order as a red computed-style E2E contract, a CSS source guard, then stylesheet/token deletion and Card anatomy/variant composition, followed by a full UI and live-preview check.
- [x] Skip `architect` because this is a reversible migration to the explicit shadcn-only contract, not an open architecture fork. Skip fan-out because the CSS and component composition share one system and no isolated seams need parallel work.
- [x] Add the failing component-style E2E assertion and source guard before changing app styling.
- [x] Delete the legacy visual rules and unused theme aliases while preserving app layout and canvas geometry.
- [x] Recompose shared surfaces with generated Card anatomy and generated Button variants.
- [x] Run the source guard, build, full UI E2E, and inspect the updated desktop preview.

The user's consumer-facing change is stock shadcn component appearance across the app. The next maintainer should inherit one neutral theme, app-specific layout/geometry CSS, and a guard that catches CSS re-skinning of shared components.

### Result

The live preview exposed black Card borders because Tailwind's generated `border` utility inherited each Card's foreground color. A computed-style E2E assertion failed with the expected neutral border token (`oklch(0.922 0 0)`) versus the actual foreground token (`oklch(0.145 0 0)`). Setting the shared base border color to `var(--border)` fixed the source-level cause; the live Card borders now use the neutral theme in both modes.

The final UI run passed 110 tests with 3 capability-gated OCR and vision tests skipped. The production build, shadcn source check, diff check, and live preview inspection passed. The theme E2E now asserts both Card background and border against the active body tokens.
