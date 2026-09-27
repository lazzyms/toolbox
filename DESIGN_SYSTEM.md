# Toolbox design system

The design system lives in `ToolboxTauri/src/design-system/`. It gives the
desktop app a shared visual language and accessible controls for the command
center and workspaces.

## Tokens

`tokens.css` is the only place in application source where literal colors may
be defined. Use semantic custom properties in CSS and React styles instead of
adding hex, RGB, HSL, or named color values elsewhere. `npm run
check:design-system-colors` enforces this boundary, and `npm run check:release`
runs the check as part of the release gate.

Use these token groups:

- Surfaces: `--color-canvas`, `--color-rail`, `--color-panel`,
  `--color-card`, `--color-overlay`.
- Borders and text: `--color-border-subtle`, `--color-border-strong`,
  `--color-text-primary`, `--color-text-secondary`, `--color-text-muted`,
  `--color-text-disabled`.
- Actions and status: `--color-action` and its hover, active, disabled, ink,
  and subtle variants; `--color-focus` and `--color-focus-ring`; and the
  `--color-danger-*`, `--color-warning-*`, and `--color-success-*` tokens.
- Document and canvas: `--color-scene-*`, `--color-document-*`, and
  `--color-crop-shade` for content that is rendered on or around a document.
- Tool identity: `--color-tool-*` tokens for registry category tints.
- Type and layout: `--font-sans`, `--font-mono`, `--text-*`, `--tracking-*`,
  `--space-*`, and `--radius-*`.
- Elevation and motion: `--shadow-*`, `--motion-*`, and `--ease-standard`.

Prefer the semantic `--color-*` names in new code. Short aliases such as
`--bg`, `--surface`, `--fg`, `--muted`, and `--action` remain for existing
styles while they are migrated. Do not add new dependencies on those aliases.

The dark palette is the default. The existing theme preference maps the shared
semantic tokens for light mode; keep component styles token-based so palette
refinements do not require component rewrites. Document-page colors are
intentionally distinct from application surfaces.

## Components

Import components and types from the design-system barrel:

```tsx
import { Button, Panel, TextField } from "../design-system";

<Panel title="Recent files" description="Files stay on this device.">
  <TextField label="Search files" value={query} onChange={handleQuery} />
  <Button variant="primary" onClick={openFile}>Open file</Button>
</Panel>
```

Available controls:

- `Button`: `primary`, `ghost`, and `danger` variants; `sm`, `md`, and `lg`
  sizes. It defaults to `type="button"`; set `type="submit"` explicitly in
  forms.
- `IconButton`: requires an `aria-label`; supports neutral or danger tone,
  sizes, and optional `pressed` state.
- `Slider`: requires a visible label, numeric range, and change callback;
  `formatValue` controls the adjacent value text.
- `ColorSwatches`: labeled, single-selection color options. Give each option a
  human-readable label and use design tokens for its color value.
- `Toggle`: labeled switch with controlled `checked` state.
- `SegmentedControl`: labeled radio group with controlled value and arrow-key
  navigation. Disabled options are skipped.
- `TextField` and `NumberField`: labeled inputs with optional hint, error,
  leading/trailing adornment, hidden visual label, and forwarded input ref.
- `Select`: labeled native select with typed options, hint, error, and ref.

Available surfaces:

- `Pill` and `Badge`: compact status labels with neutral, info, success,
  warning, or danger tone.
- `Card`: a visual container, optionally rendered as `article`, `div`, or
  `section`. Its `interactive` prop adds visual states; it does not add
  activation semantics. For a card with child actions, keep the card as a
  container and give each action its own button or link.
- `Panel` and `Section`: titled, labeled regions with optional description,
  actions, and children.
- `Thumbnail`: image with required alt text, optional caption, and selected
  state.
- `Tooltip`: associates its content with a focusable child through
  `aria-describedby`.
- `EmptyState`: title, optional description, icon, and action.
- `Toolbar` and `ToolbarButton`: labeled toolbar region and compact icon
  controls. One enabled button is in the Tab order; use orientation-matched
  arrow keys to move among enabled buttons, with Home and End for the edges.
  Provide an accessible label for every toolbar button.
- `StatusBar`: footer-like status region for workspace state.

## Interaction and accessibility

- Keep controls controlled by their parent state; do not hide product state in
  the primitive layer.
- Supply a visible label unless the control is icon-only. Icon-only controls
  must have an accessible name.
- Preserve native disabled behavior. Do not simulate disabled controls with
  styling alone.
- Keyboard focus is visible. Keep `:focus-visible` outlines and do not remove
  them to match a screenshot.
- Keep status text readable without relying on color alone.
- Keep normal text at or above 4.5:1 contrast against the shared semantic
  surfaces in both palettes; the E2E suite checks muted text against each
  surface token.
- Honor reduced-motion settings and use the shared motion tokens for new
  transitions.
- Use `readColorToken` only when a native canvas or rendering API needs a
  computed color string; normal UI styling should consume CSS variables.

## Verification

From `ToolboxTauri/`, run `npm run check:design-system-colors` while working on
styles and `npm run check:release` before handoff. The primitive interaction
preview is covered by `tests/ui/design-system.spec.ts` and is included in
`npm run test:ui`.
