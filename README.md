# Toolbox

A native macOS app collecting the small file utilities you keep needing. Your files
are processed entirely on-device and never uploaded. The only network access is the
update check described in [Updates](#updates), which you can turn off.

**Current tools**

| Tool | What it does |
| --- | --- |
| Remove PDF Password | Saves a decrypted copy of a PDF you know the password for. Text stays selectable and searchable. |
| Convert Image Format | HEIC ⇄ PNG ⇄ JPEG ⇄ TIFF (+ WebP where macOS supports it). Reads RAW too. |
| Compress Images | Lossless (format-preserving) or lossy with a quality slider. Never returns a file bigger than the original. |
| Resize Images | Fit inside a box, longest side, percentage, or exact pixels. |

Every tool takes multiple files at once (drag-and-drop or a file picker), processes
them in parallel, and reports per-file results. One bad file never stops the batch.

## Install

Download `Toolbox-<version>.dmg`, open it, and drag **Toolbox** to Applications.

Because this build isn't notarized by Apple, the first launch needs one extra step:

> **Right-click** Toolbox in Applications → **Open** → **Open**

That's once per machine; afterwards it's a normal double-click. (See
[Distribution](#distribution) for how to remove this step entirely.)

## Updates

Toolbox updates itself using [Sparkle](https://sparkle-project.org). On first launch
it asks whether to check automatically — nothing is requested from the network before
you answer. After that it checks once a day, and **Toolbox → Check for Updates…**
works any time. Both behaviours are togglable in **Toolbox → Settings**.

Every download is verified against an Ed25519 public key baked into the app bundle
(`SUPublicEDKey` in `Resources/Info.plist`) before anything is installed. The matching
private key exists only in the maintainer's Keychain, so publishing an update requires
that key — a compromised GitHub account alone can't push code to installed copies.

### Cutting a release

```bash
./Scripts/release.sh --version 1.1.0 [--notes notes.md]
./Scripts/release.sh --version 1.1.0 --dry-run   # build + sign the feed, publish nothing
```

That builds the universal app and DMG, signs the DMG with your private key, updates
`docs/appcast.xml` (the feed clients poll), then tags and creates the GitHub release.
It aborts if the feed ends up unsigned rather than shipping something clients reject.

Two one-time setup steps:

1. `generate_keys` (in Sparkle's `bin/` under `.build/artifacts`) creates the keypair
   and stores the private half in your Keychain. Put the printed public key in
   `SUPublicEDKey`. Changing this key later strands everyone already running the app.
2. Enable GitHub Pages once — **Settings → Pages → deploy from branch → main → /docs**
   — so `SUFeedURL` resolves.

Sparkle compares `CFBundleVersion`, not the marketing version, so `release.sh` derives
it from `git rev-list --count HEAD` to keep it monotonic without manual bookkeeping.

## Build from source

Requires Xcode 16 or newer.

```bash
swift test                              # run the test suite
./Scripts/build-app.sh --version 1.0.0  # → dist/Toolbox.app (universal)
./Scripts/make-dmg.sh   --version 1.0.0 # → dist/Toolbox-1.0.0.dmg
```

`build-app.sh` compiles arm64 and x86_64 slices, `lipo`s them into a universal
binary, generates the icon, and code-signs with the Hardened Runtime.

## Distribution

The scripts support three signing levels:

| Mode | Command | Result for other people |
| --- | --- | --- |
| Ad-hoc (default) | `./Scripts/build-app.sh` | Works, but needs the right-click-Open step on first launch |
| Developer ID | `./Scripts/build-app.sh --sign auto` | Fewer warnings, still not fully trusted |
| Notarized | `--sign auto` then `./Scripts/notarize.sh` | Opens with a normal double-click, no warnings |

Notarization needs a paid Apple Developer account ($99/year). `Scripts/notarize.sh`
documents the one-time credential setup and checks your signature before
uploading. The app itself stays free either way — notarization is about Gatekeeper
trust, not licensing.

## Adding a utility

The registry in `Sources/Toolbox/Utility.swift` is the only place that knows the
tool list. To add one:

1. Put the processing logic in `Sources/ToolboxKit/` as a plain function —
   file in, file out, no UI imports. This keeps it unit-testable.
2. Add a SwiftUI view in `Sources/Toolbox/Features/`. Wrap it in `ToolScaffold`
   and reuse `DropZone`, `FileList`, `ResultsList` and `DestinationPicker` to get
   drag-and-drop, batching, progress and Finder reveal for free.
3. Append a `Utility` entry to `Utility.all` and add its `id` to the `switch` in
   `makeView()`.

`BatchRunner.run` handles concurrency, progress and per-file error isolation, so
a new tool usually needs no threading code of its own.

## Project layout

```
Sources/ToolboxKit/     processing logic, no UI — where the tests point
  PDF/                  PDFUnlocker
  Images/               ImageProcessor, ImageFormat, ResizeSpec
  Support/              BatchRunner, OutputNaming, errors, formatting
Sources/Toolbox/        SwiftUI app
  Utility.swift          the tool registry
  Components/            shared UI: drop zone, file list, scaffold
  Features/              one view per tool
  Updates/               Sparkle wiring and the Settings pane
Resources/              Info.plist, entitlements, generated icon
Scripts/                build-app.sh, make-dmg.sh, release.sh, notarize.sh, make-icon.swift
docs/                   appcast.xml — the update feed, served by GitHub Pages
Tests/                  37 tests over real generated PDFs and images
```

`ToolboxKit` has no dependencies at all, so the file-processing code stays offline and
testable; Sparkle is linked only into the app target.

## Notes on behaviour

- **Originals are never modified.** Results are written next to the input (or to
  a folder you choose) with a suffix like `-unlocked`, `-compressed`, `-resized`.
- **Nothing is overwritten.** A name collision becomes `photo-1.png`, `photo-2.png`, …
- **Compression never inflates a file.** Re-encoding a photo can legitimately
  produce more bytes (HEIC is about twice as efficient as JPEG; PNG is much worse
  for photos). When that happens the original bytes are copied instead, keeping
  the correct file extension.
- **Rotation is handled.** EXIF orientation is baked into the pixels on resize,
  so iPhone photos come out upright rather than sideways.
- **Not sandboxed, deliberately.** Under App Sandbox, picking `photo.heic` doesn't
  grant permission to create `photo.png` beside it, which would break the default
  "save next to the original" workflow. The app runs under the Hardened Runtime
  instead. See `Resources/Toolbox.entitlements`.
- **Ad-hoc builds weaken one protection.** Library Validation requires every loaded
  library to share the executable's Team ID, and an ad-hoc signature has no Team ID,
  so dyld refuses to load the embedded `Sparkle.framework`. Ad-hoc builds therefore
  sign with `Resources/Toolbox-adhoc.entitlements`, which disables it — and that also
  permits unsigned dylib injection. Developer ID builds keep it enabled, since their
  Team IDs match. `build-app.sh` picks the right entitlements automatically and
  smoke-launches the bundle, because this class of mistake only surfaces at runtime.
- **PDF unlocking requires the real password.** It decrypts with the credential
  you supply; it does not crack anything.
