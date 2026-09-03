# Tauri parity verification harness

## Goal

Prove the parity contract from [Define the parity acceptance contract for all
migrated tools](https://github.com/lazzyms/toolbox/issues/117) without using the
installed app, mutating source fixtures, or treating model-dependent output as
byte-deterministic.

## Verification layers

### 1. Static contract gate

Run on every pull request:

- Compare the Swift and Tauri registries by ordered utility id, category, and
  verification recipe.
- Validate every command/view contract, feature flag, output suffix, and
  supported-format declaration.
- Run TypeScript build, Rust tests, release-readiness checks, and dependency
  license/resource-manifest checks.

### 2. Native deterministic gate

Run the Rust kit and IPC command tests with generated disposable fixtures. Each
case records SHA-256 before and after and asserts that originals are unchanged.
The matrix covers every registry feature, including empty input, malformed
input, password failure, collision naming, batch isolation, and cancellation.

Assertions are typed by operation:

- Lossless promises: byte identity where explicitly promised; otherwise exact
  pixels plus dimensions, alpha, color profile, metadata, frame/page count,
  order, and required container invariants.
- Lossy operations: dimensions, alpha policy, orientation, metadata policy,
  output format/suffix, and a bounded perceptual difference against the Swift
  golden output.
- Privacy/model tools: no output when the adapter is absent, malformed, timed
  out, or finds no applicable subject; successful output includes the detected
  count or an equivalent diagnostic.

### 3. Golden fixture gate

Keep small, license-safe fixtures in a versioned fixture package. Generate
legacy Swift outputs once per fixture and record the tool version, OS, options,
hashes, and comparison rule in a manifest. Include:

- Oriented JPEG and transparent PNG
- Lossless and lossy WebP, HEIC, GIF, and multi-page TIFF
- Metadata-bearing JPEG/PNG/WebP/TIFF
- Animated GIF and unsupported animated WebP
- PDFs with text, scans, annotations, passwords, multiple pages, and embedded
  images
- Every icon preset and collision case
- Face profiles/occlusion/multiple faces and representative foreground scenes

Golden files are regenerated only through an explicit baseline-update workflow
that requires review. A model or codec version change cannot silently rewrite
the baseline.

### 4. Adapter matrix gate

For OCR, face blur, and background removal, run two cases on each supported
platform and architecture:

1. Adapter-present with the pinned bundled engine and model manifest.
2. Adapter-absent or invalid with the resource removed or checksum altered.

The first case compares normalized OCR text, face-region coverage, or mask
metrics against the accepted model baseline. The second case asserts the
feature is unavailable, produces no output, leaves the input hash unchanged,
and leaves unrelated tools usable. Never substitute a developer PATH tool in a
release test.

### 5. Live UI gate

Run against a freshly built, disposable Tauri app using the platform UI
harness. Do not drive `/Applications/Toolbox.app` or another installed copy.
For every registry entry, exercise the real path: open tool, focus controls,
choose fixture through the native dialog or drop surface, run, wait for the
progress state to clear, and inspect the visible result row.

The live matrix must cover keyboard-only navigation, screen-reader names and
roles, focus order, disabled/unavailable states, error announcements, progress
announcements, and the Preview-first PDF editor arrangement. Capture a
screenshot before execution and after the result is shown.

### 6. Package and update gate

Build the native release matrix: macOS arm64 and Windows arm64/x86_64. On each
artifact, verify:

- Native architecture and launchability
- Bundled helper/model presence, manifest checksum, and private-resource
  discovery
- Ad hoc/self-signature state and documented warning behavior
- Installer output, updater signature, GitHub Release URL, and architecture
  manifest entry
- Missing-helper behavior after temporarily removing one resource

Notarization and publicly trusted signing are not gates while the project uses
the free signing policy, but the workflow must leave explicit diagnostics when
those credentials are absent.

## Evidence and retention

Every run writes to `.verification/tauri-<timestamp>/` with:

- `manifest.json` containing commit, target, OS, app version, tool id, fixture,
  adapter/model versions, and comparison rule
- `before.sha256` and `after.sha256` for every input
- Structured result JSON and comparison report
- UI screenshots and accessibility snapshot
- Build, test, package, and updater transcripts

CI uploads the directory as an artifact on success and failure. Keep PR
artifacts for 14 days and release-gate artifacts for the release retention
period. Do not include user files, secrets, or full production documents.

## Merge gates

Pull requests cannot merge when static, deterministic, fixture, adapter, or
required live-platform jobs fail. Model-dependent jobs must fail closed when a
model is missing rather than being skipped. A platform job may be marked
unavailable only by an explicit feature flag backed by the platform-support
decision and a recorded reason; it cannot be inferred from runner absence.

