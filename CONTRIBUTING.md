# Contributing to Toolbox

Thanks for taking the time to help out. Toolbox is a small, deliberately boring macOS
app, and contributions of every size are welcome — a bug report, a typo fix on the
website, or a whole new utility.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Ways to contribute](#ways-to-contribute)
- [Getting set up](#getting-set-up)
- [Making a change](#making-a-change)
- [Tests](#tests)
- [Coding style](#coding-style)
- [Working on the website](#working-on-the-website)
- [Commit messages](#commit-messages)
- [Opening a pull request](#opening-a-pull-request)
- [Releases](#releases)
- [Licensing of contributions](#licensing-of-contributions)

## Code of conduct

Be decent to each other. Assume good faith, keep criticism about the code rather than
the person, and take the hint when a maintainer says a change isn't a fit. Harassment
of any kind isn't tolerated, and maintainers may edit, lock or remove contributions
that cross that line. Report problems privately by opening a
[security advisory](https://github.com/lazzyms/toolbox/security/advisories/new) or
emailing the maintainer.

## Ways to contribute

**Bug reports** are most useful with the macOS version, whether you're on Apple silicon
or Intel, the app version (**Toolbox → About**), what you did, what happened, and what
you expected. If a particular file triggers it and you can share it, that saves a lot
of guessing.

**Tool requests** belong in an issue before code. The bar is roughly: small, local,
file-in / file-out, and something you'd otherwise paste into a random website. Things
that need an account, a server or a network call are out of scope — the app ships
without a network entitlement, and that's a feature.

**Pull requests** are welcome for anything already discussed in an issue, plus obvious
wins that need no discussion: typos, broken links, failing edge cases, documentation.
For a large or architectural change, open an issue first so you don't spend a weekend
on something that gets declined.

**Security issues** should not be filed as public issues. Use a
[private advisory](https://github.com/lazzyms/toolbox/security/advisories/new) instead.

## Getting set up

You need macOS 14 or later and Xcode 16 or newer. There's no package manager step to
run by hand — SwiftPM fetches the one dependency (Sparkle) on first build.

```bash
git clone https://github.com/lazzyms/toolbox.git
cd toolbox

swift test                              # the test suite
./Scripts/build-app.sh --version 0.0.0  # → dist/Toolbox.app (universal, ad-hoc signed)
./Scripts/make-dmg.sh   --version 0.0.0 # → dist/Toolbox-0.0.0.dmg
```

`swift test` covers everything in `ToolboxKit`, which is where the real work happens,
so most changes can be developed and verified without launching the app at all. You
can also open `Package.swift` in Xcode and run the `Toolbox` scheme.

An ad-hoc signed build needs the one-time **right-click → Open** on first launch. See
[Distribution](README.md#distribution) in the README for why.

## Making a change

Where things live is documented in [Project layout](README.md#project-layout). The
short version:

| If you're changing… | Go to |
| --- | --- |
| Processing logic (PDF, images, batching) | `Sources/ToolboxKit/` |
| UI for a tool | `Sources/Toolbox/Features/` |
| Shared UI pieces | `Sources/Toolbox/Components/` |
| The tool list | `Sources/Toolbox/Utility.swift` |
| Updater / Settings | `Sources/Toolbox/Updates/` |
| The website | `docs/` |

**Adding a new utility** is three edits, described in
[Adding a utility](README.md#adding-a-utility). Keep the processing logic in
`ToolboxKit` as a plain function — file in, file out, no SwiftUI imports — so it can be
tested without a running app, and wrap the view in `ToolScaffold` so you inherit
drag-and-drop, batching, progress and Finder reveal instead of reimplementing them.

Behaviours the app guarantees, which a change shouldn't quietly break:

- Originals are never modified; results are written alongside with a suffix.
- Nothing is overwritten; collisions become `photo-1.png`, `photo-2.png`, …
- Compression never returns a file larger than the input.
- One failing file in a batch never stops the rest.
- `ToolboxKit` has no dependencies and does no networking.

## Tests

Add tests for anything you fix or add — `Tests/ToolboxKitTests/` generates real PDFs and
images via `Fixtures.swift` rather than checking binaries into the repo, so follow that
pattern instead of adding sample files. Run the full suite before pushing:

```bash
swift test
```

A pull request that changes `ToolboxKit` without touching the tests will usually get a
question about it. UI-only changes don't need tests, but say in the PR what you clicked
through to check them.

## Coding style

There's no linter in CI; match the code that's already there.

- Standard Swift conventions: `UpperCamelCase` types, `lowerCamelCase` members, 4-space
  indent, no trailing whitespace, roughly 100 columns.
- Prefer `struct` and value types; keep `class` for the places that genuinely need
  reference semantics or `NSObject` conformance.
- Surface failures as `ToolboxError` cases with messages a non-developer can act on.
  Don't `try!`, `fatalError` or silently swallow errors on a path a user can reach.
- Comments explain *why*, not *what*. The existing files are a good guide to the
  density — enough to explain a trade-off, not a narration of the code.
- Don't add dependencies. `ToolboxKit` must stay dependency-free, and a new package in
  the app target needs a strong argument in an issue first.

## Working on the website

`docs/` is a single static page — `index.html` plus `assets/` — with no build step,
bundler or dependency. Serve it locally:

```bash
python3 -m http.server -d docs 8000    # → http://localhost:8000
```

Notes before editing it:

- Two files are generated, not hand-maintained: `docs/og.png`
  (`swift Scripts/make-og-image.swift`) and `docs/assets/icon.svg`, which is the app
  icon redrawn on the same grid as `Scripts/make-icon.swift` — keep the two in sync.
- Check both appearances. Dark is the default; light is a full override under
  `[data-theme="light"]`.
- Keep it working without JavaScript. The download button falls back to the releases
  page, and the interactive app mock is decoration.
- Don't add trackers, analytics or third-party embeds. The page claims nothing is
  uploaded, and that has to stay true of the site too.
- Leave `docs/appcast.xml` alone — `Scripts/release.sh` writes and signs it.

## Commit messages

Write a short imperative subject line under ~72 characters, capitalised, no trailing
period:

```
Fix WebP option showing on systems that can't encode it
```

Use the body to explain why the change is needed and anything non-obvious about the
approach. Reference issues with `Fixes #12` so they close automatically. Conventional
Commits aren't required. Squash noise like "fix typo" commits before asking for review.

## Opening a pull request

1. Fork the repo and branch off `main`.
2. Make the change, with tests where they apply.
3. Run `swift test` and, for anything UI-facing, build and click through it.
4. Push and open a PR against `main`.

In the description, cover what changed and why, which issue it relates to, and how you
verified it. Screenshots help for UI changes, before/after numbers for anything about
file sizes. Small, focused PRs get reviewed faster than one that mixes a fix, a
refactor and a rename.

Expect review comments — they're normal, and usually about edge cases on the unhappy
path, since that's most of what a file utility does.

## Releases

Merging a pull request to `main` publishes a release. GitHub Actions runs the tests,
builds and signs the DMG, updates the feed installed copies poll, and creates the
GitHub release; the update reaches users within a day. The signing key is an Ed25519
private key held as a repository secret, so a release can only come from this repo.

For contributors that means two things:

- **Don't bump versions, edit `docs/appcast.xml`, or add tags in a pull request.** The
  version is derived from the newest tag, and the feed is rewritten by the workflow.
- **Your PR title becomes the release notes**, so write it as something a user would
  understand.

A maintainer picks the size of the bump with a label on the PR before merging: none for
a patch, `minor` or `major` to go further, `no-release` to merge without publishing.

## Licensing of contributions

Toolbox is released under the [MIT License](LICENSE). By opening a pull request you
agree that your contribution is licensed under the same terms, and that you have the
right to submit it. There's no CLA to sign.
