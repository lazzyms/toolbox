# Remove Password

## Sub-features

- Select PDFs or Office files through the native Tauri file dialog or drag-and-drop.
- Remove a password with the real password through the `remove_password` command.
- Supports PDF plus legacy and modern Word, Excel, and PowerPoint files: `.doc`, `.docx`, `.xls`, `.xlsx`, `.ppt`, and `.pptx`.
- Show per-file results and write collision-safe `-unlocked` outputs.

## How to get to it (user POV)

Launch Toolbox, press the `Documents` category, then open the `Remove Password` card.

## Driving it with Tauri desktop UI

Press `Choose files to process` (the `Drag & Drop files here` area), choose a password-protected PDF or Office file, enter its password in `File Password`, and press `Remove Password` (the last match; the sidebar has the same label). Assert the result row reports `PDF Unlocked and verified` and a `{stem}-unlocked.pdf` output for a PDF, and inspect the output without supplying a password.

## Gotchas

qpdf is required for protected PDFs. Office formats use the bundled native adapter and do not require LibreOffice or another external Office runtime. A wrong password must produce a visible failure (`Wrong password or unsupported encryption: invalid password for encrypted PDF`) without an unlocked output. Files stay on the local machine.
