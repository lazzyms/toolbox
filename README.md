# Toolbox

A native macOS app collecting the small file utilities you keep needing. Everything
runs locally — the app has no network code.

**Website:** <https://lazzyms.github.io/toolbox> (source in [`docs/`](docs/))

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
Resources/              Info.plist, entitlements, generated icon
Scripts/                build-app.sh, make-dmg.sh, notarize.sh, make-icon.swift
Tests/                  37 tests over real generated PDFs and images
docs/                   the GitHub Pages site (static, no build step)
```

## Website

`docs/` is a single static page — `index.html` plus `assets/`. There is no build
step, bundler or dependency, so GitHub Pages serves it as-is:

**Settings → Pages → Source: Deploy from a branch → `main` / `docs`**

To work on it locally:

```bash
python3 -m http.server -d docs 8000    # → http://localhost:8000
```

Two things there are generated rather than hand-maintained:

- `docs/og.png` — the social preview card. Re-render it with
  `swift Scripts/make-og-image.swift` after changing the wording.
- `docs/assets/icon.svg` — the app icon redrawn on the same 1024pt grid as
  `Scripts/make-icon.swift`; keep the two in sync.

The download button resolves the real `Toolbox-<version>.dmg` asset from the
releases API at page load, so it points straight at the current DMG without the
URL needing to be edited each release. With JavaScript disabled it falls back to
the releases page.

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
  with no network entitlement instead. See `Resources/Toolbox.entitlements`.
- **PDF unlocking requires the real password.** It decrypts with the credential
  you supply; it does not crack anything.
