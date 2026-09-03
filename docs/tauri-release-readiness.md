# Tauri release readiness

Run `npm run check:release` from `ToolboxTauri` before a release rehearsal. It verifies that the 31-tool registry is complete, every configured bundle icon exists, all Tauri targets are enabled, and updater metadata is present.

The CI workflows remain the authority for platform builds. The signed release workflow requires the macOS and Windows updater signing secrets and exercises both artifact paths. Linux is intentionally not advertised until a supported packaging and updater policy is chosen.

All file names shown in the React surface split both `/` and `\\`, while the native output naming layer keeps collision-safe paths. The file drop surface is keyboard operable and announces result updates through an `aria-live` region.
