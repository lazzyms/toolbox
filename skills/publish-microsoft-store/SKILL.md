---
name: publish-microsoft-store
description: "Build Toolbox's Windows MSIX and publish it through the reserved Microsoft Store product, using GitHub Actions for packaging and Computer/Safari for manual Partner Center submission when needed."
---

# Publish Toolbox to Microsoft Store

Use this skill when the user asks to build, upload, submit, or publish the
Toolbox Windows app to Microsoft Store. The reserved product is the source of
truth for Store identity; do not create a second product.

## Product and repository

- Repository: `lazzyms/toolbox`
- Branch: `main`
- App project: `ToolboxTauri/`
- Workflow: `.github/workflows/microsoft-store.yml`
- Store product ID: read repository variable `TOOLBOX_MSIX_STORE_ID` with
  `gh variable list --repo lazzyms/toolbox`; the current reserved product is
  `9N5R8W4GJVH4`.
- The package identity and publisher must come from the repository variables
  `TOOLBOX_MSIX_IDENTITY_NAME`, `TOOLBOX_MSIX_PUBLISHER`, and
  `TOOLBOX_MSIX_PUBLISHER_DISPLAY_NAME`.

Never put Azure, Partner Center, or signing secret values in this skill,
source files, command output, or chat messages.

## Choose the path

1. If the Microsoft Store credentials are configured in GitHub, use the
   workflow path. It builds the MSIX, validates it, and publishes it through
   the reserved product.
2. If credentials are absent or the workflow stops at `msstore reconfigure`,
   use the manual path. The workflow uploads the MSIX artifact before the
   authentication step, so a failed authentication run may still provide a
   usable artifact. Do not describe that run as a successful publication.
3. If running on Windows with the Windows SDK available, the package can also
   be built locally with `ToolboxTauri/scripts/build-msix.ps1`. macOS cannot
   run `MakeAppx.exe`; use the workflow artifact there.

## Preflight

From the repository root, verify the working tree and the release inputs:

```bash
git status --short --branch
gh variable list --repo lazzyms/toolbox
gh secret list --repo lazzyms/toolbox
```

For a local Windows build, run from `ToolboxTauri/`:

```powershell
npm ci
npm run check:release
cargo test --manifest-path src-tauri/Cargo.toml
.\scripts\build-msix.ps1
```

The expected package is under `ToolboxTauri/.artifacts/msix/`. Never overwrite
an existing package or modify source files just to make packaging pass.

## Workflow path

Dispatch the existing workflow against `main` and capture its run ID:

```bash
gh workflow run microsoft-store.yml --repo lazzyms/toolbox --ref main
gh run list --repo lazzyms/toolbox --workflow microsoft-store.yml --limit 1
```

Watch the exact run with `gh run watch RUN_ID --repo lazzyms/toolbox --exit-status`.
For a credentialed run, terminal success is required before claiming automatic
publication. For a credential-less run, download the artifact only after
confirming that `Upload MSIX build artifact` succeeded:

```bash
gh run download RUN_ID --repo lazzyms/toolbox --name toolbox-msix --dir /tmp/toolbox-msix
```

Do not retry repeatedly on unchanged state. Inspect the failed step and fix the
specific missing prerequisite.

## Manual Partner Center path through Computer/Safari

Use the Computer/CUA UI when the user asks to operate the logged-in browser.
Prefer accessibility labels, roles, and visible text; element indices are
volatile and must be rediscovered after every navigation or upload.

1. Open Partner Center and select the reserved product, not “New product”.
   The product Store URL is `https://apps.microsoft.com/detail/9N5R8W4GJVH4`.
2. Create or open a submission for the new MSIX version.
3. Open Packages, choose Windows Desktop, and upload the exact `.msix` from
   the downloaded artifact. Verify the package identity, architecture, and
   version shown by Partner Center before saving.
4. Complete Store listing fields and required PNG screenshots. Use factual,
   search-friendly language for PDF, Office, image, local processing, merge,
   split, compress, convert, organize, protect, unlock, watermark, and batch
   image tools. Do not claim cloud processing, unsupported formats, or features
   not present in the app.
5. Check Properties, Age ratings, and Submission Options. For the desktop
   `runFullTrust` warning, provide a factual explanation that Toolbox launches
   local file-processing helpers and processes user-selected files on-device.
6. Save and wait until every required section reports `Complete`. A package
   upload or a draft is not a publication.
7. Before clicking **Submit for certification**, stop and ask the user for
   explicit confirmation at that moment. The button creates an external Store
   submission and must not be clicked based only on an earlier general request.
8. After confirmation, click it once, record the resulting submission status,
   and report whether it is in certification, approved, publishing, or live.

Do not enter payment, tax, or payout details for a free Store listing unless the
user explicitly requests a paid offer or Microsoft presents a required account
verification step that the user chooses to complete.

## Automatic publishing credentials

The workflow expects these GitHub encrypted secrets:

- `AZURE_AD_TENANT_ID`
- `SELLER_ID`
- `AZURE_AD_APPLICATION_CLIENT_ID`
- `AZURE_AD_APPLICATION_SECRET`

The Entra values are only for unattended GitHub authentication; they are not
needed for a browser-based manual submission. Never ask the user to paste a
client secret into chat. If the user wants to add it with GitHub CLI, have them
run `gh secret set NAME --repo lazzyms/toolbox` locally so the CLI prompts
without echoing the value, or use an already-authorized secure input channel.

## Verification and reporting

Before reporting success, verify the matching authoritative state:

- manual path: Partner Center shows the exact package in the reserved product
  and the submission status is recorded;
- automatic path: the exact GitHub Actions run is green through the publish
  step;
- code path: `git status --short --branch` is clean except for pre-existing
  user changes.

Report the package version, workflow run URL or Partner Center submission state,
and any remaining certification or credential gate. Do not call an artifact
build “published” until Microsoft reports a successful publish operation.
