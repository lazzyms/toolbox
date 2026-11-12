# Settings

Settings controls appearance, where outputs go, Finder quick actions, optional packs, anonymous usage counts, and shows the version, support links, and keyboard shortcuts.

## Sub-features

- `appearance` selects `Light` or `Dark`.
- `output-location` saves `Next to each original` or `Always in one folder`.
- `quick-actions` turns Finder quick actions on or off.
- `packs` lists optional packs and installs one from a file.
- `privacy` controls `Share anonymous usage counts`.
- `updates` shows the running version and how updates arrive.
- `about-support` shows support links and open-source notices.
- `keyboard-shortcuts` lists shortcuts under `Everywhere` and `Batch tools`, with a `Search shortcuts` field.

## How to get to it (user POV)

- Choose `Settings` in the rail.
- Choose a section checkbox: `Appearance`, `Output location`, `Quick actions`, `Packs`, `Privacy`, `Updates`, `About and support`, or `Keyboard shortcuts`.
- Choose `All tools` or `← All tools` to leave Settings.

## Driving it with CUA desktop control

Preconditions:

- The verification window passes Doctor.
- Record the theme, output location, and privacy state before you change any of them.

- **Version.** Read `Version <x> for macOS` and compare it with `npm pkg get version` from `ToolboxTauri`.
- **Appearance.** Read which of `Light` and `Dark` is selected. If you change it, restore the starting theme.
- **Output location.** Read which of `Next to each original` and `Always in one folder` is selected. Do not change it unless the run needs it, and restore it after.
- **Privacy.** Read the `Share anonymous usage counts` checkbox state. If you change it, restore it before you leave.
- **Shortcuts.** Open `Keyboard shortcuts` and confirm the `Everywhere` and `Batch tools` headings and the `Search shortcuts` field.
- **Read-only sections.** Open `Quick actions`, `Packs`, `Updates`, and `About and support` and read their headings. Do not choose their controls.

## Gotchas

- Do not choose the Quick actions checkboxes, `Install from file…`, a pack install control, `Choose output folder`, or `Sponsor on GitHub`. They change system integration, install code, or open a browser.
- With `VITE_ENABLE_UPDATES=false`, the Updates section shows store-managed update text and no check button. That is expected for the verification build.
- Quick actions lists `Remove Image Background` even when the Remove Background tool is unavailable.
- Settings persist in the verification profile. Restore values the recipe changed.
