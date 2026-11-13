# Settings

Settings is a full page with eight sections: Appearance, Output location, Quick actions, Packs, Privacy, Updates, About and support, and Keyboard shortcuts. Changes save as you make them.

## Sub-features

- `theme` selects the Light or Dark appearance. Dark is the default.
- `output-location` saves results next to each original or always in one folder.
- `quick-actions` lists the Finder right-click actions as `Right-click menu` checkboxes.
- `packs` lists optional packs and offers `Install from file…`.
- `privacy-counts` controls anonymous usage counts.
- `update-check` checks the configured update endpoint and reports current, available, or failed state.
- `app-version` shows the version from the running Toolbox build.
- `support` shows the `Sponsor on GitHub` button and the third-party notices.
- `shortcuts` lists keyboard shortcuts with a search field.

## How to get to it (user POV)

- Choose `Settings` at the bottom of the sidebar, or press `super+,`.
- The `Settings` navigation lists the eight sections as checkboxes. The page heading is the chosen section's title.
- Choose `← All tools`, press `Escape`, or press `super+k` to leave. There is no `Close settings` button.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Record the selected theme, output location, and privacy switch before changing any of them.

- **Open Settings.** Choose `Settings`. Confirm the eight section checkboxes and the hint "Changes save as you make them."
- **Change appearance.** In Appearance, choose `Light` or `Dark` in the `Theme` radio group. The whole UI changes. Restore the starting theme.
- **Read the output location.** In Output location, the `Save results` radio group offers `Next to each original` and `Always in one folder`, with `Choose output folder`. Recipes that check files beside scratch inputs need `Next to each original`.
- **Read the quick actions.** Quick actions lists `Right-click menu` checkboxes for Compress PDF, Convert PDF to Images, Compress Image, Convert Image to PNG, Convert Image to JPG and Remove Image Background. Read them; do not toggle them, because they change the Finder menu.
- **Read the packs.** Packs shows "Optional packs", `Install from file…` and "No packs installed." Do not install a pack.
- **Read the privacy control.** In Privacy, read the `Share anonymous usage counts` switch. Checked means counts are shared. If the recipe changes it, restore the recorded state.
- **Check the version.** Updates shows "Version <v>." and About and support shows "Version <v> for macOS". Compare with `npm pkg get version` from `ToolboxTauri`.
- **Check for updates.** With the verification config, `Check now` contacts only a dead local endpoint and reports "Could not check for updates". Do not choose installation or restart controls.
- **Read shortcuts.** Keyboard shortcuts says "Shown for the computer you are on.", has a `Search shortcuts` field and lists the Everywhere, Batch tools and PDF Editor groups.

## Gotchas

- Theme, output location, and privacy choices persist in the verification app. Restore values changed by the recipe.
- The verification config replaces the update endpoint with a dead local address. A build without that overlay contacts GitHub on `Check now` and checks again at launch.
- An update status does not authorize installation. Stop before any download, install, or relaunch action.
- About and support contains `Sponsor on GitHub` and `Open notices`. Both are display-only for this recipe.
- The window title reads `Toolbox — Settings` while Settings is open, and returns to the tool view's title when you leave.
