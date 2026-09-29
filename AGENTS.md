# Toolbox — Agent Guide

## Scope

This branch is the cross-platform Tauri app. The legacy Swift macOS app and its
release workflow were removed during the migration to Tauri. Do not reintroduce
Swift package files, Swift sources, or the Swift release workflow here. Keep
`docs/` unchanged unless the task explicitly targets the website.

## Commands

Run from `ToolboxTauri/`:

- `npm run test:ui` — all Playwright UI tests
- `npm run test:analytics` — install analytics unit tests
- `npm run test:native:e2e` — native command matrix with isolated fixtures
- `npm run check:release` — tool matrix and release configuration checks
- `npm run build` — TypeScript and Vite production build
- `npm run tauri build` — native installer bundle
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust tests

## Architecture

- `ToolboxTauri/src/` contains the React UI and feature views.
- `ToolboxTauri/src-tauri/src/` contains native commands and processing logic.
- `ToolboxTauri/tests/` contains browser automation using the checked-in fixture.
- `ToolboxTauri/src-tauri/src/main.rs` contains the fixture-backed native E2E matrix.

New processing behavior belongs in the native kit and must remain deterministic,
local, and testable. UI views should use the shared `ToolScaffold` and preserve
per-file failure isolation.

## UI components

- Use only shadcn/ui components from `ToolboxTauri/src/components/ui/` for app controls and reusable surfaces. Import them directly where they are used.
- Use the shadcn `Card` component for every card, bordered panel, preview panel, result/history surface, and reusable empty-state container. App-specific canvas or drop behavior may remain specialized inside a `Card` surface.
- Every app button, text/number/search field, select, textarea, checkbox, switch, slider, grouped control, and card must use its shadcn component. Do not write raw `<button>`, `<input>`, `<select>`, or `<textarea>` elements in app code; the only native-control exceptions are file and color pickers.
- Build modal surfaces with shadcn `Dialog`; do not recreate dialogs with raw ARIA markup. Do not attach `onClick` actions to generic layout elements such as `div`, `section`, `article`, or `span`.
- Do not add a parallel component library, custom control wrappers, or a replacement for a shadcn component. Do not copy a generated component outside `src/components/ui/`. Compose shadcn components with semantic HTML for app layout.
- Let generated shadcn styles control component appearance. Do not re-skin Cards, controls, badges, or dialogs with CSS; use generated Card anatomy, Button variants, component sizes, and class names for local layout or density. Keep CSS for app layout and feature-specific canvas geometry.
- Build PDF and image editing geometry with their existing canvas and SVG elements. Use shadcn buttons and fields for controls around those surfaces.
- Keep `components.json` on shadcn's `new-york` style and `neutral` base. Keep shared surfaces, actions, and focus states neutral. Reserve color for file content and clear status meaning.
- Run `npm run check:shadcn-components` after UI changes; it enforces these component and theme rules.

## Invariants

- Never modify originals or overwrite an existing output.
- Keep output naming collision-safe and operation-specific.
- Keep native helper/model discovery explicit and fail closed when unavailable.
- Maintain parity across macOS and Windows release targets.
- Keep analytics anonymous and development builds inactive.

## Agent skills

### Issue tracker

GitHub Issues for `lazzyms/toolbox`, operated with `gh`. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context domain documentation in `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.

## Release

`main` is the default branch and primary release line. Its release workflow
publishes signed macOS and Windows artifacts plus the updater manifest. Keep the
release path cross-platform and Tauri based.
