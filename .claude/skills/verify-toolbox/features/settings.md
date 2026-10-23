# Settings

Settings is a full page with six sections: Appearance, Output location, Privacy, Updates, About and support, and Keyboard shortcuts. Changes save as you make them.

## Sub-features

- `theme` selects the Light or Dark appearance. Dark is the default.
- `output-location` saves results next to each original or always in one folder.
- `privacy-counts` controls anonymous usage counts.
- `update-check` checks the configured update endpoint and reports current, available, or failed state.
- `app-version` shows the version from the running Toolbox build.
- `support` shows the support QR code and the third-party notices.
- `shortcuts` lists keyboard shortcuts with a search field.

## How to get to it (user POV)

- Choose `Settings` at the bottom of the sidebar, or press `super+,`.
- The `Settings` navigation lists the six sections as checkboxes. The page heading is the chosen section's title.
- Choose `← All tools`, press `Escape`, or press `super+k` to leave. There is no `Close settings` button.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Record the selected theme, output location, and privacy switch before changing any of them.
- Do not run an update check unless the verification request includes an online check.

- **Open Settings.** Choose `Settings`. Confirm the six section checkboxes and the hint "Changes save as you make them."
- **Change appearance.** In Appearance, choose `Light` or `Dark` in the `Theme` radio group. The whole UI changes. Restore the starting theme.
- **Read the output location.** In Output location, the `Save results` radio group offers `Next to each original` and `Always in one folder`, with `Choose output folder`. Recipes that check files beside scratch inputs need `Next to each original`.
- **Read the privacy control.** In Privacy, read the `Share anonymous usage counts` switch. Checked means counts are shared. If the recipe changes it, restore the recorded state.
- **Check the version.** Updates shows "Version <v>." and About and support shows "Version <v> for macOS". Compare with `npm pkg get version` from `ToolboxTauri`.
- **Check for updates.** Only when the request permits network access, choose `Check now` and wait for the result. Do not choose installation or restart controls.
- **Read shortcuts.** Keyboard shortcuts has a `Search shortcuts` field and lists Everywhere and Batch tools shortcuts.

## Gotchas

- Theme, output location, and privacy choices persist in the verification app. Restore values changed by the recipe.
- `Check now` contacts the configured update endpoint. It is not a local-only check.
- An update status does not authorize installation. Stop before any download, install, or relaunch action.
- About and support contains a donation QR code and `Open notices`. Both are display-only for this recipe.
