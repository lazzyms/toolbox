# Remove Password

## Sub-features

- Select PDFs or Office files through the native Tauri file dialog or drag-and-drop.
- Remove a password with the real password through the `remove_password` command.
- Supports PDF plus legacy and modern Word, Excel, and PowerPoint files: `.doc`, `.docx`, `.xls`, `.xlsx`, `.ppt`, and `.pptx`.
- Show per-file results and write collision-safe `-unlocked` outputs.

## How to get to it (user POV)

Launch Toolbox, then choose `All tools` → `Documents` → `Remove Password`.

## Driving it with Tauri desktop UI

Click the `Drag & Drop files here` area, choose a password-protected PDF or Office file, enter its password in `File Password`, and click `Remove Password`. Assert the result row reports a successful unlock and inspect the output without supplying a password.

## Gotchas

qpdf is required for protected PDFs. Office formats use the bundled native adapter and do not require LibreOffice or another external Office runtime. A wrong password must produce a visible failure without an unlocked output. Files stay on the local machine.
