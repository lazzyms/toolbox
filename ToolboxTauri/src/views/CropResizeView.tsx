import { useCallback, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CircleAlert, TriangleAlert } from "lucide-react";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import type { AtomicToolId, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { parsePageRanges } from "../shared/pageRanges";
import { shortcutTarget } from "../shortcuts";
import { capabilityFor } from "../registry/capabilities";
import { CropPreview, ResizePreview, usePageSizes, type CropTarget, type PageSizes, type ResizeTarget } from "../features/pdf-crop/CropPreview";
import {
  DEFAULT_RECT, cutsOff, describePage, fieldLength, papers, placePage, pointsPerUnit, readPaperSide, rectIssues, sameRect, showSize,
  type Margins, type Orientation, type PageSize, type Placement, type Rect, type RectField, type Unit,
} from "../features/pdf-crop/geometry";

type Mode = "crop" | "resize";
type CropBy = "rectangle" | "margins";
type Side = keyof Margins;

const sides: ReadonlyArray<{ key: Side; label: string }> = [
  { key: "top", label: "Top" },
  { key: "right", label: "Right" },
  { key: "bottom", label: "Bottom" },
  { key: "left", label: "Left" },
];

const ERROR_ID = "mar-err";
const RECT_ERROR_ID = "rect-err";
const PAGES_ERROR_ID = "crop-pages-err";
const PAGES_HELP_ID = "crop-pages-help";
const PAGES_FIX_REASON: Record<Mode, string> = { crop: "Fix the pages to crop.", resize: "Fix the pages to resize." };
const DEFAULT_PAPER = "a4";
/** The Page size option that shows Width and Height. */
const CUSTOM = "custom";
const SIZE_ERROR_ID = "size-err";
const SIZE_FIX_REASON = "Fix the page size to resize.";

type PaperSide = "width" | "height";
type TargetSize = { preset: string } | { custom: { width: number; height: number } };

const paperSides: ReadonlyArray<{ key: PaperSide; label: string }> = [
  { key: "width", label: "Width" },
  { key: "height", label: "Height" },
];

type SizeCheck =
  | { size: TargetSize; paper: PageSize; name: string; errors: [] }
  | { size: null; paper: null; name: null; errors: Array<{ side: PaperSide; message: string }> };

/** The size to send for the chosen preset or typed custom size, converted to points once, or why it can't be used. */
const checkSize = (paper: string, custom: Record<PaperSide, string>, unit: Unit): SizeCheck => {
  if (paper !== CUSTOM) {
    const preset = papers.find(({ id }) => id === paper)!;
    return { size: { preset: paper }, paper: preset, name: preset.name, errors: [] };
  }
  const errors: Array<{ side: PaperSide; message: string }> = [];
  const points = {} as Record<PaperSide, number>;
  for (const { key, label } of paperSides) {
    const read = readPaperSide(label, custom[key], unit);
    if ("error" in read) errors.push({ side: key, message: read.error });
    else points[key] = read.points;
  }
  return errors.length
    ? { size: null, paper: null, name: null, errors }
    : { size: { custom: points }, paper: points, name: showSize(points, unit), errors: [] };
};

/** Pages a typed scope covers in a PDF of this many pages, clamped to it; null when the scope can't be read. */
const pagesInScope = (text: string, pageCount: number): number[] | null => {
  if (!text.trim()) return Array.from({ length: pageCount }, (_, index) => index);
  const parsed = parsePageRanges(text);
  if (!parsed.ok) return null;
  const covered = new Set<number>();
  for (const { first, last } of parsed.value) {
    for (let page = first; page <= Math.min(last ?? pageCount, pageCount); page += 1) covered.add(page - 1);
  }
  return [...covered];
};

/** The pages in scope Resize places on the new paper and those already that paper, for one PDF. */
const resizePlan = (pages: readonly PageSize[], scopeText: string, paper: PageSize, orientation: Orientation, placement: Placement) =>
  (pagesInScope(scopeText, pages.length) ?? []).map((index) => placePage(pages[index], paper, orientation, placement));

const rectFields: ReadonlyArray<{ key: RectField; label: string; axis: "width" | "height" }> = [
  { key: "left", label: "Left", axis: "width" },
  { key: "top", label: "Top", axis: "height" },
  { key: "width", label: "Width", axis: "width" },
  { key: "height", label: "Height", axis: "height" },
];

/** A typed rectangle length, or why it can't be read. Range problems are left to `rectIssues`. */
const readLength = (label: string, text: string): { value: number } | { error: string } => {
  const trimmed = text.trim();
  if (!trimmed) return { error: `${label} is empty.` };
  const value = Number(trimmed);
  return Number.isFinite(value) ? { value } : { error: `${label} must be a number.` };
};

const isTextField = (target: EventTarget) =>
  target instanceof HTMLElement && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));

type FieldCheck = { value: number } | { error: string };

const checkField = (label: string, text: string): FieldCheck => {
  const trimmed = text.trim();
  if (!trimmed) return { error: `${label} is empty; enter 0 for no margin.` };
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return { error: `${label} must be a number.` };
  if (value < 0) return { error: `${label} can't be less than 0.` };
  return { value };
};

type MarginCheck = { margins: Margins; errors: [] } | { margins: null; errors: Array<{ side: Side | "all"; message: string }> };

const checkMargins = (same: boolean, all: string, fields: Record<Side, string>, unit: Unit): MarginCheck => {
  const scale = pointsPerUnit[unit];
  if (same) {
    const check = checkField("All sides", all);
    if ("error" in check) return { margins: null, errors: [{ side: "all", message: check.error }] };
    const points = check.value * scale;
    return { margins: { top: points, right: points, bottom: points, left: points }, errors: [] };
  }
  const errors: Array<{ side: Side; message: string }> = [];
  const margins = {} as Margins;
  for (const { key, label } of sides) {
    const check = checkField(label, fields[key]);
    if ("error" in check) errors.push({ side: key, message: check.error });
    else margins[key] = check.value * scale;
  }
  return errors.length ? { margins: null, errors } : { margins, errors: [] };
};

type CropSpec = ({ kind: "margins" } & Margins) | ({ kind: "rectangle" } & Rect);

/** Every page, or the typed ranges exactly as entered; each PDF clamps them to its own length natively. */
type CropScope = "all" | { range: { ranges: string } };

/** The scope to send for the typed Pages text, or why it can't be read. */
const readScope = (text: string): { scope: CropScope } | { error: string } => {
  if (!text.trim()) return { scope: "all" };
  const parsed = parsePageRanges(text);
  return parsed.ok ? { scope: { range: { ranges: text } } } : { error: parsed.message };
};

const cropWith = (spec: CropSpec, scope: CropScope): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>("crop_pdf", {
    runId,
    request: { paths, spec, scope, outputLocation },
  });

/**
 * Resize sets every page in scope to a preset or custom paper, turned to match each page as displayed or forced
 * portrait or landscape, with the page scaled to fit or kept at actual size, centred. Only what Resize mode shows
 * is sent; Crop's values never are.
 */
const resizeWith = (size: TargetSize, orientation: Orientation, placement: Placement, scope: CropScope): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>("resize_pdf", {
    runId,
    request: { paths, size, orientation, placement, scope, outputLocation },
  });

const pdfCount = (count: number) => `${count} ${count === 1 ? "PDF" : "PDFs"}`;

const UnitInput = ({ id, label, unit, value, invalid, errorId = ERROR_ID, disabled, onChange, onFocus, onBlur }: {
  id: string;
  label: string;
  unit: Unit;
  value: string;
  invalid: boolean;
  errorId?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  onFocus?: () => void;
  onBlur?: () => void;
}) => (
  <div className="workspace-field">
    <Label htmlFor={id}>{label}</Label>
    <div className="workspace-unit-input">
      <Input
        id={id}
        inputMode="decimal"
        value={value}
        disabled={disabled}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errorId : undefined}
        onChange={(event) => onChange(event.target.value)}
        onFocus={onFocus}
        onBlur={onBlur}
      />
      <span aria-hidden="true">{unit}</span>
    </div>
  </div>
);

/** The rectangle being edited, its typed text and the rectangles Undo brings back. */
type RectState = {
  rect: Rect;
  drafts: Partial<Record<RectField, string>>;
  past: Rect[];
};

export const CropResizeView = ({ utility, initialPaths, onFilesChange, onSelectTool }: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  onFilesChange?: (paths: readonly string[]) => void;
  onSelectTool?: (id: AtomicToolId) => void;
}) => {
  const [mode, setMode] = useState<Mode>("crop");
  const [cropBy, setCropBy] = useState<CropBy>("rectangle");
  const [unit, setUnit] = useState<Unit>("mm");
  const [same, setSame] = useState(false);
  const [all, setAll] = useState("12");
  const [fields, setFields] = useState<Record<Side, string>>({ top: "15", right: "30", bottom: "15", left: "30" });
  const [rectState, setRectState] = useState<RectState>({ rect: DEFAULT_RECT, drafts: {}, past: [] });
  const [pagesText, setPagesText] = useState("");
  const [paper, setPaper] = useState(DEFAULT_PAPER);
  const [customSize, setCustomSize] = useState<Record<PaperSide, string>>({ width: "200", height: "250" });
  const [orientation, setOrientation] = useState<Orientation>("match");
  const [placement, setPlacement] = useState<Placement>("fit");
  const [files, setFiles] = useState<readonly string[]>([]);
  const sizes = usePageSizes(files);
  const trackFiles = useCallback((paths: readonly string[]) => {
    setFiles(paths);
    onFilesChange?.(paths);
  }, [onFilesChange]);
  // Resize runs its own command, so its rows, running label and output names come from resize_pdf.
  const resizeUtility = useMemo<ToolDefinition>(() => ({
    ...utility, command: "resize_pdf", capability: capabilityFor("resize_pdf"), resultVerb: "Resized", runningVerb: "Resizing",
  }), [utility]);
  const pages = readScope(pagesText);
  const check = checkMargins(same, all, fields, unit);
  const sizeCheck = checkSize(paper, customSize, unit);
  const invalidSides = new Set(check.errors.map((error) => error.side));

  const { rect, drafts, past } = rectState;
  const draftErrors = rectFields.flatMap(({ key, label }) => {
    const draft = drafts[key];
    const read = draft === undefined ? null : readLength(label, draft);
    return read && "error" in read ? [{ fields: [key], message: read.error }] : [];
  });
  const rectErrors = [...draftErrors, ...rectIssues(rect)];
  const invalidRectFields = new Set(rectErrors.flatMap((error) => error.fields));

  const spec: CropSpec | null = cropBy === "rectangle"
    ? rectErrors.length ? null : { kind: "rectangle", ...rect }
    : check.margins ? { kind: "margins", ...check.margins } : null;
  const fixReason = cropBy === "rectangle" ? "Fix the rectangle to crop." : "Fix the margins to crop.";

  const showRect = (next: Rect) => setRectState((current) => ({ ...current, rect: next, drafts: {} }));
  const record = (previous: Rect) => setRectState((current) => ({ ...current, past: [...current.past, previous] }));
  const undo = () => setRectState((current) => current.past.length
    ? { rect: current.past[current.past.length - 1], drafts: {}, past: current.past.slice(0, -1) }
    : current);
  const reset = () => setRectState((current) => sameRect(current.rect, DEFAULT_RECT) && !Object.keys(current.drafts).length
    ? current
    : { rect: DEFAULT_RECT, drafts: {}, past: [...current.past, current.rect] });

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="pdf-crop"
      utility={mode === "resize" ? resizeUtility : utility}
      initialPaths={initialPaths}
      onFilesChange={trackFiles}
      renderFileExtra={(path) => {
        // A PDF whose pages in scope are all already the new paper is skipped, as the native resize does.
        const pages = sizes.get(path);
        if (mode !== "resize" || !sizeCheck.paper || !Array.isArray(pages)) return null;
        const plan = resizePlan(pages, pagesText, sizeCheck.paper, orientation, placement);
        if (!plan.length || plan.some((page) => page !== null)) return null;
        return <span className="file-selection-note" data-testid="resize-file-note">Already {sizeCheck.name}. Will be skipped.</span>;
      }}
      onRun={mode === "resize"
        ? sizeCheck.size && "scope" in pages
          ? resizeWith(sizeCheck.size, orientation, placement, pages.scope)
          : async () => { throw new Error(sizeCheck.size ? PAGES_FIX_REASON.resize : SIZE_FIX_REASON); }
        : spec && "scope" in pages ? cropWith(spec, pages.scope) : async () => { throw new Error(spec ? PAGES_FIX_REASON.crop : fixReason); }}
    >
      {(props) => (
        <CropWorkspace
          {...props}
          utility={utility}
          sizes={sizes}
          mode={mode}
          onModeChange={setMode}
          cropBy={cropBy}
          onCropByChange={setCropBy}
          unit={unit}
          onUnitChange={setUnit}
          spec={spec}
          fixReason={fixReason}
          resize={{
            paper,
            onPaperChange: setPaper,
            custom: customSize,
            onCustomChange: (side, text) => setCustomSize((current) => ({ ...current, [side]: text })),
            check: sizeCheck,
            orientation,
            onOrientationChange: setOrientation,
            placement,
            onPlacementChange: setPlacement,
          }}
          pages={{ text: pagesText, error: "error" in pages ? pages.error : null, onChange: setPagesText }}
          onSelectTool={onSelectTool}
          rectangle={{
            state: rectState,
            errors: rectErrors.map((error) => error.message),
            invalid: invalidRectFields,
            show: showRect,
            record,
            undo,
            reset,
            canUndo: past.length > 0,
            canReset: !sameRect(rect, DEFAULT_RECT) || Object.keys(drafts).length > 0,
            type: (key, text, page) => setRectState((current) => {
              const read = readLength(key, text);
              const axis = rectFields.find((field) => field.key === key)!.axis;
              const next = "value" in read ? { ...current.rect, [key]: (read.value * pointsPerUnit[unit]) / page[axis] } : current.rect;
              return { ...current, rect: next, drafts: { ...current.drafts, [key]: text } };
            }),
            settle: (key) => setRectState((current) => {
              const draft = current.drafts[key];
              if (draft === undefined || "error" in readLength(key, draft)) return current;
              const { [key]: _settled, ...drafts } = current.drafts;
              return { ...current, drafts };
            }),
          }}
          margins={
            <section className="grid gap-3" aria-label="Margins">
              <div className="workspace-field-heading">
                <Label className="workspace-check" htmlFor="crop-same-margins">
                  <Checkbox id="crop-same-margins" checked={same} onCheckedChange={(checked) => setSame(checked === true)} />
                  <span>Same on all sides</span>
                </Label>
                <div className="workspace-check">
                  <Label htmlFor="crop-margin-unit">Unit</Label>
                  <NativeSelect id="crop-margin-unit" size="sm" value={unit} onChange={(event) => setUnit(event.target.value as Unit)}>
                    <option value="mm">mm</option>
                    <option value="in">in</option>
                    <option value="pt">pt</option>
                  </NativeSelect>
                </div>
              </div>
              {same ? (
                <div className="workspace-field-grid">
                  <UnitInput id="crop-margin-all" label="All sides" unit={unit} value={all} invalid={invalidSides.has("all")} onChange={setAll} />
                </div>
              ) : (
                <div className="workspace-field-grid">
                  {sides.map(({ key, label }) => (
                    <UnitInput
                      key={key}
                      id={`crop-margin-${key}`}
                      label={label}
                      unit={unit}
                      value={fields[key]}
                      invalid={invalidSides.has(key)}
                      onChange={(value) => setFields((current) => ({ ...current, [key]: value }))}
                    />
                  ))}
                </div>
              )}
              {check.errors.length > 0 && (
                <p id={ERROR_ID} role="alert" className="workspace-field-error">
                  <CircleAlert aria-hidden="true" />
                  <span>{check.errors.map((error) => error.message).join(" ")}</span>
                </p>
              )}
              <p className="workspace-note">Every page loses the same lengths. A page left with no area fails its PDF.</p>
            </section>
          }
          marginsTarget={check.margins ? { margins: check.margins } : null}
        />
      )}
    </ToolScaffold>
  );
};

type ResizeControls = {
  paper: string;
  onPaperChange: (paper: string) => void;
  custom: Record<PaperSide, string>;
  onCustomChange: (side: PaperSide, text: string) => void;
  check: SizeCheck;
  orientation: Orientation;
  onOrientationChange: (orientation: Orientation) => void;
  placement: Placement;
  onPlacementChange: (placement: Placement) => void;
};

type ScaffoldProps = Parameters<React.ComponentProps<typeof ToolScaffold>["children"]>[0];

type RectangleControls = {
  state: RectState;
  errors: string[];
  invalid: ReadonlySet<RectField>;
  show: (rect: Rect) => void;
  record: (previous: Rect) => void;
  undo: () => void;
  reset: () => void;
  canUndo: boolean;
  canReset: boolean;
  /** Takes typed text for one field, measured on the previewed page. */
  type: (key: RectField, text: string, page: PageSize) => void;
  /** Drops a field's typed text once it has been read, so it shows the rounded length again. */
  settle: (key: RectField) => void;
};

/** The preview and options for the files in the scaffold; owns what depends on the loaded PDFs. */
const CropWorkspace = ({
  files, run, loading, destination, selectedFileIndex, selectFile, utility, sizes, mode, onModeChange, cropBy, onCropByChange, unit, onUnitChange, spec, fixReason,
  resize, pages: pageScope, onSelectTool, rectangle, margins, marginsTarget,
}: ScaffoldProps & {
  utility: ToolDefinition;
  sizes: PageSizes;
  mode: Mode;
  onModeChange: (mode: Mode) => void;
  cropBy: CropBy;
  onCropByChange: (cropBy: CropBy) => void;
  unit: Unit;
  onUnitChange: (unit: Unit) => void;
  spec: CropSpec | null;
  fixReason: string;
  resize: ResizeControls;
  pages: { text: string; error: string | null; onChange: (text: string) => void };
  onSelectTool?: (id: AtomicToolId) => void;
  rectangle: RectangleControls;
  margins: React.ReactNode;
  marginsTarget: CropTarget | null;
}) => {
  const path = files[selectedFileIndex];
  const [shownPage, setShownPage] = useState<{ path: string | undefined; index: number }>({ path, index: 0 });
  const pages = path === undefined ? undefined : sizes.get(path);
  const pageCount = Array.isArray(pages) ? pages.length : 0;
  const pageIndex = shownPage.path === path ? Math.min(shownPage.index, Math.max(pageCount - 1, 0)) : 0;
  const page = Array.isArray(pages) ? pages[pageIndex] : undefined;
  const { rect, drafts } = rectangle.state;
  const byRectangle = cropBy === "rectangle";
  const focusStart = useRef<Rect | null>(null);

  const runLabel = mode === "crop"
    ? files.length === 0 ? "Crop PDFs" : `Crop ${pdfCount(files.length)}`
    : files.length === 0 ? "Resize PDFs" : `Resize ${pdfCount(files.length)}`;
  const pagesFixReason = PAGES_FIX_REASON[mode];
  const reason = files.length === 0
    ? mode === "crop" ? "Add PDFs to crop." : "Add PDFs to resize."
    : mode === "crop" && !spec ? fixReason
      : mode === "resize" && !resize.check.size ? SIZE_FIX_REASON
        : pageScope.error ? pagesFixReason : null;
  const invalidPaperSides = new Set(resize.check.errors.map((error) => error.side));
  // Pages in scope that actual size would cut off, across the PDFs whose page sizes are known.
  const resizePaper = resize.check.paper;
  const cutOffPages = mode === "resize" && resize.placement === "actualSize" && resizePaper
    ? files.reduce((count, file) => {
      const filePages = sizes.get(file);
      if (!Array.isArray(filePages)) return count;
      return count + resizePlan(filePages, pageScope.text, resizePaper, resize.orientation, resize.placement).filter(cutsOff).length;
    }, 0)
    : 0;
  const resizeTarget: ResizeTarget | null = resize.check.paper
    ? { paper: resize.check.paper, name: resize.check.name, orientation: resize.orientation, placement: resize.placement }
    : null;
  const target: CropTarget | null = byRectangle ? (rectangle.errors.length ? null : { rect }) : marginsTarget;

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!byRectangle || mode !== "crop" || isTextField(event.target)) return;
    if ((event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "z") {
      event.preventDefault();
      rectangle.undo();
    }
  };

  return (
    <div className="crop-workspace" onKeyDown={onKeyDown}>
      {mode === "crop" && files.length > 0 && (
        <CropPreview
          files={files}
          selectedFileIndex={selectedFileIndex}
          selectFile={selectFile}
          sizes={sizes}
          pageIndex={pageIndex}
          onPageChange={(index) => setShownPage({ path: files[selectedFileIndex], index })}
          target={target}
          invalidCaption={byRectangle ? "Fix the rectangle to see the crop." : "Fix the margins to see the crop."}
          unit={unit}
          editable={byRectangle && !loading}
          history={byRectangle ? { canUndo: rectangle.canUndo, canReset: rectangle.canReset, onUndo: rectangle.undo, onReset: rectangle.reset } : null}
          onRect={rectangle.show}
          onRecord={rectangle.record}
        />
      )}
      {mode === "resize" && files.length > 0 && (
        <ResizePreview
          files={files}
          selectedFileIndex={selectedFileIndex}
          selectFile={selectFile}
          sizes={sizes}
          pageIndex={pageIndex}
          onPageChange={(index) => setShownPage({ path: files[selectedFileIndex], index })}
          target={resizeTarget}
          unit={unit}
        />
      )}
      <Card className="workspace-control-panel py-0">
        <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{utility.title}</h2>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              aria-label="Mode"
              value={mode}
              onValueChange={(value) => {
                if (value) onModeChange(value as Mode);
              }}
            >
              <ToggleGroupItem value="crop">Crop pages</ToggleGroupItem>
              <ToggleGroupItem value="resize">Resize pages</ToggleGroupItem>
            </ToggleGroup>
            <p className="workspace-panel-copy">
              {mode === "crop" ? "Trim the edges of every page." : "Change the paper size of every page."}
            </p>
            <p className="workspace-note">Accepts .pdf</p>
          </div>
          {mode === "crop" && (
            <>
              <div className="grid gap-2">
                <Label id="crop-by-label">Crop by</Label>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-labelledby="crop-by-label"
                  value={cropBy}
                  onValueChange={(value) => {
                    if (value) onCropByChange(value as CropBy);
                  }}
                >
                  <ToggleGroupItem value="rectangle">Rectangle</ToggleGroupItem>
                  <ToggleGroupItem value="margins">Margins</ToggleGroupItem>
                </ToggleGroup>
              </div>
              {byRectangle ? (
                <section className="grid gap-3" aria-label="Rectangle">
                  <div className="workspace-field-grid">
                    {rectFields.map(({ key, label, axis }) => (
                      <UnitInput
                        key={key}
                        id={`crop-rect-${key}`}
                        label={label}
                        unit={unit}
                        value={!page ? "" : drafts[key] ?? fieldLength(rect[key] * page[axis], unit)}
                        disabled={!page}
                        invalid={rectangle.invalid.has(key)}
                        errorId={RECT_ERROR_ID}
                        onFocus={() => { focusStart.current = rect; }}
                        onChange={(text) => { if (page) rectangle.type(key, text, page); }}
                        onBlur={() => {
                          rectangle.settle(key);
                          if (focusStart.current && !sameRect(focusStart.current, rect)) rectangle.record(focusStart.current);
                          focusStart.current = null;
                        }}
                      />
                    ))}
                  </div>
                  {page && rectangle.errors.length > 0 && (
                    <p id={RECT_ERROR_ID} role="alert" className="workspace-field-error">
                      <CircleAlert aria-hidden="true" />
                      <span>{rectangle.errors.join(" ")}</span>
                    </p>
                  )}
                  <p className="workspace-note" data-testid="crop-rect-hint">
                    {files.length === 0
                      ? "Add PDFs, then draw the rectangle on the preview or type it here."
                      : "Drag on the preview, or type. Arrow keys move it 1 mm. Shift+Arrow resizes it."}
                  </p>
                </section>
              ) : margins}
              <div className="workspace-note grid justify-items-start gap-1" data-testid="crop-hidden-note">
                <span>Crop hides the rest of the page. It stays in the file and can be shown again.</span>
                <Button variant="link" size="sm" className="h-auto p-0" onClick={() => onSelectTool?.("pdf-redact")}>
                  Remove content with Redact PDF
                </Button>
              </div>
            </>
          )}
          {mode === "resize" && (
            <>
              <div className="grid gap-3">
                <div className="workspace-field">
                  <Label htmlFor="psize">Page size</Label>
                  <NativeSelect id="psize" value={resize.paper} onChange={(event) => resize.onPaperChange(event.target.value)}>
                    {papers.map(({ id, width, height }) => <option key={id} value={id}>{describePage({ width, height }, "mm")}</option>)}
                    <option value={CUSTOM}>Custom size</option>
                  </NativeSelect>
                </div>
                {resize.paper === CUSTOM && (
                  <section className="grid gap-3" aria-label="Custom size">
                    <div className="workspace-field-heading justify-end">
                      <div className="workspace-check">
                        <Label htmlFor="resize-size-unit">Unit</Label>
                        <NativeSelect id="resize-size-unit" size="sm" value={unit} onChange={(event) => onUnitChange(event.target.value as Unit)}>
                          <option value="mm">mm</option>
                          <option value="in">in</option>
                          <option value="pt">pt</option>
                        </NativeSelect>
                      </div>
                    </div>
                    <div className="workspace-field-grid">
                      {paperSides.map(({ key, label }) => (
                        <UnitInput
                          key={key}
                          id={`resize-size-${key}`}
                          label={label}
                          unit={unit}
                          value={resize.custom[key]}
                          invalid={invalidPaperSides.has(key)}
                          errorId={SIZE_ERROR_ID}
                          onChange={(text) => resize.onCustomChange(key, text)}
                        />
                      ))}
                    </div>
                    {resize.check.errors.length > 0 && (
                      <p id={SIZE_ERROR_ID} role="alert" className="workspace-field-error">
                        <CircleAlert aria-hidden="true" />
                        <span>{resize.check.errors.map((error) => error.message).join(" ")}</span>
                      </p>
                    )}
                  </section>
                )}
              </div>
              <div className="grid gap-2">
                <Label id="resize-orientation-label">Orientation</Label>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-labelledby="resize-orientation-label"
                  value={resize.orientation}
                  onValueChange={(value) => {
                    if (value) resize.onOrientationChange(value as Orientation);
                  }}
                >
                  <ToggleGroupItem value="match">Match page</ToggleGroupItem>
                  <ToggleGroupItem value="portrait">Portrait</ToggleGroupItem>
                  <ToggleGroupItem value="landscape">Landscape</ToggleGroupItem>
                </ToggleGroup>
              </div>
              <div className="grid gap-2">
                <Label id="resize-placement-label">Placement</Label>
                <RadioGroup value={resize.placement} onValueChange={(value) => resize.onPlacementChange(value as Placement)} aria-labelledby="resize-placement-label">
                  <Label className="workspace-check" htmlFor="resize-placement-fit">
                    <RadioGroupItem id="resize-placement-fit" value="fit" />
                    <span>Scale to fit, centered <span className="workspace-note">Nothing is cut off</span></span>
                  </Label>
                  <Label className="workspace-check" htmlFor="resize-placement-actual">
                    <RadioGroupItem id="resize-placement-actual" value="actualSize" />
                    <span>Actual size, centered <span className="workspace-note">Edges may be cut off</span></span>
                  </Label>
                </RadioGroup>
              </div>
              <p className="workspace-note">Links, comments and form fields move with the content. Text stays sharp.</p>
              {cutOffPages > 0 && (
                <p role="status" className="workspace-field-warning" data-testid="resize-cut-off">
                  <TriangleAlert aria-hidden="true" />
                  <span>Cuts off content on {cutOffPages} {cutOffPages === 1 ? "page" : "pages"}. Choose Scale to fit to keep everything.</span>
                </p>
              )}
            </>
          )}
          <div className="workspace-field">
            <Label htmlFor="crop-pages">Pages</Label>
            <Input
              id="crop-pages"
              value={pageScope.text}
              placeholder="All pages"
              aria-invalid={pageScope.error ? true : undefined}
              aria-describedby={pageScope.error ? `${PAGES_ERROR_ID} ${PAGES_HELP_ID}` : PAGES_HELP_ID}
              onChange={(event) => pageScope.onChange(event.target.value)}
            />
            {pageScope.error && (
              <p id={PAGES_ERROR_ID} role="alert" className="workspace-field-error">
                <CircleAlert aria-hidden="true" />
                <span>{pageScope.error}</span>
              </p>
            )}
            <p id={PAGES_HELP_ID} className="workspace-note">Empty means all pages. Pages past the end are ignored. A PDF with none left is skipped.</p>
          </div>
          {destination}
          <p className="workspace-note">Names end in {mode === "crop" ? "-cropped" : "-resized"}. A taken name gets -1, -2.</p>
          {reason && <p className={`workspace-note ${reason === fixReason || reason === pagesFixReason || reason === SIZE_FIX_REASON ? "workspace-note-error" : ""}`} data-testid="crop-run-reason">{reason}</p>}
          <Button
            variant="default"
            aria-label={runLabel}
            disabled={loading || reason !== null}
            onClick={run}
            className="workspace-primary-action"
            {...shortcutTarget("run")}
          >
            {runLabel}
            <ShortcutHint id="run" />
          </Button>
        </CardContent>
      </Card>
    </div>
  );
};
