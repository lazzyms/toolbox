# Unlock PDF

## Sub-features

- Select PDFs through the native Tauri file dialog or drag-and-drop.
- Unlock with the real password through the `unlock_pdf` command.
- Show per-file results and write `-unlocked` outputs.

## How to get to it (user POV)

Launch Toolbox, then choose `Unlock PDF` in the PDF section of the sidebar.

## Driving it with Tauri desktop UI

Click the `Drag & Drop files here` area, choose a password-protected PDF, enter its password in `PDF Password`, and click `Unlock PDF`. Assert the result row reports `PDF Unlocked` and inspect the output with qpdf or a PDF reader.

## Gotchas

qpdf is required for protected PDFs. A wrong password must produce a visible failure without an unlocked output. Files stay on the local machine.
