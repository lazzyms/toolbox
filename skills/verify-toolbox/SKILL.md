---
name: verify-toolbox
description: "Drive and verify the Toolbox Tauri desktop app through its native window. Use when checking desktop behavior or investigating a reported UI issue."
---

# Verify Toolbox

Use this skill to verify the native Toolbox window. Read the [feature map](./features/README.md) and the matching workspace recipe before you drive the app.

The Playwright UI specs under `ToolboxTauri/tests/ui/` replace Tauri IPC with test stubs. They can verify frontend behavior, but they do not prove native dialogs, commands, or files written by the real app.

## Launch

From `ToolboxTauri`, build the verification app:

```sh
VITE_ENABLE_UPDATES=false npm run tauri -- build --debug --config ../skills/verify-toolbox/tauri.verify.json
```

`VITE_ENABLE_UPDATES=false` compiles the build without the updater, so a run can never check for or install an update. The Updates section in Settings then shows store-managed update text instead of a check button.

Open the bundle with local helper paths so the PDF, OCR, and image features can produce output. On macOS, install the helpers with `brew install qpdf poppler tesseract`:

```sh
app_path="$PWD/src-tauri/target/debug/bundle/macos/Toolbox Verification.app"
b="$(brew --prefix)"
open -n \
  --env TOOLBOX_QPDF_PATH="$b/bin/qpdf" \
  --env TOOLBOX_PDFTOPPM_PATH="$b/bin/pdftoppm" \
  --env TOOLBOX_PDFTOTEXT_PATH="$b/bin/pdftotext" \
  --env TOOLBOX_TESSERACT_PATH="$b/bin/tesseract" \
  --env TOOLBOX_TESSDATA_PATH="$b/share/tessdata/eng.traineddata" \
  "$app_path"
```

The config names this bundle `Toolbox Verification` and assigns it the identifier `com.toolbox.desktop.verify`, so its app data stays apart from the installed `com.toolbox.desktop` app. Its window title is `Toolbox`, or `Toolbox — <workspace>` inside a workspace. It does not use or stop the preview on port 1420.

The app is ready when `lsof -t "$app_path/Contents/MacOS/toolbox"` prints one process ID and that process owns a window titled `Toolbox`. Record the process ID in `evidence/app.pid`. Drive only that process.

## Doctor

Run this read-only check from `ToolboxTauri` before driving the app:

```sh
test "$(node -p "require('./package.json').version")" = "$(node -p "JSON.parse(require('node:fs').readFileSync('src-tauri/tauri.conf.json', 'utf8')).version")"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")" = "com.toolbox.desktop.verify"
app_pid="$(lsof -t "$app_path/Contents/MacOS/toolbox")"
test -n "$app_pid"
```

Then read the window with CUA using that process ID and an accessibility timeout of 8 to 15 seconds. The WebView tree is large, and the default timeout can fail. Open **Settings** and compare its `Version <x> for macOS` text with `npm pkg get version`. Choose **All tools** to leave Settings. The verification build needs no account or sign-in.

Run Doctor again after any drive that fails or leaves an unexpected dialog open.

This launch recipe targets macOS. The Windows and Linux bundle paths are not mapped here. Do not install an MSI or drive an installed app as a substitute.

## Drive

Use CUA desktop control against the native verification window. Get a fresh accessibility tree before each action and use visible control names. Do not call React setters, write page storage, or use the UI test IPC stubs.

### Navigation

- The home rail lists `All tools`, `Favorites`, one button per workspace (`File Security`, `PDF Editor`, `PDF Conversion`, `Image Editor`, `Compress Files`, `Media`), and `Settings`.
- The home **Tool library** has an `Open <workspace>` button and `Open <tool>` buttons for each workspace. Long lists end with a button such as `8 more tools`.
- Inside a workspace, the rail shows only `← All tools` and `Settings`. Choose `← All tools` before you choose a different workspace.
- Each workspace has a toolbar of tool checkboxes. Choose a toolbar item to switch tools without losing the chosen files.
- `Search tools` matches one substring. Type a single word such as `merge` or `extract`. A multi-word query such as `extract pages` shows `No matching tools`.

### Choosing files

Choose `Choose files to process` and drive the native open panel:

1. Take the topmost `Open and Save Panel Service` window. Its process is not the Toolbox process.
2. Send `cmd+shift+g` and then `cmd+a` in foreground mode to that panel window, and type the absolute fixture path.
3. Read the panel's `PathTextField` and confirm that it holds the path.
4. Send `return` to the untitled Toolbox window that hosts the sheet.
5. Press the sheet's `OKButton`.

Each choice adds to the list. A workspace keeps one file list for all its tools, so the list shows `N file(s) open`, `Clear list`, and `Remove from list`. Choose `Clear list` before a tool that needs different inputs. A file that the current tool cannot use is flagged and stays in the list. Toolbox shows `Remove 1 flagged file` for it.

If a dialog asks to replace an existing file, choose `Cancel`. Never overwrite a fixture.

### Typing into fields

Click the field by name, send `cmd+a` in foreground mode, and type the value. The typing tool can report the text as unverified when it landed, so read the field before you retype. A cleared numeric field keeps a `0`, so typing `128` can show `0128`. Toolbox reads that as 128. Press `Tab` to commit a field before you run.

### Running and results

Each tool names its run button after the action and the file count, such as `Lock 1 file`, `Crop 2 PDFs`, or `Export Merge PDF`. The workspace recipes list them.

Wait for a status that starts with `Done.`, such as `Done. 2 merged` or `Done. 0 unlocked, 1 failed`. Do not wait on `Save the`, which also appears in help text. Each result row shows the output name, a detail line, and a status such as `Converted` or `Rotated`.

Outputs go next to each original by default. `Change for this run` changes the folder for one run. Each tool's example line shows the output name and states that an existing name gets `-1`, `-2`, and so on.

Tools that write one output from many files open an in-app save dialog. Its heading starts with `Save the`, and it has `File name`, a `Save to` path, `Change…`, `Cancel`, and `Save`. Confirm that `Save to` is the run's `scratch/` directory before you choose `Save`.

### Never choose

Do not choose `Open <output>`, `Show in Finder`, `Show all in Finder`, `Sponsor on GitHub`, `Install from file…`, any pack install control, or the Quick actions checkboxes in Settings. They open other apps, contact the network, or change system integration.

## Fixtures

Create every fixture inside the run's `scratch/` directory and hash them before you drive:

- **Image.** Copy `ToolboxTauri/src-tauri/icons/icon.png`, and convert a copy to JPEG with `sips -s format jpeg`.
- **PDF.** Write a small multi-page text PDF with a script, or make one with **Images to PDF**. Use at least three pages so page tools have a range to work on.
- **Office.** Write a Markdown note, an HTML page, and a valid one-sheet `.xlsx` for **Convert to PDF**.
- **TIFF.** Make a two-page TIFF with `tiffutil -cat` from two single-page TIFFs.
- **GIF.** Make an animated GIF with **Create GIF**, then feed it to **Extract GIF Frames**.
- **Password.** Lock a PDF copy with **Lock & Unlock** and a disposable password, then use it for unlock checks.

## Evidence

Save each run under `.verification/verify-toolbox/<run-id>/`, with inputs and outputs in `scratch/` and proof in `evidence/`. Record the app version, each feature ID, the user action, and the resulting accessible state in `run.md`. Save the CUA accessibility tree of a result as `<feature-id>.aria.txt`.

Before you save any tree, remove lines that come from the macOS menu bar. The `Open Recent` menu lists personal file names. Do not quote personal file names in evidence or reports.

For file operations, confirm the output exists and can be read, check its format with a tool such as `qpdf --check`, `pdfinfo`, `sips -g`, or `file`, and compare fixture hashes before and after. Do not treat a success message or a Playwright stub response as proof of a native output.

## Cleanup

Restore favorites and settings that the run changed. Choose **Quit Toolbox Verification** from the macOS app menu. Confirm that `lsof -t "$app_path/Contents/MacOS/toolbox"` returns no process. Do not kill a process by name or stop a shared preview.

Check fixture hashes one last time, then remove the run's `scratch/` directory. Keep `evidence/` so the proof survives cleanup.

## Helpers

There are no helper scripts. `tauri.verify.json` is a Tauri configuration overlay. The Launch command shows how to use it.
