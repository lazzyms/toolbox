# Toolbox Tauri feature map

The primary surface is the cross-platform Tauri desktop window: a React/Vite frontend invokes Rust commands through Tauri IPC. Each feature is reached from the sidebar and shares the native file dialog, drag-and-drop area, and per-file result rows.

| Feature | Entry point | Proof of success |
| --- | --- | --- |
| [Unlock PDF](unlock-pdf.md) | PDF → Unlock PDF | `PDF Unlocked` result and an unencrypted `-unlocked` output |
| [Protect PDF](protect-pdf.md) | PDF → Protect PDF | `PDF Protected` result and an encrypted `-protected` output |
| [Compress images](compress-images.md) | Images → Compress | Successful result and a no-larger `-compressed` output |
| [Convert image format](convert-image-format.md) | Images → Convert | Successful result and valid output in the selected format |
