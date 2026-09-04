# Toolbox — Agent Guide

## Scope

This branch is the cross-platform Tauri app. The legacy Swift macOS app and its
release workflow live only on `main`. Do not reintroduce Swift package files,
Swift sources, or the Swift release workflow here. Keep `docs/` unchanged unless
the task explicitly targets the website.

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

## Invariants

- Never modify originals or overwrite an existing output.
- Keep output naming collision-safe and operation-specific.
- Keep native helper/model discovery explicit and fail closed when unavailable.
- Maintain parity across macOS and Windows release targets.
- Keep analytics anonymous and development builds inactive.

## Release

`tauri-port` is the default branch and primary release line. Its release workflow
publishes signed macOS and Windows artifacts plus the updater manifest. The Swift
release workflow remains on `main` and must not be copied into this branch.
