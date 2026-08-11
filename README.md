# Toolbox

A native macOS app collecting the small file utilities you keep needing. Your files
are processed entirely on-device and never uploaded. The only network access is the
update check described in [Updates](#updates), which you can turn off.

**Website:** <https://lazzyms.github.io/toolbox> (source in [`docs/`](docs/))

**Licence:** [MIT](LICENSE) — free to use, fork, rebrand and sell. See
[License](#license).

**Current tools**

| Tool | What it does |
| --- | --- |
| Remove PDF Password | Saves a decrypted copy of a PDF you know the password for. Text stays selectable and searchable. |
| Convert Image Format | HEIC ⇄ PNG ⇄ JPEG ⇄ TIFF (+ WebP where macOS supports it). Reads RAW too. |
| Compress Images | Lossless (format-preserving) or lossy with a quality slider. Never returns a file bigger than the original. |
| Resize Images | Fit inside a box, longest side, percentage, or exact pixels. |

Every tool takes multiple files at once (drag-and-drop or a file picker), processes
them in parallel, and reports per-file results. One bad file never stops the batch.

## Window or menu bar

Toolbox runs either way, and every tool works the same in both.

- **Menu bar** — the hammer icon opens a panel with the full toolbox: pick a tool,
  drop files in, run it. On by default; turn it off in **Settings → General**.
- **Dock-less** — switch on **Hide Dock icon** and Toolbox becomes a menu bar app:
  no Dock icon, no app switcher entry, no window at login. It takes effect
  immediately, no relaunch. The menu bar icon stays switched on while the Dock icon
  is hidden, so there's always a way back — its menu also has **Show Dock Icon** and
  **Open Toolbox Window**.
- **Open at login** is in the same place, for either mode.

## Install

Grab the latest `Toolbox-<version>.dmg` from
**[Releases](https://github.com/lazzyms/toolbox/releases/latest)**, open it, and drag
**Toolbox** to Applications. After that, Toolbox keeps itself up to date — see
[Updates](#updates).

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

Releases are automatic: **merging a PR to `main` publishes one**. No version to bump,
no tag to push, nothing to run.
[`.github/workflows/release.yml`](.github/workflows/release.yml) runs the tests, builds
the universal app and DMG, signs the feed, tags, and creates the GitHub release —
after which installed copies pick it up within a day.

The version comes from the newest `v*` tag, bumped one patch. Labels on the PR change
that:

| Label on the merged PR | Result |
| --- | --- |
| *(none)* | patch — `1.0.0` → `1.0.1` |
| `minor` | `1.0.0` → `1.1.0` |
| `major` | `1.0.0` → `2.0.0` |
| `no-release` | nothing is published |

Release notes come from the merged PR: a `## Release notes` section in its description
if there is one, otherwise the PR title. They show up on the release page *and* in
Sparkle's "what's new" dialog, so that section is worth writing whenever the title
alone wouldn't tell a user what changed.

To release by hand, or to test a change to the pipeline:

```bash
./Scripts/next-version.sh                        # what would ship next
./Scripts/release.sh --version 1.1.0 --dry-run   # build + sign the feed, publish nothing
./Scripts/release.sh --version 1.1.0 [--notes notes.md]
gh workflow run Release -f version=1.1.0         # or from CI, with --dry-run available
```

`release.sh` aborts if the feed ends up unsigned rather than shipping something clients
reject. Pushing a `ci/**` branch runs the whole workflow as a rehearsal — build, DMG,
signed feed — and stops before publishing.

One-time setup, all of it already done for this repo:

1. `generate_keys` (in Sparkle's `bin/` under `.build/artifacts`) creates the keypair
   and stores the private half in your Keychain. Put the printed public key in
   `SUPublicEDKey`. Changing this key later strands everyone already running the app.
2. `generate_keys -x key.txt`, then add the contents as the `SPARKLE_PRIVATE_KEY`
   repository secret and delete the file — a runner has no Keychain to unlock, and
   `release.sh` reads that variable when it's set.
3. **Settings → Actions → General → Workflow permissions → Read and write**, so the
   workflow can commit `docs/appcast.xml` and create the release.
4. Enable GitHub Pages once — **Settings → Pages → deploy from branch → main → /docs**
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
   drag-and-drop, batching, progress and Finder reveal for free. Those components
   read `\.toolPresentation` from the environment and size themselves for the
   window or the menu bar panel, so one view covers both; for option pickers use
   `.optionPickerStyle(presentation)` rather than `.pickerStyle(.segmented)`,
   which is too wide for the panel.
3. Append a `Utility` entry to `Utility.all` and add its `id` to the `switch` in
   `makeView()`. Its `shortTitle` is what the menu bar panel's picker chips show,
   so keep it to a word or two.

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
  MenuBar/               the status-item panel, Dock/login-item settings
  Updates/               Sparkle wiring and the Settings pane
Resources/              Info.plist, entitlements, generated icon
Scripts/                build-app.sh, make-dmg.sh, release.sh, next-version.sh, notarize.sh
.github/workflows/      ci.yml (tests on PRs), release.yml (release on merge)
docs/                   the website and appcast.xml — both served by GitHub Pages
Tests/                  37 tests over real generated PDFs and images
```

`ToolboxKit` has no dependencies at all, so the file-processing code stays offline and
testable; Sparkle is linked only into the app target.

## Website

`docs/` is a single static page — `index.html` plus `assets/`. There is no build
step, bundler or dependency, so GitHub Pages serves it as-is. Enabling Pages is
the same one-time step the update feed needs (see [Updates](#updates)); the site
and `appcast.xml` are served from the same directory and don't interact.

`assets/styles.css` is a hand-written port of the [shadcn/ui](https://ui.shadcn.com)
design language rather than a framework install — the same design tokens
(`--background`/`--foreground`, `--card`, `--muted`, `--primary`, `--border`,
`--ring`, `--radius`) and the same component recipes (button, card, badge, alert,
separator, table, accordion, tabs, input, slider, progress), which is what keeps
the directory build-free. The palette is shadcn's "neutral" scale with the chroma
set to zero, so the page is greyscale in both appearances: emphasis comes from
contrast and borders, never from hue. Adding a component means adding its recipe
under the `shadcn components` heading in that file and reusing the tokens.

To work on it locally:

```bash
python3 -m http.server -d docs 8000    # → http://localhost:8000
```

Append `?theme=light` or `?theme=dark` to force an appearance without touching
system settings.

Two things there are generated rather than hand-maintained:

- `docs/og.png` — the social preview card. Re-render it with
  `swift Scripts/make-og-image.swift` after changing the wording. Note it still
  uses the app's blue brand gradient, unlike the page itself.
- `docs/assets/icon.svg` — the app icon redrawn on the same 1024pt grid as
  `Scripts/make-icon.swift`; keep the two in sync. It stays in the app's brand
  colours on purpose, so the favicon matches the icon in the Dock.

The tip jar is the Buy Me a Coffee widget loaded from `cdnjs.buymeacoffee.com` at
the bottom of `index.html`, rather than a section in the page. It is the only
third-party request the page makes and the only part of it not served from this
repo; with JavaScript disabled nothing appears, so the footer keeps a plain link.

The download button resolves the real `Toolbox-<version>.dmg` asset from the
releases API at page load, so it points straight at the current DMG without the
URL needing to be edited each release. With JavaScript disabled it falls back to
the releases page.

`.nojekyll` matters here: it stops Pages running the files through Jekyll, which
would otherwise be free to reinterpret them.

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

## Contributing

Bug reports, tool requests and pull requests are all welcome —
[CONTRIBUTING.md](CONTRIBUTING.md) covers the setup, the coding conventions, what the
tests expect and how to open a PR. For a new utility, start with
[Adding a utility](#adding-a-utility) above; for anything large, open an issue first.

## License

Toolbox is released under the [MIT License](LICENSE). In plain terms, you can:

- use it for anything, personal or commercial, free of charge;
- modify it however you like;
- fork it, rename it, restyle it and ship it as your own product;
- sell it, or sell something built on top of it, and keep the money.

The only condition is that the copyright notice and licence text travel with copies or
substantial portions of the source. There's nothing to ask permission for and nothing to
pay. It comes with no warranty.

Sparkle, the app's one dependency, is MIT licensed too, so a redistributed build carries
the same terms. If you do ship your own build, repoint `SUFeedURL` and `SUPublicEDKey` in
`Resources/Info.plist` at your own update feed and signing key — otherwise your users
will be checking this project's feed for updates.

## Support

The app is free and stays free. If it's useful to you, there's a tip jar:

**[buymeacoffee.com/lazzyms](https://buymeacoffee.com/lazzyms)**

Entirely optional — nothing in the app is gated behind it.
