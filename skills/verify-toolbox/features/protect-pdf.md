# Protect PDF

## Sub-features

- Select plain PDFs through the native Tauri file dialog or drag-and-drop.
- Encrypt with a user-entered password through the `protect_pdf` command.
- Show per-file results and write `-protected` outputs.

## How to get to it (user POV)

Launch Toolbox, then choose `Protect PDF` in the PDF section of the sidebar.

## Driving it with Tauri desktop UI

Choose a plain PDF, enter a password in `Encryption Password`, and click `Protect PDF`. Assert the result row reports `PDF Protected`, then verify the output is encrypted and opens with the password.

## Gotchas

qpdf must be on PATH during development or bundled beside the release executable. Use a disposable fixture password.
