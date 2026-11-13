import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { TriangleAlert } from 'lucide-react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { DiscardGuard, RunDestinationControl, WorkspaceSourceAction } from '../components/ToolScaffold';
import { ExportDestination, ExportProgress } from '../components/ExportControls';
import type { FileProgress } from '../components/liveRun';
import { fileName } from '../components/resultRow';
import type { ShellEvent } from '../hooks/useShellBridge';
import type { AtomicToolId, ToolDefinition, ToolError, ToolResult } from '../contracts';
import { useToolAvailability } from '../ToolAvailabilityContext';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { RadioGroup, RadioGroupItem } from '../components/ui/radio-group';
import { ToggleGroup, ToggleGroupItem } from '../components/ui/toggle-group';
import { Label } from '../components/ui/label';
import { Slider } from '../components/ui/slider';
import type { PdfDocument, PdfEditRun, PdfFormEntry, PdfFormField, PdfFormInspection, PdfFormValue, PdfPage, PdfTextEdit, PdfTextRun } from '../features/pdf-editor/contracts';
import { formEntryFlag, isFormEntryEmpty } from '../features/pdf-editor/form-checks';
import { colorTokens, readColorToken } from '../features/pdf-editor/colorTokens';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { RedactDisclosure, RedactInspector, listMarks, redactionFills } from '../features/pdf-editor/RedactInspector';
import type { ListedMark } from '../features/pdf-editor/RedactInspector';
import { subToolInspectors } from '../features/pdf-editor/SubToolInspectors';
import type { SubToolInspectorProps, UsedSignature } from '../features/pdf-editor/SubToolInspectors';
import { combinedSuffix, exportSuffix, planSceneExport, sceneContributors, sceneSections } from '../features/pdf-editor/subtools';
import { proposedSaveName } from '../components/saveStep';
import type { SaveDialog } from '../registry/saveSteps';
import { movePagesBefore, objectsOnPage, pagesInRange, removePages, rotatePages, sceneFromDocument, swapPage, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, TextStyle } from '../features/pdf-editor/scene';
import { PREVIEW_CACHE_MAX_ENTRIES, PREVIEW_RENDER_SETTINGS, createPreviewCache, pdfEditorErrorMessage, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';
import type { GenerationState } from './pdfPreviewHelpers';
import { ShortcutHint } from '../components/ShortcutHint';
import { isPdfSceneTool } from '../registry';
import { pdfEditorBatchTools, type PdfEditorBatchTools } from '../features/pdf-editor/batchTools';
import { PdfEditorBatchSurface } from '../features/pdf-editor/BatchSurface';
import { PDFConversionWorkspaceView } from './PDFConversionWorkspaceView';
import { matchShortcut, pressShortcutTarget, shortcutLabel, shortcutTarget } from '../shortcuts';

export { createPreviewCache, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';

type IndexedScenePreview = { pageIndex: number; preview: ScenePreview };

const exportAsDialog: SaveDialog = { title: 'Export a copy', extension: 'pdf' };
const exportAsSummary = 'Saves a new copy with your edits. The original isn’t changed, and an existing file is never replaced.';

/** Organize shows the whole page grid. The other tools edit one page on the canvas. Both share one scene. */
type EditorTool = SceneTool | 'organize';
const tools: { id: EditorTool; label: string; glyph: string }[] = [
  { id: 'organize', label: 'Organize', glyph: '▦' },
  { id: 'select', label: 'Select', glyph: '↖' }, { id: 'edit-text', label: 'Edit text', glyph: '✎' }, { id: 'text', label: 'Text', glyph: 'T' },
  { id: 'highlight', label: 'Highlight', glyph: '▰' }, { id: 'shape', label: 'Shape', glyph: '□' },
  { id: 'signature', label: 'Signature', glyph: '〰' }, { id: 'form', label: 'Fill form', glyph: '☑' }, { id: 'watermark', label: 'Watermark', glyph: 'W' },
  { id: 'crop', label: 'Crop', glyph: '⌗' }, { id: 'redact', label: 'Redact', glyph: '▮' },
];
const toolFlagBySceneTool: Record<EditorTool, AtomicToolId> = {
  organize: 'pdf-organize', select: 'pdf-edit', 'edit-text': 'pdf-edit', text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', form: 'pdf-edit', watermark: 'pdf-watermark', crop: 'pdf-crop', redact: 'pdf-redact',
};
const toolFlagByObjectKind: Record<SceneObject['kind'], AtomicToolId> = {
  text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', watermark: 'pdf-watermark',
};
const toolGuidance: Record<EditorTool, string> = {
  organize: 'See every page in order.', select: 'Select, move, or resize an object.',
  'edit-text': 'Click text with a dashed outline to change it. Text that is part of a picture can\'t be edited. Use OCR first.',
  text: 'Drag to add text. Double-click text to edit it.',
  highlight: 'Drag across text or draw a highlight area.', shape: 'Drag to draw a shape.',
  signature: 'Type your name or choose a picture of your signature, then click where it goes.', form: 'Type into the form fields on the page.', watermark: 'Add one watermark to every page you choose.',
  crop: 'Drag a rectangle on the page to crop it.', redact: 'Drag over anything you want removed.',
};
const kindLabels: Record<SceneObject['kind'], string> = {
  text: 'Text box', highlight: 'Highlight', shape: 'Shape', signature: 'Signature', watermark: 'Watermark',
};
// Signatures placed during this app session, offered again under "Use again". Memory only, never saved to disk.
let usedSignatures: UsedSignature[] = [];
const usedSignatureKey = (item: UsedSignature) => item.mode === 'image' ? `image:${item.path}` : `text:${item.text}:${item.color.toLowerCase()}`;
const objectText = (object: SceneObject) => object.kind === 'shape' ? object.shape ?? 'square'
  : object.kind === 'highlight' ? 'highlighted area'
    : object.kind === 'signature' && object.signatureMode === 'image' ? 'signature image' : object.text;
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
// Form entries live outside the scene history, so they are counted per field against the last export.
const formChanges = (current: readonly PdfFormValue[], saved: readonly PdfFormValue[]) => {
  const exported = new Map(saved.map((entry) => [entry.name, entry.value]));
  return current.filter((entry) => exported.get(entry.name) !== entry.value).length
    + saved.filter((entry) => !current.some((item) => item.name === entry.name)).length;
};
type History = { past: PdfScene[]; present: PdfScene; future: PdfScene[] };
export type ExistingPdfUtilityId = 'pdf-merge' | 'pdf-split' | 'lock-unlock';
export type PdfEditorNavigation = (navigation: {
  utilityId: ExistingPdfUtilityId;
  initialPaths: readonly string[];
}) => void;
type ThumbnailSize = 'small' | 'medium' | 'large';
type OpenFailure = { kind: 'locked' | 'unreadable'; name: string };

/** Reads the typed error `inspect_pdf_scene` returns for a file the editor can't open. */
const openFailureFor = (reason: unknown, path: string): OpenFailure | null => {
  if (typeof reason !== 'object' || reason === null || (reason as ToolError).kind !== 'invalidInput') return null;
  const error = reason as ToolError;
  const locked = error.pointer?.kind === 'tool' && error.pointer.toolId === 'lock-unlock';
  return { kind: locked ? 'locked' : 'unreadable', name: fileName(path) };
};

function keepToolbarTabStop(toolbar: HTMLDivElement | null) {
  const buttons = Array.from(toolbar?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []);
  const active = buttons.indexOf(document.activeElement as HTMLButtonElement);
  buttons.forEach((button, index) => { button.tabIndex = index === (active >= 0 ? active : 0) ? 0 : -1; });
}

function handleToolbarFocus(event: React.FocusEvent<HTMLDivElement>) {
  const target = event.target;
  if (!(target instanceof HTMLButtonElement) || target.disabled) return;
  const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
  buttons.forEach((button) => { button.tabIndex = button === target ? 0 : -1; });
}

function handleToolbarKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
  const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (current < 0 || buttons.length === 0) return;
  const next = event.key === 'ArrowRight' ? (current + 1) % buttons.length
    : event.key === 'ArrowLeft' ? (current - 1 + buttons.length) % buttons.length
      : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
  if (next < 0) return;
  event.preventDefault();
  buttons.forEach((button, index) => { button.tabIndex = index === next ? 0 : -1; });
  buttons[next].focus();
}

type PDFEditorWorkspaceProps = {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  onNavigate?: PdfEditorNavigation;
  onLeaveGuard?: (guard: DiscardGuard | null) => void;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
  fileActivation?: Extract<ShellEvent, { kind: 'files' }>;
  onActivationAccepted?: (activationId: string) => void;
  onFilesChange?: (paths: readonly string[]) => void;
  onSelectTool?: (id: AtomicToolId) => void;
};

export const PDFEditorWorkspaceView = ({ batchTools = pdfEditorBatchTools, ...props }: PDFEditorWorkspaceProps & { batchTools?: PdfEditorBatchTools }) => {
  const { utility, initialPaths, onFilesChange, onSelectTool } = props;
  if (isPdfSceneTool(utility.id)) return <PDFSceneEditor {...props} />;
  const batchTool = batchTools[utility.id];
  if (batchTool) return <PdfEditorBatchSurface utility={utility} tool={batchTool} initialPaths={initialPaths} onFilesChange={onFilesChange} />;
  return <PDFConversionWorkspaceView utility={utility} initialPaths={initialPaths} onFilesChange={onFilesChange} onSelectTool={onSelectTool} />;
};

const PDFSceneEditor = ({
  utility,
  initialPaths,
  onNavigate,
  onWorkspaceSourceAction,
  fileActivation,
  onActivationAccepted,
  onFilesChange,
  onLeaveGuard,
}: PDFEditorWorkspaceProps) => {
  const availability = useToolAvailability();
  const session = useRef<{ path: string; scene: PdfScene; baseline: PdfScene; formValues: PdfFormValue[] } | null>(null);
  const [exported, setExported] = useState<{ path: string; scene: PdfScene; formValues: PdfFormValue[] } | null>(null);
  const [changes, setChanges] = useState(0);
  const pending = useRef(0);
  const reportChanges = useCallback((count: number) => { pending.current = count; setChanges(count); }, []);
  const [confirm, setConfirm] = useState<{ proceed: () => void; cancel?: () => void } | null>(null);
  const confirmDiscard: DiscardGuard = useCallback((proceed, cancel) => {
    if (pending.current > 0) setConfirm({ proceed, cancel }); else proceed();
  }, []);
  useEffect(() => {
    onLeaveGuard?.(confirmDiscard);
    return () => onLeaveGuard?.(null);
  }, [confirmDiscard, onLeaveGuard]);
  const settle = (choice: 'discard' | 'cancel' | 'export') => {
    const pendingChoice = confirm;
    setConfirm(null);
    if (!pendingChoice) return;
    if (choice === 'discard') { pending.current = 0; pendingChoice.proceed(); return; }
    pendingChoice.cancel?.();
    if (choice === 'export') pressShortcutTarget(window.document, 'run');
  };
  const requestedTool: EditorTool = utility.id === 'pdf-organize' ? 'organize' : utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  const initialTool = availability.allows(toolFlagBySceneTool[requestedTool])
    ? requestedTool
    : tools.find((item) => availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  // The export name follows the sub-tools that change the document, so the destination's example tracks the edits.
  const [suffix, setSuffix] = useState<string | null>(null);
  return <><ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene" initialPaths={initialPaths} onWorkspaceSourceAction={onWorkspaceSourceAction} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange} confirmDiscard={confirmDiscard}
    exampleName={(inputPath) => proposedSaveName(inputPath, suffix ?? combinedSuffix, 'pdf')}
    onRun={async (paths, outputLocation, { runId, outputPath }) => {
      const [inputPath] = paths;
      if (!inputPath || !session.current || session.current.path !== inputPath) throw new Error('Select exactly one open PDF before exporting.');
      const { scene, formValues } = session.current;
      const { request } = planSceneExport({ paths, scene, baseline: session.current.baseline, outputLocation, outputPath, formValues });
      const results = await invoke<ToolResult>('export_pdf_scene', { runId, request });
      if (results.length && results.every((result) => result.state === 'succeeded')) setExported({ path: inputPath, scene, formValues });
      return results.map((result) => result.failure ? { ...result, failure: { ...result.failure, message: pdfEditorErrorMessage(result.failure.message, 'export') } } : result);
    }}>
    {({ files, run, runSavingAs, loading, runDestination, live, browse, replaceFiles }) => <PDFSceneFiles files={files} title={utility.shortTitle ?? utility.title} replaceFiles={replaceFiles} onNavigate={onNavigate}>
      {(inputPath) => <PDFSceneSession key={inputPath ?? 'empty'} path={inputPath} initialTool={initialTool}
        exporting={loading} exportStep={live?.steps[0]} destination={runDestination} onExport={run} exported={exported} onChanges={reportChanges}
        onExportAs={() => runSavingAs({ dialog: exportAsDialog, suffix: suffix ?? combinedSuffix, summary: exportAsSummary })} onBrowse={browse}
        onNavigate={onNavigate}
        onScene={(path, scene, baseline, formValues) => {
          session.current = path && scene && baseline ? { path, scene, baseline, formValues } : null;
          setSuffix(scene && baseline ? exportSuffix(sceneContributors(scene, baseline, formValues)) : null);
        }} />}
    </PDFSceneFiles>}
  </ToolScaffold>
    <Dialog open={confirm !== null} onOpenChange={(open) => { if (!open) settle('cancel'); }}>
      <DialogContent role="alertdialog" showCloseButton={false} className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>Export your changes before closing?</DialogTitle>
          <DialogDescription>{session.current ? fileName(session.current.path) : 'This PDF'} has {changes} {changes === 1 ? 'change' : 'changes'} that haven't been exported. If you close now, they're lost.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="destructive" onClick={() => settle('discard')}>Discard changes</Button>
          <Button variant="outline" onClick={() => settle('cancel')}>Cancel</Button>
          <Button onClick={() => settle('export')}>Export…</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>;
};

/**
 * The scene editors work on one PDF. A newly dropped PDF replaces the open one. When several arrive at once,
 * the open document stays put while a dialog asks which one to open, and points to Merge for combining them.
 */
function PDFSceneFiles({ files, title, replaceFiles, onNavigate, children }: {
  files: string[]; title: string; replaceFiles: (paths: readonly string[]) => void;
  onNavigate?: PdfEditorNavigation; children: (path: string | null) => React.ReactNode;
}) {
  const availability = useToolAvailability();
  const openPath = useRef<string | null>(null);
  if (files.length <= 1) openPath.current = files[0] ?? null;
  const incoming = files.length > 1 ? files.filter((path) => path !== openPath.current) : [];
  const [choice, setChoice] = useState<string | null>(null);
  const incomingKey = incoming.join('\n');
  useEffect(() => {
    if (incoming.length === 1) replaceFiles(incoming);
    setChoice(incoming[0] ?? null);
  }, [incomingKey]);
  const choosing = incoming.length > 1;
  const keepOpenFile = () => replaceFiles(openPath.current ? [openPath.current] : []);
  return <>
    {children(openPath.current)}
    <Dialog open={choosing} onOpenChange={(open) => { if (!open) keepOpenFile(); }}>
      <DialogContent showCloseButton={false} className="scene-several-dropped">
        <DialogHeader>
          <DialogTitle>{title} works on one PDF at a time</DialogTitle>
          <DialogDescription>You dropped {incoming.length} PDFs. Choose the one to open. The others aren’t opened or changed.</DialogDescription>
        </DialogHeader>
        <RadioGroup aria-label="PDF to open" value={choice ?? ''} onValueChange={setChoice} className="gap-2">
          {incoming.map((path, index) => <div key={path} className="flex items-center gap-3">
            <RadioGroupItem id={`scene-dropped-${index}`} value={path} />
            <Label htmlFor={`scene-dropped-${index}`}>{fileName(path)}</Label>
          </div>)}
        </RadioGroup>
        {onNavigate && availability.allows('pdf-merge') && <Card className="py-0"><CardContent className="flex items-center gap-3 p-3 text-sm">
          <span className="flex-1 text-muted-foreground">Want them all as one PDF? Merge combines whole files in order. Then open the merged copy here to rearrange its pages.</span>
          <Button variant="outline" size="sm" onClick={() => onNavigate({ utilityId: 'pdf-merge', initialPaths: incoming })}>Open Merge</Button>
        </CardContent></Card>}
        <DialogFooter>
          <Button variant="outline" onClick={keepOpenFile}>Cancel</Button>
          <Button disabled={!choice} onClick={() => { if (choice) replaceFiles([choice]); }}>Open</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>;
}

function PDFSceneSession({ path, initialTool, exporting, exportStep, destination, onExport, onExportAs, onBrowse, exported, onChanges, onNavigate, onScene }: {
  path: string | null; initialTool: EditorTool | null; exporting: boolean; exportStep: FileProgress | undefined;
  destination: RunDestinationControl; onExport: () => Promise<void>; onExportAs: () => Promise<void>;
  onBrowse: () => Promise<void>; exported: { path: string; scene: PdfScene; formValues: PdfFormValue[] } | null; onChanges: (count: number) => void;
  onNavigate?: PdfEditorNavigation;
  onScene: (path: string | null, scene: PdfScene | null, baseline: PdfScene | null, formValues: PdfFormValue[]) => void;
}) {
  const availability = useToolAvailability();
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [signatureNoticeDismissedFor, setSignatureNoticeDismissedFor] = useState<string | null>(null);
  const [history, setHistory] = useState<History>({ past: [], present: { pages: [] }, future: [] });
  const [saved, setSaved] = useState<{ scene: PdfScene; formValues: PdfFormValue[]; index: number } | null>(null);
  const [tool, setTool] = useState<EditorTool | null>(initialTool);
  const [thumbnailSize, setThumbnailSize] = useState<ThumbnailSize>('medium');
  const [visiblePages, setVisiblePages] = useState<number[]>([]);
  const [openFailure, setOpenFailure] = useState<OpenFailure | null>(null);
  const organizeGrid = useRef<HTMLDivElement>(null);
  useEffect(() => setTool(initialTool), [initialTool]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [selectedPages, setSelectedPages] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<number | null>(null);
  const [zoom, setZoom] = useState(1);
  const text = 'Text';
  const [fontSize, setFontSize] = useState(18);
  const [textStyle, setTextStyle] = useState<TextStyle>({ fontFamily: 'Noto Sans', bold: false, italic: false, align: 'left' });
  const [color, setColor] = useState(() => readColorToken(colorTokens.documentText));
  const [opacity, setOpacity] = useState(1);
  const [highlightColor, setHighlightColor] = useState(() => readColorToken(colorTokens.documentHighlight));
  const [highlightOpacity, setHighlightOpacity] = useState(0.35);
  const [shape, setShape] = useState<SceneShape>('square');
  const [signatureMode, setSignatureMode] = useState<SignatureMode>('text');
  const [signaturePath, setSignaturePath] = useState<string | null>(null);
  const [signaturePreview, setSignaturePreview] = useState<string | null>(null);
  const [signatureText, setSignatureText] = useState('Your signature');
  const [signatureTextDraft, setSignatureTextDraft] = useState('');
  const [signatureColor, setSignatureColor] = useState('#1f1f1f');
  const [signatureError, setSignatureError] = useState<string | null>(null);
  const [reusable, setReusable] = useState(usedSignatures);
  const [watermarkPictureError, setWatermarkPictureError] = useState<string | null>(null);
  const [redactionFill, setRedactionFill] = useState<string>(redactionFills.Black);
  const [reviewing, setReviewing] = useState(false);
  const [previewRedactions, setPreviewRedactions] = useState(true);
  const [disclosure, setDisclosure] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Form entries live outside the scene history: Reset clears them, Undo does not.
  const [form, setForm] = useState<PdfFormInspection | null>(null);
  const [formEntries, setFormEntries] = useState<Record<string, PdfFormEntry>>({});
  const [formLeft, setFormLeft] = useState<ReadonlySet<string>>(new Set());
  const [formFocus, setFormFocus] = useState<string | null>(null);
  const [formNotice, setFormNotice] = useState<string | null>(null);
  const formRequested = useRef(false);
  const [loading, setLoading] = useState(Boolean(path));
  const [rendering, setRendering] = useState(false);
  const [preview, setPreview] = useState<{ key: string; value: ScenePreview } | null>(null);
  const [thumbnails, setThumbnails] = useState<Record<string, { key: string; value: ScenePreview }>>({});
  const [renderError, setRenderError] = useState<string | null>(null);
  const group = useRef<string | null>(null);
  const focusMark = useRef(false);
  const draggedPage = useRef<string | null>(null);
  const toolsToolbar = useRef<HTMLDivElement>(null);
  const bottomToolbar = useRef<HTMLDivElement>(null);
  const previewCache = useRef(createPreviewCache());
  const previewGeneration = useRef<GenerationState>({ current: 0 });
  const textGeneration = useRef<GenerationState>({ current: 0 });
  useLayoutEffect(() => {
    keepToolbarTabStop(toolsToolbar.current);
    keepToolbarTabStop(bottomToolbar.current);
  });
  const scene = history.present;
  const page = scene.pages.find((item) => item.id === currentId) ?? scene.pages[0];
  const currentIndex = scene.pages.findIndex((item) => item.id === page?.id);
  const watermarks = page ? objectsOnPage(scene, currentIndex).filter(({ object }) => object.kind === 'watermark').map(({ object }) => object) : [];
  const selected = page?.objects.find((object) => object.id === selectedId) ?? watermarks.find((object) => object.id === selectedId);
  const serializedScene = JSON.stringify(scene);
  const previewKey = path ? previewCacheKey(path, currentIndex, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
  const exactPreview = preview?.key === previewKey ? preview.value : null;
  useEffect(() => {
    if (tool !== 'form' || !path || formRequested.current) return;
    formRequested.current = true;
    invoke<PdfFormInspection | null>('inspect_pdf_form', { request: { path } })
      .then((value) => setForm(value ?? { fields: [], unsupported: null }))
      .catch(() => setForm({ fields: [], unsupported: "This form can't be filled here. Toolbox couldn't read its fields." }));
  }, [tool, path]);
  const formValue = (field: PdfFormField) => formEntries[field.name] ?? field.value;
  const formValues = useMemo<PdfFormValue[]>(() => (form?.fields ?? [])
    .filter((field) => field.name in formEntries && JSON.stringify(formEntries[field.name]) !== JSON.stringify(field.value))
    .map((field) => ({ name: field.name, value: formEntries[field.name] })), [form, formEntries]);
  const baseline = document ? sceneFromDocument(document) : null;
  const contributors = baseline ? sceneContributors(scene, baseline, formValues) : [];
  const dirty = contributors.length > 0;
  const canExport = Boolean(page) && dirty && !exporting && !renderError && !rendering && Boolean(exactPreview);
  // Why Export is off, said next to it: a copy with no changes would match the original, and a stale preview can't be verified.
  const exportReason = !page || exporting || renderError ? null
    : !dirty ? { title: 'Nothing to export yet', detail: `This document has no changes, so a copy would match the original. Use a tool in the toolbar first. ${shortcutLabel('run')} does nothing until then.` }
      : !exactPreview || rendering ? { title: 'Updating the preview', detail: 'Export turns on once the preview shows your latest edits.' }
        : null;
  const selectedSignature = selected?.kind === 'signature' ? selected : null;
  const marks = listMarks(scene);
  // Export and Export As both pause on the redaction disclosure; the chosen action runs once the marks are confirmed.
  const pendingExport = useRef(onExport);
  const requestExport = (action: () => Promise<void>) => { pendingExport.current = action; if (marks.length) setDisclosure(true); else void action(); };
  const updateThumbnail = (item: ScenePage, key: string, value: ScenePreview) => {
    setThumbnails((existing) => {
      if (existing[item.id]?.key === key) return existing;
      const next = { ...existing, [item.id]: { key, value } };
      const limit = tool === 'organize' ? Math.max(PREVIEW_CACHE_MAX_ENTRIES, scene.pages.length) : PREVIEW_CACHE_MAX_ENTRIES;
      const staleIds = Object.keys(next).slice(0, Math.max(0, Object.keys(next).length - limit));
      for (const staleId of staleIds) delete next[staleId];
      return next;
    });
  };

  useEffect(() => {
    let active = true;
    if (!path) { onScene(null, null, null, []); return; }
    invoke<PdfDocument>('inspect_pdf_scene', { request: { path } }).then((value) => {
      if (!active) return;
      if (!value.pages.length) throw new Error('The PDF has no pages.');
      setDocument(value);
      const initial = sceneFromDocument(value);
      setHistory({ past: [], present: initial, future: [] });
      setCurrentId(initial.pages[0].id); setSelectedPages([initial.pages[0].id]);
    }).catch((reason) => {
      if (!active) return;
      const failure = openFailureFor(reason, path);
      if (failure) setOpenFailure(failure); else setError(pdfEditorErrorMessage(reason, 'open'));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [path]);
  useEffect(() => {
    const sourceIndex = page?.sourceIndex;
    const sourcePage = sourceIndex === null || sourceIndex === undefined ? undefined : document?.pages[sourceIndex];
    if (!path || !document || sourceIndex === null || sourceIndex === undefined || !sourcePage || sourcePage.textRuns != null) {
      return () => { textGeneration.current.current += 1; };
    }
    const requestGeneration = ++textGeneration.current.current;
    void requestWithGeneration(textGeneration.current, requestGeneration,
      () => invoke<PdfPage>('inspect_pdf_scene_page', { request: { path, pageIndex: sourceIndex } }),
      (value) => {
        if (value.index !== sourceIndex) return;
        const textRuns: PdfTextRun[] = value.textRuns ?? [];
        const editRuns: PdfEditRun[] = value.editRuns ?? [];
        setDocument((current) => {
          if (!current || current.path !== path || textGeneration.current.current !== requestGeneration || current.pages[sourceIndex]?.textRuns != null) return current;
          return { ...current, pages: current.pages.map((item, index) => index === sourceIndex ? { ...item, preview: value.preview ?? item.preview ?? null, textRuns, editRuns } : item) };
        });
      },
      (reason) => setError(pdfEditorErrorMessage(reason, 'open')),
      () => {});
    return () => { textGeneration.current.current += 1; };
  }, [path, document, page?.sourceIndex]);
  // Flags show once the user has left a field: an entry the export would refuse, or a required field still empty.
  const formFlag = (field: PdfFormField) => !formLeft.has(field.name) ? null
    : formEntryFlag(field, formValue(field)) ?? (field.required && isFormEntryEmpty(formValue(field)) ? 'Required' : null);
  const formLabel = (field: PdfFormField) => field.label || field.name;
  const leaveFormField = (field: PdfFormField) => {
    setFormLeft((left) => new Set(left).add(field.name));
    const flag = formEntryFlag(field, formValue(field));
    setFormNotice(flag ? `${formLabel(field)}: ${flag.toLowerCase()}.` : field.required && isFormEntryEmpty(formValue(field)) ? `${formLabel(field)} is required.` : null);
  };
  useEffect(() => { onScene(path, document ? scene : null, document ? sceneFromDocument(document) : null, formValues); }, [path, scene, document, formValues]);
  useLayoutEffect(() => {
    if (selectedSignature) setSignatureTextDraft(selectedSignature.text);
  }, [selectedSignature?.id, selectedSignature?.text]);
  useEffect(() => {
    previewGeneration.current.current += 1;
    previewCache.current.clear();
    setPreview(null);
    setThumbnails({});
    setRenderError(null);
    setRendering(false);
  }, [path, serializedScene]);
  useEffect(() => {
    const requestGeneration = ++previewGeneration.current.current;
    const serializedScene = JSON.stringify(scene);
    if (!path || !page || currentIndex < 0) {
      setPreview(null); setRenderError(null); setRendering(false);
      return () => { previewGeneration.current.current += 1; };
    }
    setRenderError(null);
    setRendering(true);
    setPreview(null);
    const indexes = [...new Set([currentIndex, currentIndex - 1, currentIndex + 1]
      .filter((index) => index >= 0 && index < scene.pages.length))];
    const missing: { index: number; item: ScenePage; key: string }[] = [];
    for (const index of indexes) {
      const item = scene.pages[index];
      const key = previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS);
      const cached = previewCache.current.get(key);
      if (cached) {
        if (index === currentIndex) {
          setPreview({ key, value: cached });
          setRendering(false);
        }
        updateThumbnail(item, key, cached);
      } else missing.push({ index, item, key });
    }
    const timer = missing.length ? window.setTimeout(() => {
      if (previewGeneration.current.current !== requestGeneration) return;
      void requestWithGeneration(previewGeneration.current, requestGeneration,
        () => invoke<IndexedScenePreview[]>('preview_pdf_scene_pages', { request: { path, ...sceneSections(scene), pageIndices: missing.map(({ index }) => index) } }),
        (values) => {
          const previews = new Map(values.map(({ pageIndex, preview }) => [pageIndex, preview]));
          for (const { index, item, key } of missing) {
            const value = previews.get(index);
            if (!value) {
              if (index === currentIndex) setRenderError('Preview response did not include the current page.');
              continue;
            }
            previewCache.current.set(key, value);
            if (index === currentIndex) setPreview({ key, value });
            updateThumbnail(item, key, value);
          }
        },
        (reason) => { if (missing.some(({ index }) => index === currentIndex)) setRenderError(pdfEditorErrorMessage(reason, 'preview')); },
        () => setRendering(false));
    }, 200) : undefined;
    return () => { if (timer !== undefined) window.clearTimeout(timer); previewGeneration.current.current += 1; };
  }, [path, currentIndex, serializedScene]);

  // The organize grid asks for previews of the pages on screen, a few at a time, as the person scrolls.
  useEffect(() => {
    const grid = organizeGrid.current;
    if (tool !== 'organize' || !grid || typeof IntersectionObserver === 'undefined') return;
    const shown = new Set<number>();
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const index = Number((entry.target as HTMLElement).dataset.pageIndex);
        if (entry.isIntersecting) shown.add(index); else shown.delete(index);
      }
      setVisiblePages([...shown].sort((a, b) => a - b));
    }, { root: grid, rootMargin: '200px' });
    grid.querySelectorAll('[data-page-index]').forEach((cell) => observer.observe(cell));
    return () => observer.disconnect();
  }, [tool, scene.pages.length, Boolean(page)]);
  useEffect(() => {
    if (tool !== 'organize' || !path) return;
    const missing = visiblePages.filter((index) => {
      const item = scene.pages[index];
      if (!item) return false;
      const key = previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS);
      if (thumbnails[item.id]?.key === key) return false;
      const cached = previewCache.current.get(key);
      if (cached) { updateThumbnail(item, key, cached); return false; }
      return true;
    }).slice(0, PREVIEW_CACHE_MAX_ENTRIES);
    if (!missing.length) return;
    let active = true;
    const timer = window.setTimeout(() => {
      invoke<IndexedScenePreview[]>('preview_pdf_scene_pages', { request: { path, scene, pageIndices: missing } }).then((values) => {
        if (!active) return;
        for (const { pageIndex, preview: value } of values) {
          const item = scene.pages[pageIndex];
          if (!item) continue;
          const key = previewCacheKey(path, pageIndex, serializedScene, PREVIEW_RENDER_SETTINGS);
          previewCache.current.set(key, value);
          updateThumbnail(item, key, value);
        }
      }).catch(() => { /* A grid thumbnail is optional; the focused page reports its own preview errors. */ });
    }, 200);
    return () => { active = false; window.clearTimeout(timer); };
  }, [tool, path, serializedScene, visiblePages.join(',')]);

  const commit = (next: PdfScene, mergeGroup: string | null = null) => {
    const merge = Boolean(mergeGroup && group.current === mergeGroup);
    group.current = mergeGroup;
    setHistory((previous) => same(previous.present, next) ? previous : ({
      past: merge ? previous.past : [...previous.past, previous.present].slice(-100), present: next, future: [],
    }));
    setPreview(null);
  };
  const commitPage = (next: ScenePage) => commit({ ...scene, pages: scene.pages.map((item) => item.id === next.id ? next : item) });
  const undo = () => { group.current = null; setPreview(null); setHistory((value) => value.past.length ? {
    past: value.past.slice(0, -1), present: value.past[value.past.length - 1], future: [value.present, ...value.future],
  } : value); };
  const redo = () => { group.current = null; setPreview(null); setHistory((value) => value.future.length ? {
    past: [...value.past, value.present], present: value.future[0], future: value.future.slice(1),
  } : value); };
  const endGroup = () => { group.current = null; };
  const reset = () => { if (document) commit(sceneFromDocument(document)); setFormEntries({}); setFormLeft(new Set()); setFormNotice(null); setSelectedId(null); setSelectedRun(null); };
  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === 'undo') undo();
      else if (command === 'redo') redo();
      else if (command === 'export' && canExport) requestExport(onExport);
    };
    window.addEventListener('toolbox:editor-command', onShellCommand);
    return () => window.removeEventListener('toolbox:editor-command', onShellCommand);
  }, [canExport, onExport, redo, undo, marks.length]);
  const selectMark = (mark: ListedMark) => { endGroup(); setCurrentId(mark.pageId); setSelectedPages([mark.pageId]); setSelectedId(mark.id); };
  const removeMarks = (ids: string[]) => {
    if (!availability.allows('pdf-redact')) return;
    commit({ pages: scene.pages.map((item) => item.redactions?.some((mark) => ids.includes(mark.id))
      ? { ...item, redactions: item.redactions.filter((mark) => !ids.includes(mark.id)) } : item) });
    setSelectedId(null);
    if (ids.length >= marks.length) setReviewing(false);
  };
  const recolorMarks = (color: string) => {
    setRedactionFill(color);
    if (marks.some((mark) => mark.color !== color)) commit({ pages: scene.pages.map((item) => item.redactions?.length
      ? { ...item, redactions: item.redactions.map((mark) => ({ ...mark, color })) } : item) });
  };
  const review = (on: boolean) => {
    setReviewing(on);
    if (on && marks.length && !marks.some((mark) => mark.id === selectedId)) selectMark(marks[0]);
  };
  useEffect(() => {
    if (!focusMark.current) return;
    focusMark.current = false;
    window.document.querySelector<SVGElement>(`[data-redaction="${selectedId}"]`)?.focus();
  }, [selectedId, currentId]);
  // Patches or removes an object on whichever page stores it, so a range-applied watermark can be edited from any page.
  const changeObject = (id: string, patch: Partial<SceneObject> | null, field: string) => {
    const object = scene.pages.flatMap((item) => item.objects).find((item) => item.id === id);
    if (!object || !availability.allows(toolFlagByObjectKind[object.kind])) return;
    commit({ ...scene, pages: scene.pages.map((item) => item.objects.some((candidate) => candidate.id === id) ? { ...item,
      objects: patch ? item.objects.map((candidate) => candidate.id === id ? { ...candidate, ...patch } : candidate) : item.objects.filter((candidate) => candidate.id !== id),
    } : item) }, patch ? `${id}:${field}` : null);
    if (!patch) setSelectedId(null);
  };
  const updateObject = (patch: Partial<SceneObject>, field: string) => { if (selected) changeObject(selected.id, patch, field); };
  const makeObject = (kind: SceneObject['kind'], rect: SceneRect): SceneObject => ({
    id: crypto.randomUUID(), kind, rect,
    text: kind === 'signature' ? signatureText : text,
    fontSize: kind === 'signature' ? Math.max(18, fontSize) : fontSize,
    color: kind === 'highlight' ? highlightColor : kind === 'signature' ? signatureColor : color,
    opacity: kind === 'highlight' ? highlightOpacity : kind === 'signature' ? 1 : opacity,
    shape: kind === 'shape' ? shape : undefined,
    highlightMode: kind === 'highlight' ? 'area' : undefined,
    signatureMode: kind === 'signature' ? signatureMode : undefined,
    signaturePath: kind === 'signature' && signatureMode === 'image' ? signaturePath : undefined,
    signaturePreview: kind === 'signature' && signatureMode === 'image' ? signaturePreview : undefined,
    ...(kind === 'text' ? textStyle : {}),
  });
  const chooseSignatureImage = async () => {
    if (!availability.allows('pdf-sign')) return;
    try {
      const picked = await open({ multiple: false, filters: [{ name: 'Signature picture', extensions: ['png', 'jpg', 'jpeg'] }] });
      if (typeof picked !== 'string') return;
      try { await invoke('inspect_signature_image', { request: { path: picked } }); }
      catch (reason) { setSignatureError(String(reason)); return; }
      setSignatureError(null);
      let previewSource: string | null = null;
      try { previewSource = convertFileSrc(picked); } catch { /* Keep the original path for native export, but fail closed for the preview. */ }
      setSignatureMode('image'); setSignaturePath(picked); setSignaturePreview(previewSource);
      if (selected?.kind === 'signature') updateObject({ signatureMode: 'image', signaturePath: picked, signaturePreview: previewSource }, 'signature-image');
    } catch (reason) { setError(String(reason)); }
  };
  useEffect(() => {
    const placed = scene.pages.flatMap((item) => item.objects).filter((object) => object.kind === 'signature' && object.signatureMode
      && (object.signatureMode === 'image' ? object.signaturePath : object.text.trim()))
      .map((object): UsedSignature => ({ mode: object.signatureMode!, text: object.text, path: object.signaturePath ?? null, preview: object.signaturePreview ?? null, color: object.color }));
    const fresh = placed.filter((item, index) => !usedSignatures.some((used) => usedSignatureKey(used) === usedSignatureKey(item))
      && placed.findIndex((other) => usedSignatureKey(other) === usedSignatureKey(item)) === index);
    if (fresh.length) { usedSignatures = [...fresh, ...usedSignatures]; setReusable(usedSignatures); }
  }, [scene]);
  const reuseSignature = (item: UsedSignature) => {
    setSignatureMode(item.mode); setSignatureColor(item.color); setSignatureError(null);
    if (item.mode === 'text') setSignatureText(item.text); else { setSignaturePath(item.path); setSignaturePreview(item.preview); }
    endGroup(); setTool('signature'); setSelectedId(null);
  };
  const clearUsedSignatures = () => { usedSignatures = []; setReusable(usedSignatures); };
  const addWatermark = () => {
    if (!page || !availability.allows('pdf-watermark')) return;
    const color = readColorToken(colorTokens.documentWatermark);
    const object: SceneObject = { ...makeObject('watermark', visibleBounds(page)), text: 'DRAFT', color, opacity: 1, fontFamily: 'Noto Sans', bold: true,
      watermark: { text: 'DRAFT', opacity: 18, color, position: 'center', rotation: -45 }, layer: 'behind', pageRange: 'all' };
    commitPage({ ...page, objects: [...page.objects, object] });
    setSelectedId(object.id); setWatermarkPictureError(null);
  };
  const chooseWatermarkPicture = async () => {
    if (selected?.kind !== 'watermark' || !selected.watermark || !availability.allows('pdf-watermark')) return;
    setWatermarkPictureError(null);
    try {
      const picked = await open({ multiple: false, filters: [{ name: 'Watermark picture', extensions: ['png', 'jpg', 'jpeg'] }] });
      if (typeof picked !== 'string') return;
      const size = await invoke<{ width: number; height: number }>('inspect_pdf_watermark_image', { request: { path: picked } });
      let previewSource: string | null = null;
      try { previewSource = convertFileSrc(picked); } catch { /* Keep the path for native export; the rendered preview still shows the picture. */ }
      const { text: _text, color: _color, ...options } = selected.watermark;
      updateObject({ watermark: { ...options, logoPath: picked, size: undefined }, watermarkPreview: previewSource, watermarkImageSize: size }, 'watermark-picture');
    } catch (reason) { setWatermarkPictureError(reason instanceof Error ? reason.message : String(reason)); }
  };
  const selectPage = (id: string, additive = false, range = false) => {
    if (range && page) {
      const other = scene.pages.findIndex((item) => item.id === id);
      setSelectedPages(scene.pages.slice(Math.min(currentIndex, other), Math.max(currentIndex, other) + 1).map((item) => item.id));
    } else setSelectedPages((old) => additive ? old.includes(id) ? old.filter((item) => item !== id) : [...old, id] : [id]);
    setCurrentId(id); setSelectedId(null); setSelectedRun(null); group.current = null;
  };
  const goToField = (field: PdfFormField) => {
    const target = scene.pages.find((item) => item.sourceIndex === field.widgets[0].page);
    if (!target) return;
    if (target.id !== page?.id) selectPage(target.id);
    requestAnimationFrame(() => window.document.querySelector<HTMLElement>(`[data-form-field="${CSS.escape(field.name)}"] :is(input, textarea, select, button, [tabindex]):not([tabindex="-1"])`)?.focus());
  };
  const goToNextRequired = () => {
    const fields = form?.fields ?? [];
    const from = fields.findIndex((field) => field.name === formFocus);
    const next = [...fields.slice(from + 1), ...fields.slice(0, from + 1)].find((field) => field.required && isFormEntryEmpty(formValue(field)));
    if (!next) return;
    setFormNotice(`${formLabel(next)} is required.`);
    goToField(next);
  };
  const formUnavailable = Boolean(form && (form.unsupported || !form.fields.length));
  const pageTargets = selectedPages.filter((id) => scene.pages.some((item) => item.id === id));
  const targets = pageTargets.length ? pageTargets : page ? [page.id] : [];
  const deletePages = () => {
    if (!availability.allows('pdf-remove-pages')) return;
    const next = removePages(scene, targets);
    if (!next) return;
    commit(next); setCurrentId(next.pages[Math.min(currentIndex, next.pages.length - 1)].id); setSelectedPages([]); setSelectedId(null);
  };
  const movePages = (targetId: string) => {
    if (!availability.allows('pdf-organize')) return;
    const dragged = draggedPage.current; draggedPage.current = null;
    if (!dragged || dragged === targetId) return;
    commit(movePagesBefore(scene, targets.includes(dragged) ? targets : [dragged], targetId));
  };
  const shiftPage = (delta: number) => {
    if (!availability.allows('pdf-organize')) return;
    commit(swapPage(scene, currentIndex, delta));
  };
  const pageNumbers = () => {
    if (!availability.allows('pdf-page-numbers')) return;
    commit({ ...scene, pages: scene.pages.map((item, index) => {
    const b = visibleBounds(item);
    return { ...item, objects: [...item.objects, { ...makeObject('text', { x: b.x + b.width / 2 - 20, y: b.y + Math.max(0, b.height - 30), width: Math.min(40, b.width), height: Math.min(20, b.height) }), text: String(index + 1), fontSize: 12 }] };
    }) });
  };
  const savedScene = saved?.scene ?? baseline;
  const sceneChanges = !savedScene || same(scene, savedScene) ? 0 : Math.max(1, Math.abs(history.past.length - (saved?.index ?? 0)));
  const changes = sceneChanges + formChanges(formValues, saved?.formValues ?? []);
  useEffect(() => {
    if (!exported || exported.path !== path) return;
    const states = [...history.past, history.present].map((item) => JSON.stringify(item));
    const index = states.lastIndexOf(JSON.stringify(exported.scene));
    setSaved({ scene: exported.scene, formValues: exported.formValues, index: index >= 0 ? index : history.past.length });
  }, [exported]);
  useEffect(() => onChanges(changes), [changes, onChanges]);
  useEffect(() => () => onChanges(0), [onChanges]);
  const placed = scene.pages.flatMap((item, index) => item.objects.map((object) => ({ pageId: item.id, number: index + 1, object })));
  const duplicate = () => {
    if (!selected || !page || selected.kind === 'watermark' || !availability.allows(toolFlagByObjectKind[selected.kind])) return;
    const b = visibleBounds(page), r = selected.rect;
    const copy: SceneObject = { ...selected, id: crypto.randomUUID(), rect: { ...r,
      x: Math.max(b.x, Math.min(b.x + b.width - r.width, r.x + 10)), y: Math.max(b.y, Math.min(b.y + b.height - r.height, r.y + 10)) } };
    group.current = null;
    commitPage({ ...page, objects: [...page.objects, copy] });
    setSelectedId(copy.id);
  };
  useEffect(() => {
    const onDuplicate = (event: KeyboardEvent) => {
      if (event.shiftKey || matchShortcut(event, 'editor') !== 'duplicate') return;
      if (window.document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
      event.preventDefault();
      duplicate();
    };
    window.addEventListener('keydown', onDuplicate);
    return () => window.removeEventListener('keydown', onDuplicate);
  });
  const pageViewTool = tools.find((item) => item.id !== 'organize' && availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  const focusGridPage = (index: number) => {
    const target = scene.pages[Math.max(0, Math.min(scene.pages.length - 1, index))];
    if (!target) return;
    selectPage(target.id);
    requestAnimationFrame(() => organizeGrid.current?.querySelector<HTMLButtonElement>(`[data-page-index="${scene.pages.indexOf(target)}"]`)?.focus());
  };
  const gridKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const next = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? currentIndex + 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? currentIndex - 1
        : event.key === 'Home' ? 0 : event.key === 'End' ? scene.pages.length - 1 : null;
    if (next === null) return;
    event.preventDefault();
    focusGridPage(next);
  };
  const documentName = path ? fileName(path) : '';
  const pageCount = `${scene.pages.length} ${scene.pages.length === 1 ? 'page' : 'pages'}`;
  const editRuns: PdfEditRun[] = page?.sourceIndex == null ? [] : document?.pages[page.sourceIndex]?.editRuns ?? [];
  const textEdits = scene.textEdits ?? [];
  const pageEdits = textEdits.filter((edit) => edit.pageIndex === page?.sourceIndex);
  const commitTextEdits = (next: PdfTextEdit[]) => commit({ pages: scene.pages, ...(next.length ? { textEdits: next } : {}) });
  const setTextEdit = (run: PdfEditRun, replacement: string) => {
    if (!availability.allows('pdf-edit') || run.reason || page?.sourceIndex == null) return;
    const pageIndex = page.sourceIndex;
    const others = textEdits.filter((edit) => edit.pageIndex !== pageIndex || edit.run !== run.index);
    commitTextEdits(replacement === run.text ? others : [...others, { pageIndex, run: run.index, original: run.text, replacement }]);
  };
  const revertEdit = (edit: PdfTextEdit) => { if (availability.allows('pdf-edit')) commitTextEdits(textEdits.filter((item) => item !== edit)); };
  const showEdit = (edit: PdfTextEdit) => {
    const target = scene.pages.find((item) => item.sourceIndex === edit.pageIndex);
    if (!target) return;
    selectPage(target.id); setTool('edit-text'); setSelectedRun(edit.run);
  };
  const chooseTool = (next: EditorTool) => { if (!availability.allows(toolFlagBySceneTool[next])) return; endGroup(); setTool(next); setSelectedId(null); setSelectedRun(null); };
  const selectedKind = selected?.kind ?? tool;
  const selectedEnabled = Boolean(selected && availability.allows(toolFlagByObjectKind[selected.kind]));
  // A new signature needs a name or a checked picture before it can be placed.
  const signatureReady = signatureMode === 'image' ? Boolean(signaturePath) : Boolean(signatureText.trim());
  const canEditCanvas = tool !== null && availability.allows(toolFlagBySceneTool[tool])
    && (!selected || availability.allows(toolFlagByObjectKind[selected.kind]))
    && (selected || tool !== 'signature' || signatureReady);
  const activeColor = selected?.color ?? (tool === 'highlight' ? highlightColor : color);
  const activeOpacity = selected?.opacity ?? (tool === 'highlight' ? highlightOpacity : opacity);

  const inspector: SubToolInspectorProps | null = page ? {
    page, pageNumber: currentIndex + 1, selected, hasTextRuns: Boolean(document?.pages[page.sourceIndex ?? -1]?.textRuns?.length), updateObject, commitPage, endGroup,
    fontSize, setFontSize, textStyle, setTextStyle, shape, setShape,
    signatureMode, setSignatureMode, signaturePath, signaturePreview, signatureText, setSignatureText,
    signatureTextDraft, setSignatureTextDraft, signatureColor, setSignatureColor, signatureError, chooseSignatureImage,
    usedSignatures: reusable, reuseSignature, clearUsedSignatures,
    pageCount: scene.pages.length, addWatermark, chooseWatermarkPicture, watermarkPictureError,
    form, formValue, formFlag, goToField, goToNextRequired, switchToAddText: () => { endGroup(); setTool('text'); setSelectedId(null); },
    selectedRun: editRuns.find((run) => run.index === selectedRun), textEdits,
    pageNumberOf: (sourceIndex) => scene.pages.findIndex((item) => item.sourceIndex === sourceIndex) + 1, showEdit, revertEdit, chooseTool,
  } : null;
  const watermarkCounts = new Array<number>(scene.pages.length).fill(0);
  scene.pages.forEach((item, origin) => item.objects.forEach((object) => {
    if (object.kind !== 'watermark') return;
    try { for (const index of object.pageRange == null ? [origin] : pagesInRange(object.pageRange, scene.pages.length)) watermarkCounts[index] += 1; } catch { /* The inspector reports the range. */ }
  }));
  const markCount = (index: number) => scene.pages[index].objects.filter((object) => object.kind !== 'watermark').length + watermarkCounts[index];
  const Fields = selectedKind && selectedKind !== 'organize' ? subToolInspectors[selectedKind]?.Fields : undefined;
  const Guide = tool && tool !== 'organize' ? subToolInspectors[tool]?.Guide : undefined;

  return <section className="pdf-scene-workspace" aria-label="PDF editing session" onKeyDown={(event) => {
    const markId = event.target instanceof SVGElement ? event.target.dataset.redaction : undefined;
    if (reviewing && event.key === 'Tab' && markId && marks.length) {
      event.preventDefault();
      const index = marks.findIndex((mark) => mark.id === markId);
      focusMark.current = true;
      selectMark(marks[(index + (event.shiftKey ? -1 : 1) + marks.length) % marks.length]);
      return;
    }
    const input = event.target instanceof HTMLElement && (['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName) || event.target.isContentEditable);
    if (input || !(event.metaKey || event.ctrlKey)) return;
    const key = event.key.toLowerCase();
    if (key === 'z') { event.preventDefault(); event.shiftKey ? redo() : undo(); }
    else if (key === 'y' && !event.shiftKey) { event.preventDefault(); redo(); }
  }}>
    <div ref={toolsToolbar} className="scene-tools" role="toolbar" aria-label="PDF editor tools" aria-orientation="horizontal" onFocusCapture={handleToolbarFocus} onKeyDown={handleToolbarKeyDown}>
      {tools.filter((item) => availability.allows(toolFlagBySceneTool[item.id])).map((item) => <Button key={item.id} variant={tool === item.id ? "secondary" : "ghost"} size="sm" aria-label={item.label} aria-description={toolGuidance[item.id]} title={toolGuidance[item.id]} aria-pressed={tool === item.id}
        onClick={() => chooseTool(item.id)} disabled={!page}><span aria-hidden="true">{item.glyph}</span>{item.label}</Button>)}
      <span className="scene-toolbar-divider" />
      <Button variant="ghost" size="icon-sm" aria-label="Undo" title={`Undo (${shortcutLabel('undo')})`} disabled={!history.past.length} onClick={undo}>↶</Button>
      <Button variant="ghost" size="icon-sm" aria-label="Redo" title={`Redo (${shortcutLabel('redo')})`} disabled={!history.future.length} onClick={redo}>↷</Button>
      <Button variant="destructive" size="sm" aria-label="Reset edits" disabled={!dirty} onClick={reset}>Reset</Button>
      <ExportDestination destination={destination} inputPath={path} />
      {exporting ? <ExportProgress step={exportStep} fileName={path ? fileName(path) : 'PDF'} /> : <>
        <Button variant="outline" size="sm" className="scene-export-as" aria-label="Export As…" disabled={!canExport}
          aria-describedby={exportReason ? 'scene-export-reason' : undefined} onClick={() => requestExport(onExportAs)}>Export As…</Button>
        <Button variant="default" size="sm" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!canExport}
          aria-describedby={exportReason ? 'scene-export-reason' : undefined}
          {...shortcutTarget('run')} onClick={() => requestExport(onExport)}>Export PDF<ShortcutHint id="run" /></Button>
      </>}
      {exportReason && <p id="scene-export-reason" role="status" className="scene-export-reason"><strong>{exportReason.title}</strong> {exportReason.detail}</p>}
    </div>
    {error && <p role="alert" className="scene-error">{error}</p>}
    {renderError && <p role="alert" className="scene-error">{renderError} Export is disabled until the preview can be verified.</p>}
    {document?.signed && signatureNoticeDismissedFor !== document.path && <Card role="status" className="scene-signature-notice flex-row items-start gap-2.5 px-3 py-2.5 shadow-none">
      <TriangleAlert aria-hidden="true" className="scene-signature-notice-icon size-4 shrink-0" />
      <p className="flex-1 text-sm text-muted-foreground"><strong className="font-semibold text-foreground">This PDF is digitally signed.</strong> Any copy you export won't be signed any more. The original keeps its signature.</p>
      <Button variant="ghost" size="sm" aria-label="Dismiss signature notice" onClick={() => setSignatureNoticeDismissedFor(document.path)}>Got it</Button>
    </Card>}
    {!page ? <Card className="scene-empty"><CardHeader className="my-auto items-center gap-3 text-center">
      {openFailure ? <>
        <div role="alert" className="grid gap-1.5">
          <CardTitle role="heading" aria-level={2}>{openFailure.kind === 'locked' ? `${openFailure.name} is password-protected` : `Toolbox can’t read ${openFailure.name}`}</CardTitle>
          <CardDescription>{openFailure.kind === 'locked'
            ? 'Remove its password with File Security first, then open the unlocked copy here.'
            : 'The file may be damaged, or it isn’t a PDF that Toolbox supports. Nothing was changed. If another app can open it, save a new copy from there and try that.'}</CardDescription>
        </div>
        <div className="flex justify-center gap-2">
          {openFailure.kind === 'locked' && onNavigate && path && availability.allows('lock-unlock') && <Button onClick={() => onNavigate({ utilityId: 'lock-unlock', initialPaths: [path] })}>Open File Security</Button>}
          <Button variant="outline" onClick={() => void onBrowse()}>Open another PDF<ShortcutHint id="open" /></Button>
        </div>
        <p role="status" className="scene-property-note">Couldn’t open {openFailure.name}. {openFailure.kind === 'locked' ? 'It’s password-protected.' : 'It may be damaged.'}</p>
      </> : loading ? <CardTitle role="heading" aria-level={2}>Opening PDF…</CardTitle> : tool === 'organize' ? <>
        <CardTitle role="heading" aria-level={2}>Open a PDF to organize its pages</CardTitle>
        <CardDescription>Reorder, rotate, mirror and remove pages. When you export, Toolbox saves a new copy and never changes your original.</CardDescription>
        <div className="flex justify-center"><Button onClick={() => void onBrowse()}>Open PDF</Button></div>
        <CardDescription>Or drop one PDF anywhere in this window.</CardDescription>
        {onNavigate && availability.allows('pdf-merge') && <Card className="py-0 text-left"><CardContent className="flex items-center gap-3 p-3 text-sm">
          <span className="flex-1 text-muted-foreground">Organize works on one PDF at a time. To combine several PDFs, use Merge first, then organize the result here.</span>
          <Button variant="outline" size="sm" onClick={() => onNavigate({ utilityId: 'pdf-merge', initialPaths: [] })}>Open Merge</Button>
        </CardContent></Card>}
      </> : <>
        <CardTitle role="heading" aria-level={2}>Open a PDF to edit</CardTitle>
        <CardDescription>Text, markup, signatures and page edits in one document.<br />Files stay on your device. Export creates a new copy.</CardDescription>
      </>}
    </CardHeader></Card> : tool === 'organize' ? <div className="scene-organize">
      <div className="scene-organize-bar">
        <div className="scene-organize-document">
          <h2>{documentName}</h2>
          <p>{pageCount}{dirty ? '. Changes not exported' : ''}</p>
        </div>
        <div className="scene-organize-size">
          <span id="scene-thumbnail-size-label">Thumbnails</span>
          <ToggleGroup type="single" variant="outline" size="sm" aria-labelledby="scene-thumbnail-size-label" value={thumbnailSize}
            onValueChange={(value) => { if (value) setThumbnailSize(value as ThumbnailSize); }}>
            <ToggleGroupItem value="small">Small</ToggleGroupItem>
            <ToggleGroupItem value="medium">Medium</ToggleGroupItem>
            <ToggleGroupItem value="large">Large</ToggleGroupItem>
          </ToggleGroup>
        </div>
      </div>
      <div className="scene-organize-body">
        <div ref={organizeGrid} className="scene-organize-grid" role="listbox" aria-label={`Pages of ${documentName}, in order`} data-size={thumbnailSize} onKeyDown={gridKeyDown}>
          {scene.pages.map((item, index) => {
            const position = index + 1;
            const original = item.sourceIndex === null ? null : item.sourceIndex + 1;
            const cached = thumbnails[item.id];
            const thumbnailKey = path ? previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
            const thumb = cached?.key === thumbnailKey ? cached.value.dataUrl : item.sourceIndex === null ? null : document?.pages[item.sourceIndex]?.preview;
            const focused = item.id === page.id;
            return <Button key={item.id} role="option" aria-selected={focused} tabIndex={focused ? 0 : -1} data-page-index={index}
              variant={focused ? 'secondary' : 'ghost'} className="scene-organize-page h-auto flex-col gap-2 p-2"
              aria-label={original === null ? `Position ${position}` : `Position ${position}, original page ${original}`} onClick={() => selectPage(item.id)}>
              <span className="scene-organize-thumb">{thumb && <img alt="" src={thumb} />}</span>
              <span className="scene-organize-label"><strong>{position}</strong>{original !== null && original !== position && <span> · was page {original}</span>}</span>
            </Button>;
          })}
        </div>
        <aside className="scene-organize-focus" aria-label="Preview of the focused page">
          <div className="scene-organize-focus-heading"><strong>Page {currentIndex + 1}</strong><span>of {scene.pages.length}</span></div>
          <div className="scene-organize-focus-frame">
            {exactPreview ? <img alt={`Preview of page ${currentIndex + 1}`} src={exactPreview.dataUrl} /> : <span>{rendering ? 'Rendering on device…' : 'No preview yet'}</span>}
          </div>
          <dl className="scene-organize-facts">
            <dt>Position</dt><dd>{currentIndex + 1} of {scene.pages.length}</dd>
            <dt>Original page</dt><dd>{page.sourceIndex === null ? 'None' : page.sourceIndex + 1}</dd>
            <dt>Orientation</dt><dd>{page.rotation ? `Rotated ${page.rotation}° right` : 'As in the original'}</dd>
          </dl>
          <Button variant="outline" size="sm" disabled={!pageViewTool} onClick={() => { if (pageViewTool) { endGroup(); setTool(pageViewTool); } }}>Open in page view</Button>
        </aside>
      </div>
      <div className="scene-organize-status">
        <span role="status">Showing {pageCount}. Previews load as you scroll.</span>
        <span className="scene-original-note">Your original is never changed</span>
      </div>
    </div> : <>
      <div className="scene-document">
        <Card className="scene-thumbnails py-0" role="complementary" aria-label="PDF page thumbnails">
          <CardContent className="scene-thumbnails-content p-2">
          <div className="scene-page-actions">
            {availability.allows('pdf-remove-pages') && <Button variant="destructive" size="icon-sm" aria-label="Delete selected pages" title="Delete selected pages" disabled={targets.length >= scene.pages.length} onClick={deletePages}>−</Button>}
            <PageActionsMenu sourcePath={path} onNavigate={onNavigate} />
          </div>
          {scene.pages.map((item, index) => {
            const cached = thumbnails[item.id];
            const thumbnailKey = path ? previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
            const thumb = cached?.key === thumbnailKey ? cached.value.dataUrl : item.sourceIndex === null ? null : document?.pages[item.sourceIndex]?.preview;
            return <Button key={item.id} variant={targets.includes(item.id) ? "secondary" : "ghost"} size="sm" className="scene-thumbnail h-auto min-h-0" aria-label={'Page ' + (index + 1)}
              aria-current={page.id === item.id ? 'page' : undefined} aria-pressed={targets.includes(item.id)} draggable={availability.allows('pdf-organize')}
              onDragStart={() => { if (availability.allows('pdf-organize')) draggedPage.current = item.id; }} onDragEnd={() => { draggedPage.current = null; }} onDragOver={(event) => { if (availability.allows('pdf-organize')) event.preventDefault(); }} onDrop={() => movePages(item.id)}
              onClick={(event) => selectPage(item.id, event.metaKey || event.ctrlKey, event.shiftKey)}>
              {thumb ? <img alt={`Thumbnail of page ${index + 1}`} src={thumb} /> : <span className="scene-thumb-placeholder" />}
              <span>{index + 1}</span><small>{markCount(index) ? `${markCount(index)} marks` : ''}</small>
            </Button>;
          })}
          </CardContent>
        </Card>
        <div className="scene-canvas-column">
          <SceneCanvas page={page} sourcePreview={page.sourceIndex === null ? null : document?.pages[page.sourceIndex]?.preview ?? null}
            textRuns={page.sourceIndex === null ? [] : document?.pages[page.sourceIndex]?.textRuns ?? []}
            renderedPreview={exactPreview} tool={tool ?? 'select'} selectedId={selectedId} zoom={zoom} onSelect={(id) => { endGroup(); setSelectedId(id); }}
            onCommit={(next) => { if (canEditCanvas) commitPage(next); }} makeObject={makeObject}
            redactionFill={redactionFill} redactionPreview={tool === 'redact' && reviewing && previewRedactions}
            formFields={form?.unsupported ? [] : form?.fields} formValue={formValue}
            onFormValue={(field, value) => { if (availability.allows('pdf-edit') && !field.readOnly) setFormEntries((entries) => ({ ...entries, [field.name]: value })); }}
            formFlag={formFlag} onFormFocus={(field) => setFormFocus(field.name)} onFormLeave={leaveFormField}
            editRuns={editRuns} textEdits={pageEdits} selectedRun={selectedRun} onSelectRun={setSelectedRun} onEditRun={setTextEdit}
            watermarks={watermarks} onWatermarkChange={(id, patch) => { if (canEditCanvas) changeObject(id, patch, 'watermark-position'); }} />
        </div>
        <aside className="scene-inspector" aria-label="PDF editor inspector">
          <Card role="region" aria-label="Properties" className="scene-inspector-panel gap-4 overflow-y-auto py-4">
            <CardHeader className="gap-1 px-4 py-0">
              <CardTitle>{selected ? `${selected.kind[0].toUpperCase()}${selected.kind.slice(1)} properties` : tool === 'edit-text' ? 'Edit text' : tool === 'redact' ? 'Redact' : tool === 'form' ? 'Fill form' : 'Properties'}</CardTitle>
              <CardDescription>{selected ? 'Adjust the selected item.' : tool === 'select' ? 'Select a mark to edit its properties.' : tool === 'edit-text' ? toolGuidance['edit-text'] : tool === 'redact' ? 'Drag over anything you want removed. Marks are only previews until you export.' : tool === 'form' ? 'Fill in the fields this PDF already has.' : tool ? `New ${tool} settings` : 'No page markup tools are enabled.'}</CardDescription>
            </CardHeader>
            <CardContent className="scene-inspector-content flex flex-col gap-3 px-4">
              {tool === 'redact' && !selected && <RedactInspector marks={marks} selectedId={selectedId} reviewing={reviewing} preview={previewRedactions} fill={redactionFill}
                onSelect={selectMark} onRemove={removeMarks} onReview={review} onPreview={setPreviewRedactions} onFill={recolorMarks} />}
              {Fields && inspector && <Fields {...inspector} />}
              {selectedKind !== 'watermark' && selectedKind !== 'signature' && (selected || (tool !== null && !['select', 'edit-text', 'crop', 'redact', 'form'].includes(tool))) && <>
                <div className="scene-inspector-field"><Label htmlFor="scene-object-color">Object color</Label>
                  <input id="scene-object-color" type="color" aria-label="Object color" value={activeColor} onChange={(event) => {
                    if (selected) updateObject({ color: event.target.value }, 'color'); else if (tool === 'highlight') setHighlightColor(event.target.value); else setColor(event.target.value);
                  }} onBlur={endGroup} />
                </div>
                <div className="scene-inspector-field">
                  <div className="scene-inspector-slider-label"><Label id="scene-object-opacity-label">Object opacity</Label><output>{Math.round(activeOpacity * 100)}%</output></div>
                  <Slider aria-labelledby="scene-object-opacity-label" min={5} max={100} value={[Math.round(activeOpacity * 100)]}
                    onValueChange={([value]) => { if (value === undefined) return; const opacity = value / 100; if (selected) updateObject({ opacity }, 'opacity'); else if (tool === 'highlight') setHighlightOpacity(opacity); else setOpacity(opacity); }} />
                </div>
              </>}
              {selectedEnabled && selected && <div className="scene-inspector-actions">
                {selected.kind !== 'watermark' && <Button variant="outline" size="sm" onClick={duplicate}>Duplicate<ShortcutHint id="duplicate" /></Button>}
                <Button variant="destructive" size="sm" onClick={() => changeObject(selected.id, null, 'remove')}>{selected.kind === 'watermark' ? 'Remove watermark' : 'Delete object'}<ShortcutHint id="deleteMark" /></Button>
                {selected.kind !== 'watermark' && <Button variant="outline" size="sm" onClick={() => { if (availability.allows(toolFlagByObjectKind[selected.kind])) commitPage({ ...page, objects: [...page.objects.filter((item) => item.id !== selected.id), selected] }); }}>Bring to front</Button>}
              </div>}
              {selectedEnabled && selected && selected.kind !== 'watermark' && <p className="scene-property-note">Arrow keys nudge it, Shift+arrow nudges further.</p>}
              {Guide && inspector && <Guide {...inspector} />}
              {placed.length > 0 && <section className="scene-inspector-section" aria-labelledby="scene-object-list-title">
                <h3 id="scene-object-list-title" className="scene-inspector-heading">Objects in this document<span>{placed.length}</span></h3>
                <ul className="scene-object-list">
                  {placed.map(({ pageId, number, object }) => <li key={object.id}>
                    <Button variant={selectedId === object.id ? 'secondary' : 'ghost'} size="sm" className="scene-object-list-item w-full justify-start"
                      aria-label={`${kindLabels[object.kind]}, page ${number}: ${objectText(object)}`} aria-pressed={selectedId === object.id}
                      onClick={() => { selectPage(pageId); setSelectedId(object.id); }}>
                      <span className="scene-object-list-page">Page {number}</span><span className="truncate">{objectText(object)}</span>
                    </Button>
                  </li>)}
                </ul>
              </section>}
              <section className="scene-inspector-section" aria-labelledby="scene-export-summary-title">
                <h3 id="scene-export-summary-title" className="scene-inspector-heading">Export summary</h3>
                {contributors.length ? <ol className="scene-export-summary" aria-label="Changes, applied in this fixed order">
                  {contributors.map((item) => <li key={item.id}>{item.label}</li>)}
                </ol> : <p className="scene-property-note">No changes to export yet.</p>}
              </section>
              {tool === 'select' && !selected && <p className="scene-property-note">Select an object on the page to move it or adjust its properties.</p>}
              <p className="scene-render-status" role="status">{rendering ? 'Rendering on device…' : exactPreview ? 'Export preview · on device' : 'Local document'}</p>
            </CardContent>
          </Card>
        </aside>
      </div>
      <RedactDisclosure marks={marks} open={disclosure} onOpenChange={setDisclosure}
        onReview={() => { setDisclosure(false); setTool('redact'); review(true); }}
        onContinue={() => { setDisclosure(false); void pendingExport.current(); }} />
      <div ref={bottomToolbar} className="scene-bottom-bar" role="toolbar" aria-label="PDF page and zoom controls" aria-orientation="horizontal" onFocusCapture={handleToolbarFocus} onKeyDown={handleToolbarKeyDown}>
        <Button variant="ghost" size="icon-sm" aria-label="Previous page" disabled={currentIndex <= 0} onClick={() => selectPage(scene.pages[currentIndex - 1].id)}>‹</Button>
        <span>Page {currentIndex + 1} of {scene.pages.length}</span>
        <Button variant="ghost" size="icon-sm" aria-label="Next page" disabled={currentIndex >= scene.pages.length - 1} onClick={() => selectPage(scene.pages[currentIndex + 1].id)}>›</Button>
        <span className="scene-toolbar-divider" />
        <Button variant="ghost" size="icon-sm" aria-label="Zoom out" disabled={zoom <= 0.5} onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}>−</Button>
        <Button variant="ghost" size="sm" aria-label="Fit page" onClick={() => setZoom(1)}>{Math.round(zoom * 100)}% · Fit</Button>
        <Button variant="ghost" size="icon-sm" aria-label="Zoom in" disabled={zoom >= 3} onClick={() => setZoom((value) => Math.min(3, value + 0.25))}>＋</Button>
        {availability.allows('pdf-organize') && <><Button variant="ghost" size="sm" aria-label="Rotate selected pages" onClick={() => { if (availability.allows('pdf-organize')) commit(rotatePages(scene, targets)); }}>Rotate ↻</Button>
        <Button variant="ghost" size="sm" aria-label="Move page earlier" disabled={currentIndex <= 0} onClick={() => shiftPage(-1)}>Move ←</Button>
        <Button variant="ghost" size="sm" aria-label="Move page later" disabled={currentIndex >= scene.pages.length - 1} onClick={() => shiftPage(1)}>Move →</Button></>}
        {availability.allows('pdf-page-numbers') && <Button variant="ghost" size="sm" onClick={pageNumbers}>Page numbers</Button>}<span className="scene-original-note">{tool === 'form' && formUnavailable && <><span role="status">Fill form is unavailable for this PDF.</span><span className="scene-toolbar-divider" /></>}{tool === 'form' && formNotice && <><span role="status">{formNotice}</span><span className="scene-toolbar-divider" /></>}<span role="status">{changes ? `${changes} ${changes === 1 ? 'change' : 'changes'} not exported · ` : ''}</span>{tool === 'form' ? 'Fields are filled in the copy only' : 'Original unchanged'}</span>
      </div>
    </>}
  </section>;
}

function PageActionsMenu({ sourcePath, onNavigate }: { sourcePath: string | null; onNavigate?: PdfEditorNavigation }) {
  const availability = useToolAvailability();
  const [open, setOpen] = useState(false);
  const [menuPosition, setMenuPosition] = useState<{ top: number; left: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const menu = menuRef.current;
    const items = () => Array.from(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? []);
    items()[0]?.focus();
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !menu?.contains(event.target) && !triggerRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnWheelOutside = (event: WheelEvent) => {
      if (event.target instanceof Node && !menu?.contains(event.target)) setOpen(false);
    };
    const closeOnResize = () => setOpen(false);
    window.addEventListener('pointerdown', closeOutside);
    window.addEventListener('wheel', closeOnWheelOutside, { passive: true });
    window.addEventListener('resize', closeOnResize);
    return () => {
      window.removeEventListener('pointerdown', closeOutside);
      window.removeEventListener('wheel', closeOnWheelOutside);
      window.removeEventListener('resize', closeOnResize);
    };
  }, [open]);

  const close = (restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus) requestAnimationFrame(() => triggerRef.current?.focus());
  };
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const options = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? []);
    const currentIndex = options.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === 'Escape') {
      event.preventDefault();
      close(true);
      return;
    }
    let nextIndex = -1;
    if (event.key === 'ArrowDown') nextIndex = (currentIndex + 1) % options.length;
    if (event.key === 'ArrowUp') nextIndex = (currentIndex - 1 + options.length) % options.length;
    if (event.key === 'Home') nextIndex = 0;
    if (event.key === 'End') nextIndex = options.length - 1;
    if (nextIndex < 0 || !options.length) return;
    event.preventDefault();
    options[nextIndex].focus();
  };
  const navigate = (utilityId: ExistingPdfUtilityId) => {
    if (!sourcePath || !onNavigate || !availability.allows(utilityId)) return;
    close(false);
    onNavigate({ utilityId, initialPaths: [sourcePath] });
  };
  const toggleMenu = () => {
    const bounds = triggerRef.current?.getBoundingClientRect();
    if (bounds) {
      // Open beside the thumbnail rail so the menu never covers the pages it acts on.
      const railEdge = triggerRef.current?.closest('.scene-thumbnails')?.getBoundingClientRect().right ?? bounds.right;
      setMenuPosition({
        top: Math.max(8, Math.min(bounds.bottom + 5, window.innerHeight - 92)),
        left: Math.max(8, Math.min(railEdge + 4, window.innerWidth - 162)),
      });
    }
    setOpen((value) => !value);
  };

  return <>
    <Button ref={triggerRef} variant="ghost" size="icon-sm" aria-label="Page actions" aria-haspopup="menu" aria-expanded={open} aria-controls="pdf-page-actions-menu"
      title="Merge or split this PDF" disabled={!sourcePath || !onNavigate || (!availability.allows('pdf-merge') && !availability.allows('pdf-split'))} onClick={toggleMenu}>⋯</Button>
    {open && menuPosition && typeof document !== 'undefined' && createPortal(
      <div id="pdf-page-actions-menu" className="scene-page-actions-popover fixed z-50" role="menu" aria-label="Page actions" ref={menuRef} onKeyDown={handleKeyDown} style={menuPosition}>
        <Card className="w-40 gap-1 p-1">
          {availability.allows('pdf-merge') && <Button role="menuitem" tabIndex={-1} variant="ghost" size="sm" className="w-full justify-start" onClick={() => navigate('pdf-merge')}>Merge PDF</Button>}
          {availability.allows('pdf-split') && <Button role="menuitem" tabIndex={-1} variant="ghost" size="sm" className="w-full justify-start" onClick={() => navigate('pdf-split')}>Split PDF</Button>}
        </Card>
      </div>, document.body,
    )}
  </>;
}
