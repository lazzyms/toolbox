# Toolbox design system app migration

> Historical execution record. The user rejected the tinted component system on 2026-09-28 and directed a move to shadcn. This file no longer describes the current component system. See `.audit/shadcn-component-migration.md` for the active workflow.

Retired product contract: `DESIGN_SYSTEM.md` was removed when the user switched the shared component system to shadcn. This checklist and TSV are historical execution records, not current component rules or another design specification; the current shadcn policy is in `AGENTS.md` and its source checker.

## Architect phases

- [x] Ground: map shared shell, workspace scaffold, feature state, and existing E2E coverage.
- [x] Sketch: compare two adoption plans against the approved contracts.
- [x] Agree: select the synthesized plan and record scope.
- [x] Implement: migrate in verifiable slices, preserving feature state and local file behavior.
- [x] Scrap: skip; implementation friction did not repeat or require a boundary redesign.

## Arena phases

- [x] Frame: state artifact and concrete selection rubric.
- [x] Fan out: produce isolated candidate plans and rationales.
- [x] Cross-judge: grade each candidate against the same rubric.
- [x] Pick: choose the plan with the best maintainable boundary.
- [x] Graft: incorporate useful ideas and record rejections.
- [x] Verify: verify implementation and the actual rendered app.

## Definition of done

Every app shell, shared workspace, settings, and feature-family control uses the canonical contracts or remains a clearly specialized, token-backed editor surface. Existing file ordering, processing, result isolation, tool navigation, settings persistence, and editor keyboard/pointer behavior remain intact. The production build and existing UI end-to-end suites pass, and the running app preview is inspected in both themes.

## Work sequence

- [x] Shared workspace shell and result actions.
- [x] Application navigation, library, and settings.
- [x] Security, conversion, and media controls.
- [x] Image editor and PDF editor controls and token-backed specialized surfaces.
- [x] Build, E2E verification, visual inspection, and final diff review.

## Selected implementation sketch

Reuse the existing public controlled primitives at their current app seams. Keep `MainPage`, `SettingsPanel`, `ToolScaffold`, `WorkspaceCommandRail`, `ResultList`, and each feature view as the owner of its existing state and operation callbacks. Add no public component APIs and no shared workflow schema. Keep page selection, canvas gestures, and document-specific geometry as specialized native/UI elements styled with semantic tokens where the public contract does not match.

The two independent candidates converged on this shape. The cross-judge selected candidate 2 for its phase-specific E2E gates. Graft candidate 1's explicit invariants: preserve ordered inputs, cardinality and format validation, drag/drop and picker flows, run-generation guards, per-file outcomes, and stale output-action protection. Keep measured PDF canvas/page-strip/inspector bounds as a visual regression gate where the existing desktop fixture can measure them. Do not add candidate 2's illustrative `WorkspaceForm` as a shared type.

Keep the existing workspace command group as a toolbar and adopt the public `Toolbar` keyboard model rather than changing it to navigation semantics. The implementation already declares `role="toolbar"` and active commands with `aria-pressed`; align it with the contract by using vertical arrow navigation and Home/End, then cover this integration path in the existing UI E2E suite.

## Arena rubric and decision

1. Reuse the approved public contracts broadly without an unnecessary wrapper layer or second specification.
2. Preserve current state ownership, local file processing, and input/result semantics.
3. Sequence the migration into checkable slices with existing E2E gates.
4. Cover keyboard/accessibility behavior and both semantic color themes.
5. Protect specialized editor geometry, pointer/focus behavior, and async guards.

Candidate 2 is the base. The independent judge scored it 4/5 for contract reuse, 4/5 for state/data invariants, and 5/5 for phase gates, keyboard/theme coverage, and editor protection. Candidate 1 scored 5/5 for contract reuse, state preservation, and editor protection; 4/5 for E2E sequencing and keyboard/theme coverage. Both rejected a universal workspace form schema and a global CSS-only rewrite. Reject changing the command rail from toolbar to navigation because it contradicts the current exposed toolbar semantics and would evade the documented keyboard contract.

Implementation starts with tokens/shared workspace controls, then shell/settings, standard feature controls, image editor, and PDF editor. Check each slice with the relevant existing Playwright coverage. Finish with build, full UI E2E, semantic-color check, both-theme desktop inspection, and final diff review.

## Verified outcome

The shared shell, settings, workspace controls, security/media/conversion forms, and editor actions now use the canonical public primitives. Existing state owners, file ordering, selection gates, per-file outcomes, preview guards, and editor geometry remain in their original feature boundaries. Dense page selection, color picking, and drawing surfaces remain specialized and semantic-token backed.

The final Playwright UI run passed 110 tests and skipped 3 unavailable-tool cases. The production build, semantic-color check, and diff whitespace check passed. The PDF workspace was rendered and inspected in both palettes; repeatable captures are emitted by tests/ui/pdf-scene.spec.ts.
