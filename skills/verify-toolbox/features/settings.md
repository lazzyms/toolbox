# Settings

Settings shows the app version and controls appearance, anonymous usage counts, and update checks.

## Sub-features

- `theme` selects the Dark or Light appearance.
- `app-version` shows the version from the running Toolbox build.
- `privacy-counts` controls anonymous usage counts.
- `update-check` checks the configured update endpoint and reports current, available, or failed state.
- `support` displays Toolbox support information.

## How to get to it (user POV)

- Choose `Settings` in Toolbox navigation.
- Choose `Close settings` or press `Escape` to return to the command center.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Record the selected theme and privacy toggle state before changing either one.
- Do not run an update check unless the verification request includes an online check.

- **Open Settings.** Choose `Settings`. The dialog heading is `Settings` and sections include Appearance, App info, Support Toolbox, Updates, and Privacy.
- **Check the version.** Read the `Version` value under App info and compare it with `npm pkg get version` from `ToolboxTauri`.
- **Change appearance.** Choose `Dark` or `Light` in the `Theme` control. The selected option and app appearance change. Restore the starting theme.
- **Check the privacy control.** Read the `Don’t send anonymous usage counts` switch state. If the recipe changes it, restore the recorded state before closing Settings.
- **Check for updates.** Only when the request permits network access, choose `Check for updates` and wait for the `role=status` result. Do not choose installation or restart controls.
- **Close Settings.** Choose `Close settings`. The command center returns.

## Gotchas

- Theme and privacy choices persist in the verification app. Restore values changed by the recipe.
- `Check for updates` contacts the configured update endpoint. It is not a local-only check.
- An update status does not authorize installation. Stop before any download, install, or relaunch action.
- The support section contains a QR code. It is display-only for this recipe.
