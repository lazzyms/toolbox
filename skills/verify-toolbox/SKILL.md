---
name: verify-toolbox
description: "Drive and verify the Toolbox Tauri desktop app through its native window. Use when checking desktop behavior or investigating a reported UI issue."
---

# Verify Toolbox

Use this skill to verify the native Toolbox window. Read the [feature map](./features/README.md) and the matching feature recipe before you drive the app.

The Playwright UI specs under `ToolboxTauri/tests/ui/` replace Tauri IPC with test stubs. They can verify frontend behavior, but they do not prove native dialogs, commands, or files written by the real app.

## Launch

From `ToolboxTauri`, build the verification app and open its separate macOS bundle:

```sh
npm run tauri -- build --debug --config ../skills/verify-toolbox/tauri.verify.json
open -n -W "src-tauri/target/debug/bundle/macos/Toolbox Verification.app"
```

The config names this bundle `Toolbox Verification`, assigns it the identifier `com.toolbox.desktop.verify`, and disables updater artifact signing for this local build. Its window title remains `Toolbox`. The separate bundle keeps its app data apart from the installed `com.toolbox.desktop` app. It does not use or stop the preview on port 1420.

Keep the launch terminal attached and record its session ID. The app is ready when the native window titled `Toolbox` appears and CUA lists the running app as `com.toolbox.desktop.verify`.

## Doctor

Run this read-only check from `ToolboxTauri` before driving the app:

```sh
test "$(node -p "require('./package.json').version")" = "$(node -p "JSON.parse(require('node:fs').readFileSync('src-tauri/tauri.conf.json', 'utf8')).version")"
app_path="$PWD/src-tauri/target/debug/bundle/macos/Toolbox Verification.app"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")" = "com.toolbox.desktop.verify"
app_pid="$(lsof -t "$app_path/Contents/MacOS/toolbox")"
test -n "$app_pid"
```

Then bind CUA to `com.toolbox.desktop.verify` and read the `Toolbox` window. Its webview URL must be `tauri://localhost`. Open **Settings** and compare its **Version** value with `npm pkg get version`. Close Settings after the check. The verification build needs no account or sign-in.

This launch recipe targets macOS. The Windows and Linux bundle paths are not mapped here. Do not install an MSI or drive an installed app as a substitute.

## Drive

Use CUA desktop control against the native verification window. Bind `com.toolbox.desktop.verify`, then confirm the title and `tauri://localhost` URL. Get a fresh accessibility tree before each action and use the visible control name, such as `Search tools` or `Choose files to process`. Do not call React setters, write page storage, or use the UI test IPC stubs.

For a command-center search, start from the home view and use the visible search field or its `super+k` shortcut on macOS. On Windows or Linux, use `ctrl+k`. Type a tool name and confirm its `Open <tool>` action in the updated accessibility tree. The [command center recipe](./features/command-center.md) covers the filters, favorites, recent tools, and quick access entry points too.

For a file workflow, open the named tool through the command center and choose a disposable copy of a supported fixture through the native file picker. Use the active workspace's visible controls. Verify both the result shown by Toolbox and each output file on disk. The [PDF](./features/pdf-workflows.md), [image](./features/image-workflows.md), and [file security](./features/file-security.md) recipes list their entry points and result checks.

## Evidence

Save each run under `.verification/verify-toolbox/<run-id>/evidence/`. Record the app version, the feature ID, the user action, and the resulting accessible state in `run.md`. Save the full CUA accessibility tree as `<feature-id>.aria.txt`. Capture a CUA screenshot of the same result when the host can save screenshots.

Drive the real user path. Capture the action and the resulting state, not only the final screen. For file operations, confirm the output exists, can be read, and leaves the input unchanged. Do not treat a success message or a Playwright stub response as proof of a native output. Use mocks only at production boundaries that already isolate external systems.

The checked-in image at `ToolboxTauri/src-tauri/icons/icon.png` can seed a disposable image workflow. Copy it into the run's `scratch/` directory first. The repository does not include a checked-in PDF sample. To make one, use Toolbox's **Images to PDF** action on a scratch copy of that image, then use the resulting PDF as a fixture. Keep all outputs under the run's scratch directory.

## Cleanup

Choose **Quit Toolbox Verification** from the macOS app menu. Wait for the `open -W` launch command to return. Confirm that `lsof -t "$app_path/Contents/MacOS/toolbox"` returns no process. Do not kill a process by name or stop a shared preview.

Remove the run's `scratch/` directory after checking its outputs. Keep `.verification/verify-toolbox/<run-id>/evidence/` so the proof survives cleanup. The verification app has its own bundle identifier and persistent app data. Restore preferences changed by a recipe.

## Helpers

There are no helper scripts. `tauri.verify.json` is a Tauri configuration overlay. The Launch command shows how to use it.
