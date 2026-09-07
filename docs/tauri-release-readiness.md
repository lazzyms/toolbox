# Tauri release readiness

## Microsoft Store release

The reserved MSIX product is **Toolbox: PDF & File Tools** (Store ID
`9N5R8W4GJVH4`). The `microsoft-store.yml` workflow builds the x64 MSIX on
every push to `main` and publishes it with the [Microsoft Store Developer CLI]
after the initial Partner Center submission metadata is complete.

Privacy policy URL: <https://lazzyms.github.io/toolbox/privacy.html>

Configure these GitHub repository variables:

- `TOOLBOX_MSIX_STORE_ID`: `9N5R8W4GJVH4`
- `TOOLBOX_MSIX_IDENTITY_NAME`: `MaulikSompura.ToolboxPDFFileTools`
- `TOOLBOX_MSIX_PUBLISHER`: `CN=FD70DA91-97ED-48EA-8594-B3F94ADBB4FD`
- `TOOLBOX_MSIX_PUBLISHER_DISPLAY_NAME`: `Maulik Sompura`

Configure these GitHub repository secrets for the [GitHub Actions Store
publisher setup]:

- `AZURE_AD_APPLICATION_CLIENT_ID`
- `AZURE_AD_APPLICATION_SECRET`
- `AZURE_AD_TENANT_ID`
- `SELLER_ID`

The first submission still needs to be completed in Partner Center, including
pricing and availability, category, age rating, Store listing, privacy policy,
and certification notes. After that draft is publishable, pushes to `main`
can submit package updates automatically.

The workflow uses the committed [MSIX manifest template](../ToolboxTauri/packaging/windows-msix/AppxManifest.xml.template)
and [MSIX build script](../ToolboxTauri/scripts/build-msix.ps1). The package is
unsigned locally; Microsoft signs accepted Store packages.

[Microsoft Store Developer CLI]: https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/overview
[GitHub Actions Store publisher setup]: https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/github-actions

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
