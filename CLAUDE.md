# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Toolbox is a native macOS SwiftUI app bundling small on-device file utilities (PDF password removal,
image convert/compress/resize). `README.md` is detailed and current — read it for user-facing behaviour,
release/signing procedure, and the rationale behind the security tradeoffs. `CONTRIBUTING.md` covers
commit-message style (imperative, <~72 chars, no trailing period), the PR flow (branch off `main`, PR
against `main`), and scope rules for new tools. This file covers what isn't obvious from reading a
single file.

## Commands

```bash
swift test                          # all 37 tests
swift test --filter ImageProcessor  # one @Suite (match on the suite's *type* name, not its display string)
swift test --filter heicToPNG       # one test (match on the func name, not the @Test display string)
swift build                         # debug build
swift run Toolbox                   # run without bundling — see caveat below

./Scripts/build-app.sh --version 1.0.0   # → dist/Toolbox.app (universal, signed, smoke-launched)
./Scripts/make-dmg.sh   --version 1.0.0  # → dist/Toolbox-1.0.0.dmg
./Scripts/next-version.sh                # what the next release will be called
./Scripts/release.sh    --version 1.1.0 [--dry-run]   # CI runs this; rarely run by hand
```

`--filter` matches Swift identifiers, so `--filter "Compression size guard"` finds nothing — use
`CompressionGuardTests`. Requires Xcode 16+ (swift-tools-version 6.0, macOS 14 target).

`swift run Toolbox` produces a bare binary with no `Info.plist`, so `SUFeedURL` is absent and
`UpdateController` deliberately disables itself (`isAvailable == false`). Anything touching updates
must be tested from a real bundle via `build-app.sh`.

## Architecture

Two SwiftPM targets, and the split is the main invariant to preserve:

- **`Sources/ToolboxKit/`** — all processing logic. Zero dependencies (no Sparkle, no SwiftUI, no
  AppKit). Every entry point is a *synchronous* `file in → file out` function returning a `Sendable`
  result. This is what makes it callable from any concurrency domain and unit-testable without a
  running app. Tests point only here.
- **`Sources/Toolbox/`** — the SwiftUI app. Links Sparkle. Holds no processing logic.

New processing code belongs in ToolboxKit even if only one view calls it.

### The tool registry

`Sources/Toolbox/Registry/` is the single source of truth for the tool list, split so PDF and image
work never touch the same file:

- `Utility.swift` — the type, `Category`, and `all`, assembled as `pdfTools + imageTools`
- `Utility+PDF.swift`, `Utility+Images.swift` — one `[Utility]` per category

Adding a utility is **one appended line** in its category's array, and that line carries the tool's
own pane factory, so there is no lookup table to forget to update. Entries are one per line —
deliberately past the usual column budget — so two branches adding a tool conflict on that line
alone, resolved by keeping both.

Two things about `Utility` are deliberate. The stored `pane` closure takes the `Utility` and
`makeView()` feeds `self` back in, because every pane hands one to `ToolScaffold` for its header and
a closure written inline in the registry can't refer to the value it is initialising. And a closure
isn't equatable, so `Hashable`/`Equatable` are hand-written on `id` rather than synthesised.

### Convert, compress and resize are one implementation

`ImageProcessor.run(_:options:)` is decode → optional resize → encode. The three image tools differ
only in the `ImageProcessor.Options` they pass (`format`, `quality`, `resize`, `keepSmallerOriginal`,
`suffix`). Fix an image bug there once rather than per-view.

### Feature view pattern

Each view in `Features/` is `@State`-only (no view models) and follows the same shape: `files: [URL]`
plus tool options → build an `Options` value → `Task { await BatchRunner.run(...) }` → hop back with
`await MainActor.run { ... }` to publish `[JobOutcome]`. Wrap the body in `ToolScaffold` and reuse
`DropZone`, `FileList`, `ResultsList`, `DestinationPicker`, `OptionRow` — that provides drag-and-drop,
progress, the run bar, and Finder reveal. `BatchRunner.run` already handles bounded parallelism
(capped at core count), progress callbacks, and per-file error isolation, so a new tool needs no
threading code. Anything captured by the `job` closure must be `Sendable`; Swift 6 language mode is on.

The detail pane is `.id(selected.id)`-keyed in `ToolboxApp.swift`, so switching tools intentionally
tears down and resets each pane's queue and results.

## Invariants that tests enforce

These aren't stylistic — there are tests asserting each, and they encode real bugs already fixed:

- **Originals are never modified**; outputs get a suffix (`-unlocked`, `-compressed`, `-resized`).
- **Nothing is overwritten.** Always route destinations through `OutputNaming.destination`, which
  appends `-1`, `-2`, … on collision.
- **Compression never inflates a file.** Re-encoding can legitimately grow bytes (HEIC ≈ 2× more
  efficient than JPEG; PNG is far worse for photos). With `keepSmallerOriginal`, `ImageProcessor`
  copies the original bytes instead — under the *input's* extension, so the filename never lies about
  its contents. Skipped when resizing, where a size change is the point.
- **EXIF orientation is baked into pixels on resize**, so the orientation tag must *not* be copied
  into the output — doing so rotates the image twice.
- **PDF unlocking keeps text selectable.** `PDFDocument.write(to:)` on an unlocked document retains
  the encryption dictionary, so `PDFUnlocker` rebuilds into a fresh `PDFDocument`, verifies the result
  actually opens unlocked, and only then falls back to a CoreGraphics re-render. It requires the real
  password and cracks nothing.

Tests generate real PDFs and images on disk via `Tests/ToolboxKitTests/Fixtures.swift` rather than
mocking ImageIO/PDFKit. Guard format-dependent tests with `try #require(ImageFormat.x.canEncode)` —
WebP and HEIC *encoding* availability varies by macOS version.

## Packaging gotchas

- **Version numbers are injected, not committed.** `Resources/Info.plist` holds `__VERSION__` and
  `__BUILD__` placeholders that `build-app.sh` substitutes. The build number is
  `git rev-list --count HEAD` because Sparkle compares `CFBundleVersion`, not the marketing version.
- **Sparkle is linked but not embedded by SwiftPM.** `build-app.sh` copies the
  `macos-arm64_x86_64` slice with `cp -R` and adds an `@executable_path/../Frameworks` rpath by hand.
- **Two entitlement files, chosen by signing mode.** Ad-hoc signatures have no Team ID, so Library
  Validation refuses to load the embedded framework; ad-hoc builds therefore use
  `Toolbox-adhoc.entitlements` (validation disabled). Developer ID builds use `Toolbox.entitlements`.
  `build-app.sh` picks automatically and then actually launches the bundle, because this class of
  mistake only surfaces at runtime.
- Never use `codesign --deep` for Developer ID builds — it mis-signs Sparkle's XPC helpers, which then
  get rejected at update time. Sign inside-out in the order already in `build-app.sh`.
- The app is **deliberately not sandboxed** (Hardened Runtime only): under App Sandbox, picking
  `photo.heic` grants no permission to write `photo.png` beside it, breaking the default workflow.
- Universal builds compile arm64 and x86_64 in separate `swift build` invocations, then `lipo`.

`Resources/AppIcon.icns` is generated by `Scripts/make-icon.swift` and gitignored, as are `.build/`,
`dist/`, and `.vscode/`.

## Releasing is automatic — and unmerging is not an option

**Every push to `main` publishes a release** (`.github/workflows/release.yml`) that installed copies
auto-update to within a day. There is no staging step: whatever lands on `main` ships. `swift test`
gates it, and `ci.yml` runs the same tests on every PR so a red `main` doesn't become a failed release.

- **Version** comes from `Scripts/next-version.sh` — newest `v*` tag, bumped a patch. Labels on the
  merged PR override: `minor`, `major`, or `no-release` to skip publishing entirely.
- **Notes** are the merged PR's `## Release notes` section, falling back to its title. They are
  embedded in the appcast, so they're what Sparkle shows users — not just release-page decoration.
- **The signing key** is the `SPARKLE_PRIVATE_KEY` secret, read by `release.sh` when set (`--ed-key-file -`,
  via stdin) instead of the Keychain. Its public half is `SUPublicEDKey` in `Info.plist`; they must stay
  a pair or every client rejects the update.
- **Two loop guards**, because the workflow itself pushes a `Release X [skip ci]` commit to `main`:
  GITHUB_TOKEN pushes don't trigger workflows, and GitHub honours `[skip ci]` for pushes made with
  anyone's credentials (which is what covers a local `release.sh` run). Dropping the marker from that
  commit message turns one merge into an endless release chain. Guarding on the `Release ` prefix
  instead would have been worse — it silently skipped the commit that *added* this workflow.
- **Rehearsals**: pushing a `ci/**` branch runs the entire workflow and stops before tagging. Any ref
  other than `main` is forced to `--dry-run`. That is the only safe way to test a pipeline change.
- Running `Scripts/release.sh` locally still works and is occasionally right (a broken runner), but it
  pushes to `main` with your credentials and needs the Keychain key.

## Stacking changes on an unmerged PR

When work depends on a PR that hasn't merged yet, stack it rather than branching off `main` (which
would duplicate the parent's commits in the diff) or committing onto the open PR's branch (which
forces reviewers to re-review approved code).

Two tools are viable here. **`gh stack` is an official `gh` extension and is not installed** — run
`gh extension install github/gh-stack` first. **Graphite (`gt`) is already installed** and the repo is
initialized with trunk `main`. Pick one and stay with it; both keep their own parent metadata and
interleaving them desyncs whichever you weren't using.

### With `gh stack`

```bash
gh stack checkout 42          # stack number, PR number, PR URL, or branch
gh stack top                  # `add` must run from the topmost branch
gh stack add my-next-layer    # or: gh stack add -A -m "msg" to stage+commit inline
# ...edit, commit...
gh stack submit               # base is set to the branch beneath it automatically
```

After amending a lower branch: `gh stack rebase` (`--upstack` limits it to branches above the current
one). On conflict, resolve → `git add` → `gh stack rebase --continue`; `--abort` restores every branch.
`gh stack sync --prune` bundles fetch → rebase → push → PR sync and drops merged branches.
`gh stack view --short` shows the current order. Note a fully merged stack can't be extended — submit
then silently starts a *new* stack rooted at trunk.

### With Graphite

```bash
gt checkout <parent-branch>   # the unmerged PR's branch
gt create my-next-layer -am "msg"
gt submit --stack             # opens/updates a PR per branch with correct bases
```

If the parent branch was created with plain git, `gt track` it (bottom-up, confirming each parent)
before stacking, or Graphite won't know the parent. After changing a lower branch, `gt modify -am
"fix"` amends *and* restacks descendants; `gt sync` when `main` has moved. Conflicts resolve with
`git add` then `gt continue` — not `git rebase --continue`. `gt log short` is the diagnosis command.

Never mix raw `git rebase`/`git commit --amend` into a tracked stack; if it happens, `gt restack`
usually repairs the metadata and `gt info` shows the drift. To change trunk, use
`gt init --trunk <name> --reset` rather than editing `.git/.graphite_repo_config` by hand — trunk also
lives in `.git/.graphite_metadata.db` and editing only the JSON corrupts the pair.

### Two hazards specific to this repo

- **Worktrees silently break restacks.** This checkout is one of several worktrees on the same repo
  (`git worktree list`), and a branch checked out in another worktree cannot be checked out or
  restacked here — `gt restack` *skips it with a warning and the cascade stalls there*, so the change
  never reaches the branches above. Before restacking, `git -C <other-worktree> checkout --detach` any
  worktree holding a stack branch, then restore it afterwards.
- **`release.sh` is not stack-aware.** It derives `CFBundleVersion` from `git rev-list --count HEAD`,
  which counts commits on *whatever branch you run it from* — a stacked branch inflates the build
  number, and Sparkle compares that value, so releasing from a stack can strand a later release with
  a lower number. It also pushes `HEAD:main` (GitHub Pages serves `docs/appcast.xml` from the default
  branch only) and aborts rather than force-pushing if that isn't a fast-forward. **Cut releases from
  `main` after the stack has merged, not from a branch in the stack.**

## Agent skills

### Issue tracker

GitHub Issues via the `gh` CLI against `lazzyms/toolbox`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` plus `docs/adr/`. See `docs/agents/domain.md`.
