import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { TriangleAlert } from 'lucide-react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { RunDestinationControl, WorkspaceSourceAction } from '../components/ToolScaffold';
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
import type { PdfDocument, PdfPage, PdfTextRun } from '../features/pdf-editor/contracts';
import { colorTokens, readColorToken } from '../features/pdf-editor/colorTokens';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { subToolInspectors } from '../features/pdf-editor/SubToolInspectors';
import type { SubToolInspectorProps } from '../features/pdf-editor/SubToolInspectors';
import { combinedSuffix, exportSuffix, planSceneExport, sceneContributors } from '../features/pdf-editor/subtools';
import { proposedSaveName } from '../components/saveStep';
import type { SaveDialog } from '../registry/saveSteps';
import { movePagesBefore, removePages, rotatePages, sceneFromDocument, swapPage, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, WatermarkPattern } from '../features/pdf-editor/scene';
import { PREVIEW_CACHE_MAX_ENTRIES, PREVIEW_RENDER_SETTINGS, createPreviewCache, pdfEditorErrorMessage, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';
import type { GenerationState } from './pdfPreviewHelpers';
import { ShortcutHint } from '../components/ShortcutHint';
import { isPdfSceneTool } from '../registry';
import { pdfEditorBatchTools, type PdfEditorBatchTools } from '../features/pdf-editor/batchTools';
import { PdfEditorBatchSurface } from '../features/pdf-editor/BatchSurface';
import { PDFConversionWorkspaceView } from './PDFConversionWorkspaceView';
import { shortcutLabel, shortcutTarget } from '../shortcuts';

export { createPreviewCache, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';

type IndexedScenePreview = { pageIndex: number; preview: ScenePreview };

const exportAsDialog: SaveDialog = { title: 'Export a copy', extension: 'pdf' };
const exportAsSummary = 'Saves a new copy with your edits. The original isn’t changed, and an existing file is never replaced.';

/** Organize shows the whole page grid. The other tools edit one page on the canvas. Both share one scene. */
type EditorTool = SceneTool | 'organize';
const tools: { id: EditorTool; label: string; glyph: string }[] = [
  { id: 'organize', label: 'Organize', glyph: '▦' },
  { id: 'select', label: 'Select', glyph: '↖' }, { id: 'text', label: 'Text', glyph: 'T' },
  { id: 'highlight', label: 'Highlight', glyph: '▰' }, { id: 'shape', label: 'Shape', glyph: '□' },
  { id: 'signature', label: 'Signature', glyph: '〰' }, { id: 'watermark', label: 'Watermark', glyph: 'W' },
  { id: 'crop', label: 'Crop', glyph: '⌗' },
];
const toolFlagBySceneTool: Record<EditorTool, AtomicToolId> = {
  organize: 'pdf-organize', select: 'pdf-edit', text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', watermark: 'pdf-watermark', crop: 'pdf-crop',
};
const toolFlagByObjectKind: Record<SceneObject['kind'], AtomicToolId> = {
  text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', watermark: 'pdf-watermark',
};
const toolGuidance: Record<EditorTool, string> = {
  organize: 'See every page in order.', select: 'Select, move, or resize an object.', text: 'Drag to add text. Double-click text to edit it.',
  highlight: 'Drag across text or draw a highlight area.', shape: 'Drag to draw a shape.',
  signature: 'Choose a signature, then drag its box onto the page.', watermark: 'Choose a pattern, then add a watermark.',
  crop: 'Drag a rectangle on the page to crop it.',
};
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
type History = { past: PdfScene[]; present: PdfScene; future: PdfScene[] };
export type ExistingPdfUtilityId = 'pdf-merge' | 'pdf-split' | 'pdf-redact' | 'lock-unlock';
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
}: PDFEditorWorkspaceProps) => {
  const availability = useToolAvailability();
  const session = useRef<{ path: string; scene: PdfScene; baseline: PdfScene } | null>(null);
  const requestedTool: EditorTool = utility.id === 'pdf-organize' ? 'organize' : utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  const initialTool = availability.allows(toolFlagBySceneTool[requestedTool])
    ? requestedTool
    : tools.find((item) => availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  // The export name follows the sub-tools that change the document, so the destination's example tracks the edits.
  const [suffix, setSuffix] = useState<string | null>(null);
  return <ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene" initialPaths={initialPaths} onWorkspaceSourceAction={onWorkspaceSourceAction} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange}
    exampleName={(inputPath) => proposedSaveName(inputPath, suffix ?? combinedSuffix, 'pdf')}
    onRun={async (paths, outputLocation, { runId, outputPath }) => {
      const [inputPath] = paths;
      if (!inputPath || !session.current || session.current.path !== inputPath) throw new Error('Select exactly one open PDF before exporting.');
      const { request } = planSceneExport({ paths, scene: session.current.scene, baseline: session.current.baseline, outputLocation, outputPath });
      const results = await invoke<ToolResult>('export_pdf_scene', { runId, request });
      return results.map((result) => result.failure ? { ...result, failure: { ...result.failure, message: pdfEditorErrorMessage(result.failure.message, 'export') } } : result);
    }}>
    {({ files, run, runSavingAs, loading, runDestination, live, browse, replaceFiles }) => <PDFSceneFiles files={files} title={utility.shortTitle ?? utility.title} replaceFiles={replaceFiles} onNavigate={onNavigate}>
      {(inputPath) => <PDFSceneSession key={inputPath ?? 'empty'} path={inputPath} initialTool={initialTool}
        exporting={loading} exportStep={live?.steps[0]} destination={runDestination} onExport={run}
        onExportAs={() => runSavingAs({ dialog: exportAsDialog, suffix: suffix ?? combinedSuffix, summary: exportAsSummary })} onBrowse={browse}
        onNavigate={onNavigate}
        onScene={(path, scene, baseline) => {
          session.current = path && scene && baseline ? { path, scene, baseline } : null;
          setSuffix(scene && baseline ? exportSuffix(sceneContributors(scene, baseline)) : null);
        }} />}
    </PDFSceneFiles>}
  </ToolScaffold>;
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

function PDFSceneSession({ path, initialTool, exporting, exportStep, destination, onExport, onExportAs, onBrowse, onNavigate, onScene }: {
  path: string | null; initialTool: EditorTool | null; exporting: boolean; exportStep: FileProgress | undefined;
  destination: RunDestinationControl; onExport: () => Promise<void>; onExportAs: () => Promise<void>;
  onBrowse: () => Promise<void>; onNavigate?: PdfEditorNavigation;
  onScene: (path: string | null, scene: PdfScene | null, baseline: PdfScene | null) => void;
}) {
  const availability = useToolAvailability();
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [signatureNoticeDismissedFor, setSignatureNoticeDismissedFor] = useState<string | null>(null);
  const [history, setHistory] = useState<History>({ past: [], present: { pages: [] }, future: [] });
  const [tool, setTool] = useState<EditorTool | null>(initialTool);
  const [thumbnailSize, setThumbnailSize] = useState<ThumbnailSize>('medium');
  const [visiblePages, setVisiblePages] = useState<number[]>([]);
  const [openFailure, setOpenFailure] = useState<OpenFailure | null>(null);
  const organizeGrid = useRef<HTMLDivElement>(null);
  useEffect(() => setTool(initialTool), [initialTool]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [selectedPages, setSelectedPages] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const text = 'Text';
  const [fontSize, setFontSize] = useState(18);
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
  const [signatureFont, setSignatureFont] = useState('Satisfy');
  const [watermarkText, setWatermarkText] = useState('DRAFT');
  const [watermarkFontSize, setWatermarkFontSize] = useState(40);
  const [watermarkPattern, setWatermarkPattern] = useState<WatermarkPattern>('across-page');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(Boolean(path));
  const [rendering, setRendering] = useState(false);
  const [preview, setPreview] = useState<{ key: string; value: ScenePreview } | null>(null);
  const [thumbnails, setThumbnails] = useState<Record<string, { key: string; value: ScenePreview }>>({});
  const [renderError, setRenderError] = useState<string | null>(null);
  const group = useRef<string | null>(null);
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
  const selected = page?.objects.find((object) => object.id === selectedId);
  const currentIndex = scene.pages.findIndex((item) => item.id === page?.id);
  const serializedScene = JSON.stringify(scene);
  const previewKey = path ? previewCacheKey(path, currentIndex, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
  const exactPreview = preview?.key === previewKey ? preview.value : null;
  const dirty = document !== null && sceneContributors(scene, sceneFromDocument(document)).length > 0;
  const canExport = Boolean(page) && dirty && !exporting && !renderError && !rendering && Boolean(exactPreview);
  // Why Export is off, said next to it: a copy with no changes would match the original, and a stale preview can't be verified.
  const exportReason = !page || exporting || renderError ? null
    : !dirty ? { title: 'Nothing to export yet', detail: `This document has no changes, so a copy would match the original. Use a tool in the toolbar first. ${shortcutLabel('run')} does nothing until then.` }
      : !exactPreview || rendering ? { title: 'Updating the preview', detail: 'Export turns on once the preview shows your latest edits.' }
        : null;
  const selectedSignature = selected?.kind === 'signature' ? selected : null;
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
    if (!path) { onScene(null, null, null); return; }
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
        setDocument((current) => {
          if (!current || current.path !== path || textGeneration.current.current !== requestGeneration || current.pages[sourceIndex]?.textRuns != null) return current;
          return { ...current, pages: current.pages.map((item, index) => index === sourceIndex ? { ...item, preview: value.preview ?? item.preview ?? null, textRuns } : item) };
        });
      },
      (reason) => setError(pdfEditorErrorMessage(reason, 'open')),
      () => {});
    return () => { textGeneration.current.current += 1; };
  }, [path, document, page?.sourceIndex]);
  useEffect(() => { onScene(path, document ? scene : null, document ? sceneFromDocument(document) : null); }, [path, scene, document]);
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
        () => invoke<IndexedScenePreview[]>('preview_pdf_scene_pages', { request: { path, scene, pageIndices: missing.map(({ index }) => index) } }),
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
  const commitPage = (next: ScenePage) => commit({ pages: scene.pages.map((item) => item.id === next.id ? next : item) });
  const undo = () => { group.current = null; setPreview(null); setHistory((value) => value.past.length ? {
    past: value.past.slice(0, -1), present: value.past[value.past.length - 1], future: [value.present, ...value.future],
  } : value); };
  const redo = () => { group.current = null; setPreview(null); setHistory((value) => value.future.length ? {
    past: [...value.past, value.present], present: value.future[0], future: value.future.slice(1),
  } : value); };
  const reset = () => { if (document) commit(sceneFromDocument(document)); setSelectedId(null); };
  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === 'undo') undo();
      else if (command === 'redo') redo();
      else if (command === 'export' && canExport) void onExport();
    };
    window.addEventListener('toolbox:editor-command', onShellCommand);
    return () => window.removeEventListener('toolbox:editor-command', onShellCommand);
  }, [canExport, onExport, redo, undo]);
  const updateObject = (patch: Partial<SceneObject>, field: string) => {
    if (!selected || !page || !availability.allows(toolFlagByObjectKind[selected.kind])) return;
    commit({ pages: scene.pages.map((item) => item.id === page.id ? { ...page,
      objects: page.objects.map((object) => object.id === selected.id ? { ...object, ...patch } : object),
    } : item) }, `${selected.id}:${field}`);
  };
  const makeObject = (kind: SceneObject['kind'], rect: SceneRect): SceneObject => ({
    id: crypto.randomUUID(), kind, rect,
    text: kind === 'watermark' ? watermarkText : kind === 'signature' ? signatureText : text,
    fontSize: kind === 'watermark' ? watermarkFontSize : kind === 'signature' ? Math.max(18, fontSize) : fontSize,
    color: kind === 'highlight' ? highlightColor : color,
    opacity: kind === 'highlight' ? highlightOpacity : opacity,
    strokes: [],
    shape: kind === 'shape' ? shape : undefined,
    highlightMode: kind === 'highlight' ? 'area' : undefined,
    signatureMode: kind === 'signature' ? signatureMode : undefined,
    signaturePath: kind === 'signature' && signatureMode === 'image' ? signaturePath : undefined,
    signaturePreview: kind === 'signature' && signatureMode === 'image' ? signaturePreview : undefined,
    fontFamily: kind === 'signature' ? signatureFont : undefined,
    watermarkPattern: kind === 'watermark' ? watermarkPattern : undefined,
  });
  const chooseSignatureImage = async () => {
    if (!availability.allows('pdf-sign')) return;
    try {
      const picked = await open({ multiple: false, filters: [{ name: 'Signature image', extensions: ['png', 'jpg', 'jpeg', 'webp', 'tif', 'tiff'] }] });
      if (typeof picked !== 'string') return;
      let previewSource: string | null = null;
      try { previewSource = convertFileSrc(picked); } catch { /* Keep the original path for native export, but fail closed for the preview. */ }
      setSignatureMode('image'); setSignaturePath(picked); setSignaturePreview(previewSource);
      if (selected?.kind === 'signature') updateObject({ signatureMode: 'image', signaturePath: picked, signaturePreview: previewSource }, 'signature-image');
    } catch (reason) { setError(String(reason)); }
  };
  const addFixedWatermark = () => {
    if (!page || !availability.allows('pdf-watermark')) return;
    const b = visibleBounds(page);
    const rect: SceneRect = { x: b.x + b.width * 0.14, y: b.y + b.height * 0.33, width: b.width * 0.72, height: Math.max(40, Math.min(96, b.height * 0.18)) };
    const object = makeObject('watermark', rect);
    commitPage({ ...page, objects: [...page.objects, object] });
    setSelectedId(object.id);
  };
  const selectPage = (id: string, additive = false, range = false) => {
    if (range && page) {
      const other = scene.pages.findIndex((item) => item.id === id);
      setSelectedPages(scene.pages.slice(Math.min(currentIndex, other), Math.max(currentIndex, other) + 1).map((item) => item.id));
    } else setSelectedPages((old) => additive ? old.includes(id) ? old.filter((item) => item !== id) : [...old, id] : [id]);
    setCurrentId(id); setSelectedId(null); group.current = null;
  };
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
    commit({ pages: scene.pages.map((item, index) => {
    const b = visibleBounds(item);
    return { ...item, objects: [...item.objects, { ...makeObject('text', { x: b.x + b.width / 2 - 20, y: b.y + Math.max(0, b.height - 30), width: Math.min(40, b.width), height: Math.min(20, b.height) }), text: String(index + 1), fontSize: 12 }] };
    }) });
  };
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
  const endGroup = () => { group.current = null; };
  const selectedKind = selected?.kind ?? tool;
  const selectedEnabled = Boolean(selected && availability.allows(toolFlagByObjectKind[selected.kind]));
  const canEditCanvas = tool !== null && availability.allows(toolFlagBySceneTool[tool])
    && (!selected || availability.allows(toolFlagByObjectKind[selected.kind]));
  const activeColor = selected?.color ?? (tool === 'highlight' ? highlightColor : color);
  const activeOpacity = selected?.opacity ?? (tool === 'highlight' ? highlightOpacity : opacity);

  const inspector: SubToolInspectorProps | null = page ? {
    page, selected, hasTextRuns: Boolean(document?.pages[page.sourceIndex ?? -1]?.textRuns?.length), updateObject, commitPage, endGroup,
    fontSize, setFontSize, shape, setShape,
    signatureMode, setSignatureMode, signaturePath, signaturePreview, signatureText, setSignatureText,
    signatureTextDraft, setSignatureTextDraft, signatureFont, setSignatureFont, chooseSignatureImage,
    watermarkText, setWatermarkText, watermarkFontSize, setWatermarkFontSize, watermarkPattern, setWatermarkPattern, addFixedWatermark,
  } : null;
  const Fields = selectedKind && selectedKind !== 'organize' ? subToolInspectors[selectedKind]?.Fields : undefined;
  const Guide = tool && tool !== 'organize' ? subToolInspectors[tool]?.Guide : undefined;

  return <section className="pdf-scene-workspace" aria-label="PDF editing session" onKeyDown={(event) => {
    const input = event.target instanceof HTMLElement && (['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName) || event.target.isContentEditable);
    if (!input && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); event.shiftKey ? redo() : undo(); }
  }}>
    <div ref={toolsToolbar} className="scene-tools" role="toolbar" aria-label="PDF editor tools" aria-orientation="horizontal" onFocusCapture={handleToolbarFocus} onKeyDown={handleToolbarKeyDown}>
      {tools.filter((item) => availability.allows(toolFlagBySceneTool[item.id])).map((item) => <Button key={item.id} variant={tool === item.id ? "secondary" : "ghost"} size="sm" aria-label={item.label} aria-description={toolGuidance[item.id]} title={toolGuidance[item.id]} aria-pressed={tool === item.id}
        onClick={() => { if (!availability.allows(toolFlagBySceneTool[item.id])) return; endGroup(); setTool(item.id); setSelectedId(null); }} disabled={!page}><span aria-hidden="true">{item.glyph}</span>{item.label}</Button>)}
      {/* Redaction removes content for good, so it runs on a saved copy in its own screen with a confirmation step. */}
      {availability.allows('pdf-redact') && onNavigate && <Button variant="ghost" size="sm" aria-label="Redact"
        title={dirty ? 'Export or reset your edits before redacting' : 'Black out private details in a copy of this PDF'}
        disabled={!page || !path || Boolean(dirty)} onClick={() => { if (path) onNavigate({ utilityId: 'pdf-redact', initialPaths: [path] }); }}><span aria-hidden="true">▮</span>Redact</Button>}
      <span className="scene-toolbar-divider" />
      <Button variant="ghost" size="icon-sm" aria-label="Undo" title="Undo (⌘Z / Ctrl+Z)" disabled={!history.past.length} onClick={undo}>↶</Button>
      <Button variant="ghost" size="icon-sm" aria-label="Redo" title="Redo (⇧⌘Z / Ctrl+Shift+Z)" disabled={!history.future.length} onClick={redo}>↷</Button>
      <Button variant="destructive" size="sm" aria-label="Reset edits" disabled={!dirty} onClick={reset}>Reset</Button>
      <ExportDestination destination={destination} inputPath={path} />
      {exporting ? <ExportProgress step={exportStep} fileName={path ? fileName(path) : 'PDF'} /> : <>
        <Button variant="outline" size="sm" className="scene-export-as" aria-label="Export As…" disabled={!canExport}
          aria-describedby={exportReason ? 'scene-export-reason' : undefined} onClick={() => void onExportAs()}>Export As…</Button>
        <Button variant="default" size="sm" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!canExport}
          aria-describedby={exportReason ? 'scene-export-reason' : undefined}
          {...shortcutTarget('run')} onClick={() => void onExport()}>Export PDF<ShortcutHint id="run" /></Button>
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
              <span>{index + 1}</span><small>{item.objects.length ? `${item.objects.length} marks` : ''}</small>
            </Button>;
          })}
          </CardContent>
        </Card>
        <div className="scene-canvas-column">
          <SceneCanvas page={page} sourcePreview={page.sourceIndex === null ? null : document?.pages[page.sourceIndex]?.preview ?? null}
            textRuns={page.sourceIndex === null ? [] : document?.pages[page.sourceIndex]?.textRuns ?? []}
            renderedPreview={exactPreview} tool={tool ?? 'select'} selectedId={selectedId} zoom={zoom} onSelect={(id) => { endGroup(); setSelectedId(id); }}
            onCommit={(next) => { if (canEditCanvas) commitPage(next); }} makeObject={makeObject} />
        </div>
        <aside className="scene-inspector" aria-label="PDF editor inspector">
          <Card role="region" aria-label="Properties" className="scene-inspector-panel gap-4 overflow-y-auto py-4">
            <CardHeader className="gap-1 px-4 py-0">
              <CardTitle>{selected ? `${selected.kind[0].toUpperCase()}${selected.kind.slice(1)} properties` : 'Properties'}</CardTitle>
              <CardDescription>{selected ? 'Adjust the selected item.' : tool === 'select' ? 'Select a mark to edit its properties.' : tool ? `New ${tool} settings` : 'No page markup tools are enabled.'}</CardDescription>
            </CardHeader>
            <CardContent className="scene-inspector-content flex flex-col gap-3 px-4">
              {Fields && inspector && <Fields {...inspector} />}
              {(selected || (tool !== null && !['select', 'crop'].includes(tool))) && <>
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
                <Button variant="destructive" size="sm" onClick={() => { if (!availability.allows(toolFlagByObjectKind[selected.kind])) return; commitPage({ ...page, objects: page.objects.filter((item) => item.id !== selected.id) }); setSelectedId(null); }}>Delete object</Button>
                <Button variant="outline" size="sm" onClick={() => { if (availability.allows(toolFlagByObjectKind[selected.kind])) commitPage({ ...page, objects: [...page.objects.filter((item) => item.id !== selected.id), selected] }); }}>Bring to front</Button>
              </div>}
              {Guide && inspector && <Guide {...inspector} />}
              {tool === 'select' && !selected && <p className="scene-property-note">Select an object on the page to move it or adjust its properties.</p>}
              <p className="scene-render-status" role="status">{rendering ? 'Rendering on device…' : exactPreview ? 'Export preview · on device' : 'Local document'}</p>
            </CardContent>
          </Card>
        </aside>
      </div>
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
        {availability.allows('pdf-page-numbers') && <Button variant="ghost" size="sm" onClick={pageNumbers}>Page numbers</Button>}<span className="scene-original-note">Original unchanged</span>
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
