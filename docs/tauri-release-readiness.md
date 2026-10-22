# Tauri release readiness

## Microsoft Store release

The reserved MSIX product is **Toolbox: PDF & File Tools** (Store ID
`9N5R8W4GJVH4`). The `microsoft-store.yml` workflow builds the x64 MSIX after
each Tauri release on `main` and uploads it as the `toolbox-msix` artifact.
Submit that package to the Store manually in Partner Center.

Privacy policy URL: <https://lazzyms.github.io/toolbox/privacy.html>

Configure these GitHub repository variables:

- `TOOLBOX_MSIX_STORE_ID`: `9N5R8W4GJVH4`
- `TOOLBOX_MSIX_IDENTITY_NAME`: `MaulikSompura.ToolboxPDFFileTools`
- `TOOLBOX_MSIX_PUBLISHER`: `CN=FD70DA91-97ED-48EA-8594-B3F94ADBB4FD`
- `TOOLBOX_MSIX_PUBLISHER_DISPLAY_NAME`: `Maulik Sompura`

Every Store submission, including the package, listing, screenshots and
certification notes, is completed in Partner Center.

The workflow uses the committed [MSIX manifest template](../ToolboxTauri/packaging/windows-msix/AppxManifest.xml.template)
and [MSIX build script](../ToolboxTauri/scripts/build-msix.ps1). The package is
unsigned locally; Microsoft signs accepted Store packages.

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
and exposes per-output Open and Reveal actions. The latest published Tauri 1.0.11
build predates the offline vision and OCR resource integration on `main`, so those
features remain unavailable in that public download until a newer release is
published.
