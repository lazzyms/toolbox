# Protect PDF

## Sub-features

- Select plain PDFs through the native Tauri file dialog or drag-and-drop.
- Encrypt with a user-entered password through the `protect_pdf` command.
- Show per-file results and write `{stem}-protected.pdf` outputs.

## How to get to it (user POV)

Launch Toolbox, press the `PDF` category, then open the `Protect PDF` card.

## Driving it with Tauri desktop UI

Choose a plain PDF, enter a password in `Encryption Password`, and press `Protect PDF` (the last match; the sidebar has the same label). Assert the result row reports `PDF Protected with verified AES-256 encryption`, then verify the output is encrypted and opens with the password.

## Gotchas

qpdf must be on PATH during development or bundled beside the release executable. Use a disposable fixture password.
