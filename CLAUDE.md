# Toolbox repository guide

The primary application in this branch is the cross-platform Tauri app under
`ToolboxTauri/`. The legacy Swift app and its release workflow are retained only
on `main`. The static `docs/` website is independent and must remain unchanged
unless a task explicitly targets it.

## Commands

```bash
cd ToolboxTauri
npm ci
npm run test:ui
npm run test:analytics
npm run test:native:e2e
npm run check:release
npm run build
npm run tauri build
```

Native tests can be run with:

```bash
cargo test --manifest-path ToolboxTauri/src-tauri/Cargo.toml
```

## Code layout

- `ToolboxTauri/src/` — React shell, settings, shared components, and feature views
- `ToolboxTauri/src-tauri/src/` — Rust commands, processing kit, helper discovery,
  and native fixture tests
- `ToolboxTauri/tests/` — Playwright and analytics tests
- `ToolboxTauri/src-tauri/tauri.conf.json` — window, bundle, and updater settings
- `.github/workflows/tauri-release.yml` — macOS and Windows release matrix

Keep processing local and deterministic, never modify originals, avoid output
overwrites, and preserve per-file error isolation. Every user-visible behavior
change needs automated coverage.

## Branches and releases

`tauri-port` is the default branch and primary release line. Its release workflow
publishes signed macOS and Windows artifacts and updater metadata. The Swift
release line remains separate on `main`; do not add Swift package files, sources,
tests, or workflows to this branch.
