## Problem Statement

Toolbox has a legacy native Swift app whose user-visible behavior is the parity
oracle, and a Tauri app that currently exposes the registry but does not yet
prove equivalent behavior across supported platforms. Users need the same
tools, outcomes, fidelity guarantees, accessibility behavior, and safe failure
states on macOS arm64 and Windows 10/11 arm64 and x86_64.

## Solution

Build the Tauri app as a cross-platform replacement for the legacy app using a
shared command and adapter contract. Implement every legacy registry feature
behind that contract, with Preview-first PDF editing, explicit image
frame/page handling, bundled offline helpers, deterministic output rules, and
fixture-gated model behavior. Publish native architecture-specific GitHub
Release artifacts with signed updater manifests. Use ad hoc macOS signing and
optional Windows self-signing while paid notarization and publicly trusted
signing remain deferred.

## User Stories

1. As a Toolbox user, I want every legacy registry utility available in the
   Tauri app, so that migrating platforms does not remove capabilities.
2. As a macOS Apple Silicon user, I want a native arm64 application, so that
   the app does not depend on translation or emulation.
3. As a Windows user, I want native arm64 and x86_64 artifacts for Windows
   10/11, so that the installer matches my machine architecture.
4. As a user, I want to select files through the native file dialog or the
   drop surface, so that both common input paths behave consistently.
5. As a keyboard user, I want the complete workflow reachable in a predictable
   focus order, so that I can process files without a pointer.
6. As a screen-reader user, I want controls, progress, results, errors, and
   unavailable features announced with meaningful names, so that the app is
   understandable without visual inspection.
7. As a user, I want each operation to leave the original file unchanged, so
   that processing is safe and reversible.
8. As a user, I want output names to receive the documented suffix and avoid
   collisions automatically, so that existing files are never overwritten.
9. As a user, I want batch processing to isolate per-file failures, so that one
   bad input does not discard successful results for other files.
10. As a user, I want PDF pages rendered in the Preview-first arrangement with
    thumbnails on the left, the active canvas in the center, and the tool
    inspector on the right, so that the editor feels familiar.
11. As a PDF editor user, I want crop, sign, and organize actions to show the
    active page, scope, selection, overlay, and resulting order before saving,
    so that I can understand the operation.
12. As a user, I want PDF text, page count, annotations, orientation, and
    metadata invariants preserved according to the selected operation, so that
    editing does not silently damage documents.
13. As a user, I want password protection and unlocking to require and verify
    the real password, so that a failed password never creates a misleading
    success output.
14. As a user, I want PDF conversion, extraction, merging, splitting, page
    numbering, watermarking, metadata, compression, and OCR tools to expose
    their meaningful options, so that the Tauri workflow matches the legacy
    workflow.
15. As an image user, I want resize, rotate, crop, tone, watermark, compress,
    convert, metadata, and icon operations to preserve the documented color,
    alpha, orientation, and metadata semantics, so that outputs are trustworthy.
16. As an image user, I want lossless operations to preserve pixels and all
    required structural invariants, so that “lossless” has an observable
    meaning.
17. As an image user, I want lossy operations to honor quality and report their
    format and metadata policy, so that I can choose the right trade-off.
18. As an image user, I want animated GIF/WebP and multi-page TIFF inputs to
    retain their frame/page structure or fail clearly, so that content is never
    silently flattened.
19. As an image user, I want HEIC, WebP, GIF, TIFF, PNG alpha, EXIF, XMP, ICC,
    and GPS behavior to be explicit, so that conversion results are predictable
    across macOS and Windows.
20. As an icon designer, I want named macOS, favicon, iOS, Android, and custom
    icon presets with exact artifact names and sizes, so that generated assets
    can be dropped into their target projects.
21. As a privacy-conscious user, I want OCR, face blur, and background removal
    to run offline, so that files never leave my device.
22. As a user of face blur, I want the app to report how many faces it found
    and refuse to claim success when none were found, so that I can review the
    result honestly.
23. As a user of background removal, I want a transparent PNG with soft subject
    edges and upright pixels, so that cutouts remain usable around hair and fur.
24. As an OCR user, I want recognized text ordered like the document and saved
    as text, so that the result can be searched and reused.
25. As a user, I want a missing or corrupt helper/model to disable only its
    feature with an actionable explanation, so that unrelated tools remain
    usable.
26. As a developer, I want bundled helpers and models selected from private
    application resources, so that production behavior does not depend on PATH.
27. As a developer, I want development overrides for helpers, so that adapters
    can be tested without rebuilding the full release bundle.
28. As a release maintainer, I want adapter and model versions/checksums pinned,
    so that model-dependent behavior changes are deliberate and reviewable.
29. As a user, I want updates delivered through GitHub Releases with a signed
    platform-specific manifest, so that update authenticity can be checked.
30. As a user, I want a failed signature check or unavailable update to leave
    the installed app usable and explain the failure, so that updates cannot
    strand me.
31. As a tester, I want disposable fixtures, golden outputs, hashes, screenshots,
    accessibility snapshots, and transcripts retained as evidence, so that a
    parity claim can be independently inspected.
32. As a maintainer, I want deterministic and model-dependent checks to be
    merge gates on every supported platform, so that regressions do not reach
    release unnoticed.

## Implementation Decisions

- Use the shared Tauri command/adapter contract as the primary test seam. The
  frontend owns user interaction and state; the native layer owns processing,
  helper discovery, output naming, and typed outcomes. Do not duplicate
  processing logic in React.
- Keep the legacy Swift implementation as the behavioral oracle for golden
  fixtures. Match user-visible interactions and outcomes while allowing a
  different rendering implementation.
- Implement the PDF editor with the Preview-first layout: page thumbnails on
  the left, active canvas in the center, and tool inspector on the right. Keep
  page scope, tool selection, density, selection/overlay state, keyboard
  navigation, and result preview coherent.
- Model images as either a single frame or an explicit frame sequence, and PDFs
  as an explicit page sequence. Never silently reduce a sequence to frame/page
  one. Preserve or reject animation and multi-page content deliberately.
- Apply EXIF orientation once to pixels and clear the baked orientation tag.
  Preserve alpha for formats that support it. Make JPEG flattening an explicit
  conversion policy.
- Define pass-through, lossless, and lossy operations separately. Byte identity
  is required only where promised; decoded pixels and structural/metadata
  invariants are required for other lossless paths; lossy paths use bounded
  perceptual comparisons.
- Use Tesseract 5 with bundled language data for OCR, OpenCV Zoo YuNet for face
  detection, and U²-Netp through a pinned CPU ONNX adapter for background
  removal. Keep deterministic blur and cutout rendering outside the model.
- Bundle helpers and model assets per architecture with a manifest containing
  versions, checksums, licenses, input contracts, and resource limits. Resolve
  bundled resources first; retain environment/PATH overrides only for
  development and CI.
- Support macOS arm64 and Windows 10/11 arm64 and x86_64 with native release
  artifacts. Linux is deferred to a separate effort.
- Use ad hoc macOS signing and optional Windows self-signing under the current
  free-signing policy. Document Gatekeeper and SmartScreen warnings. Do not
  claim notarization or public trust without the required paid credentials.
- Publish architecture-specific installers and updater payloads through
  GitHub Releases. Sign the updater manifest and payload metadata independently
  of platform code signing. Preserve the installed app on update failure.
- Keep feature availability explicit and typed. Missing, corrupt, unsupported,
  or over-limit inputs produce no output and do not modify the source.
- Retain the project glossary in `CONTEXT.md`; use “supported target”,
  “release artifact”, “bundled helper”, “development override”, “adapter”, and
  “model baseline” consistently.

## Testing Decisions

- Test external behavior at the highest seam: the Tauri command/adapter
  contract and the real WebView UI. Avoid tests coupled to internal helper
  functions or CSS structure.
- Run static registry/contract checks, TypeScript build, Rust kit/IPC tests,
  and release-readiness checks on every pull request.
- Generate disposable fixtures and assert input SHA-256 is unchanged after
  every operation. Assert suffixes, extensions, collision naming, result
  details, and per-file failure isolation.
- Maintain reviewed Swift golden fixtures for oriented JPEG, transparent PNG,
  HEIC, lossless/lossy WebP, GIF, multi-page TIFF, metadata-bearing files,
  representative PDFs, and every icon preset.
- Compare deterministic outputs with byte identity only where promised, and
  otherwise with exact pixels plus structural and metadata invariants. Compare
  lossy/model outputs with operation-specific bounded perceptual or semantic
  metrics.
- Run adapter-present and adapter-absent tests for OCR, face blur, and
  background removal on every supported OS/architecture. Missing models must
  fail closed and never be skipped.
- Drive every registry feature through a freshly built disposable Tauri app,
  including native file selection/drop, visible options, progress, result rows,
  errors, keyboard navigation, screen-reader semantics, and Preview-first PDF
  editor behavior.
- Build and inspect macOS arm64 and Windows arm64/x86_64 release artifacts.
  Verify native architecture, helper/model manifests, updater signatures,
  installer presence, and behavior after removing one bundled helper.
- Retain redacted `.verification/tauri-<timestamp>/` evidence with environment
  manifest, fixture/model versions, hashes, comparisons, screenshots,
  accessibility snapshots, and build/test/package transcripts.
- Use the existing Swift test fixtures, Tauri registry matrix checks, Rust kit
  tests, and `skills/verify-toolbox` recipes as prior art. Extend the
  verification skill as implementation adds capabilities.

## Out of Scope

- Linux release support in this cycle.
- Paid Apple Developer ID signing, macOS notarization, and publicly trusted
  Windows code signing.
- New utilities not present in the legacy registry.
- Cloud processing, telemetry, network fallback, or runtime model downloads.
- Rewriting the legacy Swift app.

## Further Notes

The implementation should proceed in dependency order: shared contracts and
fixture harness, image/PDF fidelity foundations, PDF editor operations,
remaining tools, bundled vision adapters, then packaging/accessibility/release
gates. Each implementation ticket should carry its blocking edges and close
only with the relevant evidence bundle attached.

