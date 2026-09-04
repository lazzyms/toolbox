# Contributing to Toolbox

Toolbox is a cross-platform Tauri desktop app. Contributions should target the
`tauri-port` branch, which is the repository default and primary release line.
The legacy Swift macOS release remains separate on `main`; the `docs/` website is
also maintained independently.

## Getting set up

```bash
git clone https://github.com/lazzyms/toolbox.git
cd toolbox/ToolboxTauri
npm ci
```

Install Rust stable and the platform dependencies required by the native PDF
helpers. Run the full verification set before opening a pull request:

```bash
npm run test:ui
npm run test:analytics
npm run test:native:e2e
npm run check:release
npm run build
```

## Where changes belong

| Change | Location |
| --- | --- |
| React UI | `ToolboxTauri/src/` |
| Native processing and commands | `ToolboxTauri/src-tauri/src/` |
| UI automation | `ToolboxTauri/tests/` |
| Native fixture E2E | `ToolboxTauri/src-tauri/src/main.rs` |
| Release configuration | `.github/workflows/tauri-release.yml`, `ToolboxTauri/src-tauri/` |
| Website | `docs/` |

Keep the native command boundary explicit and cross-platform. Originals must not
be modified, output collisions must not overwrite existing files, and one failed
input must not stop the rest of a batch.

## Website

`docs/` is a static site with no build step. Keep website changes separate from
application changes, do not add trackers or third-party embeds, and leave its
appcast behavior intact.

## Pull requests

Open pull requests against `tauri-port`. Describe the behavior changed and the
verification run. UI changes should include a focused Playwright regression test;
native changes should include fixture-backed coverage.

Use short imperative commit messages under roughly 72 characters, without a
trailing period. Toolbox is released under the [MIT License](LICENSE).
