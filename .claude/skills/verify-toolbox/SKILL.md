---
name: verify-toolbox
description: "Drive and verify the Toolbox Tauri desktop app through its native macOS window with CUA desktop control. Use when proving desktop behavior end to end, checking files the real app writes, or reproducing a reported UI issue."
---

# Verify Toolbox

Use this skill to verify the native Toolbox window. Read the [feature map](./features/README.md) and the matching feature recipe before you drive the app.

The Playwright UI specs under `ToolboxTauri/tests/ui/` replace Tauri IPC with test stubs. They can verify frontend behavior, but they do not prove native dialogs, commands, or files written by the real app.

Pick a run id like `20261020-222027-file-security` and set `run=.verification/verify-toolbox/<run-id>` at the repo root. `.verification/` is gitignored.

## Launch

From `ToolboxTauri`, build the verification bundle once per code change and open it:

```sh
npm run tauri -- build --debug --config ../.claude/skills/verify-toolbox/tauri.verify.json
open -n -W "src-tauri/target/debug/bundle/macos/Toolbox Verification.app"
```

Run the `open -W` line as a background task and keep its id. It returns when the app quits.

The config names the bundle `Toolbox Verification` with identifier `com.toolbox.desktop.verify` and disables updater signing. Its window title stays `Toolbox`. App data stays apart from the installed `com.toolbox.desktop` app. It does not touch the preview on port 1420.

The app is ready when `list_windows({app:"Toolbox Verification"})` returns a window titled `Toolbox` and its tree shows the `Search tools` field.

## Doctor

Run the read-only check from anywhere in the repo:

```sh
.claude/skills/verify-toolbox/helpers/doctor.sh
```

It checks that `package.json` and `tauri.conf.json` agree, the bundle exists with the verification identifier and the same version, and exactly one verification process runs. AppKit helpers that map the binary after an Open panel, such as `openAndSavePanelService`, are not counted. Success prints `doctor: OK version=<v> pid=<pid>`. Any failure names the check and exits non-zero. Use that pid for CUA calls. The build needs no account or sign-in.

This recipe targets macOS only. Do not install an MSI or drive an installed app as a substitute.

## Drive

Make disposable inputs first:

```sh
.claude/skills/verify-toolbox/helpers/make-fixtures.sh "$run"
```

It writes `plain.pdf`, a permissions-only `restricted.pdf`, a `legacy.doc` stub and `inputs.sha256` into `$run/scratch/`, then prints their absolute paths. It needs `qpdf` and `openssl`.

Point the native Open panel at scratch before you open it:

```sh
defaults delete com.toolbox.desktop.verify NSOSPLastRootDirectory 2>/dev/null || true
defaults write com.toolbox.desktop.verify NSNavLastRootDirectory -string "$(cd "$run/scratch" && pwd)"
```

Then drive with CUA in the background:

- Use one short session label on every call.
- Read with `get_window_state` and act by element token or by name in `run_steps`, for example `{"click":{"role":"button","name":"Unlock 1 file"}}`.
- Use an `expect` with `gone:true` to wait for progress text to clear.
- Press `Choose files to process` by name. The Open panel is a separate window of the app titled `Open`. Its tree is empty and window-scoped screenshots of it go stale, so pick with desktop clicks:
  1. `list_windows({pid})` for the panel's window id.
  2. `bring_to_front(pid, panel window id)`. Without it another app can sit on top and take the click.
  3. `get_desktop_state`, then `click({scope:"desktop", x, y})` on the file's icon. Pass x,y in that screenshot's frame; the tool scales them. Do not double them.
  4. Take a fresh `get_desktop_state` to confirm the selection and an enabled `Open` button, then desktop-click `Open`.
  Multi-select in the panel has not worked. Drive multi-file batches one file per open, or record them as source-verified.
- Never send keystrokes into the Open panel, foreground or background. They do not land, and foreground keystrokes can reach another app.
- For `type_text` inside `run_steps`, target the field with `element_token`. It can misreport delivery, so re-read the field before trusting it.
- A webview `<select>` (an `AXPopUpButton`) rejects `set_value`. That failed call opens its menu; then `press_key` down or up and `return` picks the option. Re-read the value.

Never browse the user's own folders in the panel. If it opens anywhere but scratch, cancel and reset the default above.

Do not call React setters, write page storage, or use the UI test IPC stubs.

## Evidence

Save each run under `$run/evidence/`:

- `run.md` records the app version and commit, the feature id, the doctor line, each user action, the result shown in the app and the disk checks with their output.
- `<feature-id>.aria.txt` holds the accessibility tree of the result state.
- `<feature-id>.png` holds a CUA screenshot of the same state.

Drive the real user path. Capture the action and the resulting state, not only the final screen. For file operations, confirm the output exists and is valid with `qpdf --check` or the format's own reader, and confirm the inputs are unchanged with `(cd "$run/scratch" && shasum -a 256 -c inputs.sha256)`. A success message or a Playwright stub response is not proof of a native output. Never write a password value into evidence.

## Cleanup

```sh
# CUA: invoke_menu(pid, window_id, ["Toolbox Verification", "Quit Toolbox Verification"])
# wait for the background open -W task to return
lsof -t "ToolboxTauri/src-tauri/target/debug/bundle/macos/Toolbox Verification.app/Contents/MacOS/toolbox" || echo "no pid"
defaults delete com.toolbox.desktop.verify NSNavLastRootDirectory 2>/dev/null || true
defaults delete com.toolbox.desktop.verify NSOSPLastRootDirectory 2>/dev/null || true
rm -rf "$run/scratch"
ls "$run/evidence"
```

Quit through the menu. Never kill by process name or stop a shared preview. The last line must still list the evidence files. Restore any preference a recipe changed, such as Favorites or theme.

## Helpers

- `helpers/doctor.sh` takes no arguments. It is read-only.
- `helpers/make-fixtures.sh <run-dir>` writes fixtures into `<run-dir>/scratch/`. The owner password on `restricted.pdf` is random and discarded.
- `tauri.verify.json` is the Tauri config overlay that the Launch command passes.
