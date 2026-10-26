# Remove Password

## Sub-features

- Select PDFs or Office files through the native Tauri file dialog or drag-and-drop.
- Remove a password with the real password through the `remove_password` command.
- Supports PDF plus legacy and modern Word, Excel, and PowerPoint files: `.doc`, `.docx`, `.xls`, `.xlsx`, `.ppt`, and `.pptx`.
- Show per-file results and write collision-safe `-unlocked` outputs.

## How to get to it (user POV)

Launch Toolbox, then click `Documents` under Categories in the sidebar and open the card `Open Remove Password`.

## Driving it with Tauri desktop UI

Click the `Drag & Drop files here` area, choose a password-protected PDF or Office file, enter its password in `File Password`, and click `Remove Password`. Assert `1 files completed` and the detail `PDF Unlocked and verified` (or `Office document unlocked and verified`) and inspect the output without supplying a password.

## Gotchas

PDF unlock uses the in-process `lopdf` reader and does not need qpdf. A wrong PDF password fails with `Wrong password or unsupported encryption: ...`. Office formats use the bundled native adapter and do not require LibreOffice or another external Office runtime. A wrong password must produce a visible failure without an unlocked output. Files stay on the local machine.
