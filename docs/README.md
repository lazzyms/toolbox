# Toolbox documentation

The public website is [index.html](index.html). It describes the current Tauri
release on `main`.

Current references:

- [PDF and document tools](pdf.html)
- [Image tools](images.html)
- [Tauri tool matrix](tauri-tool-matrix.md)
- [Tauri release readiness](tauri-release-readiness.md)
- [Offline vision engines](tauri-vision-engines.md)
- [Documentation checker](check-docs.mjs)

The parity specification, fidelity research, engine decision, and ADR files are
historical design records. Their opening notes identify that status. They remain
useful background, but the registry and source code define the current behavior.

Run the documentation check from the repository root:

```bash
node docs/check-docs.mjs
```

The checker compares the category pages with
`ToolboxTauri/src/registry/index.ts`, checks the matrix rows, validates local
links and screenshot assets, and verifies that downloads point directly to the
macOS release asset and the Microsoft Store listing for Windows.
