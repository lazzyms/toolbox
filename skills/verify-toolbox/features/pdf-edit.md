# Edit PDF

## Sub-features

- Select a PDF through the native Tauri file dialog or drag-and-drop.
- Add text, notes, highlights, or shapes through the edit-mode controls.
- Limit edits to all pages or a comma-separated page list.

## How to get to it (user POV)

Launch Toolbox, then choose `All tools` → `PDF` → `Edit PDF`.

## Driving it with Tauri desktop UI

Choose a readable PDF, select an edit mode, enter text when applicable, optionally set `Edit pages`, and click `Edit`. Assert a successful `-edited.pdf` result, readable output, and unchanged source bytes.

## Gotchas

The current UI places text, notes, highlights, and shapes in a fixed preview rectangle; page selection is converted to zero-based native page indices. Verify the result rather than relying on the filename alone.
