# Toolbox consolidation run

## Definition of done

The seven requested workspaces use the existing 32 atomic tool IDs, preserve
source-file and local-only invariants, and expose user-visible workspace
behavior that is backed by focused UI/native verification. PDF and image
editors use typed, non-destructive plans. Preview and export consume the same
plan revision. Aggregate operations remain distinct from per-file jobs.

## Chosen architecture

Use the existing `ToolScaffold` as the thin lifecycle, file, loading, output,
and result owner. Add a small declarative action projection for routing and
input policy. Keep PDF and image session state domain-specific, with immutable
history and normalized plans. Do not introduce a universal editor or an
operation graph.

## Implementation sequence

1. Establish typed plan and history seams, shared validation, ordered input
   support, stale-run protection, and the action projection.
2. Complete the PDF editor vertical slice, including viewer controls, overlays,
   history, and a composed native export.
3. Complete the image editor vertical slice, including a live preview,
   non-destructive operation stack, history, and one combined export.
4. Move OCR into the PDF conversion workspace and add page selection plus
   output preview controls.
5. Add ordered GIF/TIFF media controls, icon presets, explicit metadata modes,
   and format-aware security copy and validation.
6. Remove obsolete standalone view imports and add a source-level consolidation
   check for the 32-tool matrix.
7. Run focused gates, the full native/UI/release matrix, and actual development
   Tauri desktop verification. Report any unavailable surface as unverified.

## Explicit boundaries

- Existing atomic IDs and verification IDs remain stable.
- Existing native commands remain available. Composed editor commands are
  internal workspace commands and do not create additional atomic tools.
- PDF page geometry and image pixel transforms remain separate domain types.
- Metadata inspection and metadata removal are explicit modes in the media
  workspace, not password/security operations.
- Office password removal remains available only where the native adapter
  supports it. The UI must not imply Office protection when only PDF
  protection is implemented.
- No raw files, reusable profiles, or processing results are persisted.
