# Tauri release readiness

Run these checks from `ToolboxTauri/` before a release rehearsal:

```bash
npm run check:release
npm run test:ui
npm run test:analytics
npm run test:native:e2e
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run check:release` checks the registry, configured bundle icons, Tauri
bundling, and updater metadata. It does not build or install a release.

The release workflow runs the UI, analytics, and native fixture tests on one
macOS arm64 runner and two Windows runners, then builds signed updater artifacts.
The workflow requires `TAURI_SIGNING_PRIVATE_KEY` and its password. It stages
qpdf and Poppler resources, publishes a macOS Apple silicon DMG, and publishes
Windows x64 and ARM64 installers. macOS uses the signing identity configured in
`tauri.conf.json`; the current value is ad hoc (`-`). Linux has no release target.

The updater signatures are separate from platform trust. The current Windows
installers are not Authenticode-signed, so SmartScreen can show “Windows
protected your PC.” A public release should add a trusted Authenticode
certificate, sign each installer with SignTool in CI, and timestamp the
signature. The certificate identity should be kept in CI secrets; it should not
be committed to the repository. SmartScreen reputation still builds over time
after signing.

The updater manifest is published at `tauri-latest.json` on the protected
`main` branch. The app checks for updates after launch when the release
build sets `VITE_ENABLE_UPDATES=true`, asks before installing, verifies the
payload with the bundled public key, and relaunches after installation.

The UI keeps original inputs unchanged, appends collision-safe operation suffixes,
and exposes per-output Open and Reveal actions. The three vision features remain
explicitly unavailable because their offline resources are not bundled.
