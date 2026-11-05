import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { createPortal } from 'react-dom';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { WorkspaceSourceAction } from '../components/ToolScaffold';
import type { ShellEvent } from '../hooks/useShellBridge';
import type { AtomicToolId, ToolDefinition, ToolError, ToolResult } from '../contracts';
import { useToolAvailability } from '../ToolAvailabilityContext';
import { ArrowLeft, ArrowRight, ArrowDown, ArrowUp, ChevronDown, RefreshCw, RotateCcw, RotateCw, TriangleAlert, Undo2 } from 'lucide-react';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { Popover, PopoverContent, PopoverTrigger } from '../components/ui/popover';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { RadioGroup, RadioGroupItem } from '../components/ui/radio-group';
import { ToggleGroup, ToggleGroupItem } from '../components/ui/toggle-group';
import { fileName } from '../components/intake';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { NativeSelect } from '../components/ui/native-select';
import { Slider } from '../components/ui/slider';
import type { PdfDocument, PdfPage, PdfTextRun } from '../features/pdf-editor/contracts';
import { colorTokens, readColorToken } from '../features/pdf-editor/colorTokens';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { interleaveDuplexScan, movePagesBefore, movePagesBy, movePagesTo, movePagesToGap, removePages, resetPageRotation, reversePages, rotatePages, sceneFromDocument, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, WatermarkPattern } from '../features/pdf-editor/scene';
import { PREVIEW_CACHE_MAX_ENTRIES, PREVIEW_RENDER_SETTINGS, createPreviewCache, pdfEditorErrorMessage, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';
import type { GenerationState } from './pdfPreviewHelpers';
import { ShortcutHint } from '../components/ShortcutHint';
import { isWindows, primaryModifierPressed, shortcutTarget } from '../shortcuts';
import { clearPageSelection, clickPage, extendPageSelection, orientationBadge, orientationLabel, pageOptionName, pagePositionsLabel, rotationAnnouncement, selectAllPages, selectPagesByParity, selectPagesByRange, selectionCountLabel, togglePage } from '../features/pdf-editor/pageSelection';
import type { PageSelection } from '../features/pdf-editor/pageSelection';
import { createSourceThumbnailQueue } from '../features/pdf-editor/sourceThumbnails';
import type { SourceThumbnail, SourceThumbnailQueue } from '../features/pdf-editor/sourceThumbnails';

export { createPreviewCache, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';

type IndexedScenePreview = { pageIndex: number; preview: ScenePreview };
/** A page drag in Organize: the pages it carries, the gap it would drop into, the pointer, and any auto-scroll. */
type PageDrag = { ids: string[]; gap: number | null; x: number; y: number; scrolling: { direction: -1 | 1; pages: number[] } | null };
const scrollBand = 44;
const prefersReducedMotion = () => typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
/** The reading order a two-sided scan ends up in, for example "1, 236, 2, 235…". */
const duplexExample = (count: number) => `1, ${count}, 2, ${count - 1}…`;

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
const signatureFontOptions = [
  { value: 'Satisfy', label: 'Satisfy — cursive' },
  { value: 'Pacifico', label: 'Pacifico — cursive' },
  { value: 'Helvetica-Oblique', label: 'Helvetica italic' },
  { value: 'Times-Italic', label: 'Times italic' },
  { value: 'Courier-Oblique', label: 'Courier italic' },
  { value: 'Helvetica', label: 'Helvetica' },
  { value: 'Times-Roman', label: 'Times' },
  { value: 'Courier', label: 'Courier' },
];
const watermarkFontSizes = [18, 24, 32, 40, 48, 56, 64, 72, 88, 104, 120];
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
type History = { past: PdfScene[]; present: PdfScene; future: PdfScene[] };
export type ExistingPdfUtilityId = 'pdf-merge' | 'pdf-split' | 'pdf-redact' | 'lock-unlock';
export type PdfEditorNavigation = (navigation: {
  utilityId: ExistingPdfUtilityId;
  initialPaths: readonly string[];
}) => void;
type ThumbnailSize = 'small' | 'medium' | 'large';
type OpenFailure = { kind: 'locked' | 'unreadable'; name: string } | { kind: 'tooManyPages' | 'tooLarge'; name: string; message: string };

/** Reads the typed error `inspect_pdf_scene` returns for a file the editor can't open. */
const openFailureFor = (reason: unknown, path: string): OpenFailure | null => {
  if (typeof reason !== 'object' || reason === null) return null;
  const error = reason as ToolError;
  const name = fileName(path);
  if (error.kind === 'limitExceeded') return { kind: error.details.limit === 'pageCount' ? 'tooManyPages' : 'tooLarge', name, message: error.message };
  if (error.kind !== 'invalidInput') return null;
  const locked = error.pointer?.kind === 'tool' && error.pointer.toolId === 'lock-unlock';
  return { kind: locked ? 'locked' : 'unreadable', name };
};
const openFailureText = (failure: OpenFailure) => {
  switch (failure.kind) {
    case 'locked': return { title: `${failure.name} is password-protected`, description: 'Remove its password with File Security first, then open the unlocked copy here.', status: 'It’s password-protected.' };
    case 'unreadable': return { title: `Toolbox can’t read ${failure.name}`, description: 'The file may be damaged, or it isn’t a PDF that Toolbox supports. Nothing was changed. If another app can open it, save a new copy from there and try that.', status: 'It may be damaged.' };
    case 'tooManyPages': return { title: `${failure.name} has too many pages`, description: failure.message, status: 'It has too many pages.' };
    case 'tooLarge': return { title: `${failure.name} is too large to open`, description: failure.message, status: 'It’s too large.' };
  }
};
/** A grid placeholder keeps the source page's shape from inspection, turned with the page, so the grid doesn't jump when the thumbnail arrives. */
const thumbnailAspect = ({ width, height, rotation }: ScenePage) => rotation % 180 === 0 ? `${width} / ${height}` : `${height} / ${width}`;
/** Source thumbnails show the original page, so the scene's rotation is drawn in CSS. A quarter turn swaps the image box to fill the frame. */
const thumbnailImageStyle = ({ width, height, rotation }: ScenePage): React.CSSProperties | undefined => {
  if (!rotation) return undefined;
  if (rotation % 180 === 0) return { transform: `rotate(${rotation}deg)` };
  return { position: 'absolute', left: '50%', top: '50%', width: `${(100 * width) / height}%`, height: `${(100 * height) / width}%`, maxWidth: 'none', maxHeight: 'none', transform: `translate(-50%, -50%) rotate(${rotation}deg)` };
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

export const PDFEditorWorkspaceView = ({
  utility,
  initialPaths,
  onNavigate,
  onWorkspaceSourceAction,
  fileActivation,
  onActivationAccepted,
  onFilesChange,
}: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  onNavigate?: PdfEditorNavigation;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
  fileActivation?: Extract<ShellEvent, { kind: 'files' }>;
  onActivationAccepted?: (activationId: string) => void;
  onFilesChange?: (paths: readonly string[]) => void;
}) => {
  const availability = useToolAvailability();
  const session = useRef<{ path: string; scene: PdfScene } | null>(null);
  const requestedTool: EditorTool = utility.id === 'pdf-organize' ? 'organize' : utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  const initialTool = availability.allows(toolFlagBySceneTool[requestedTool])
    ? requestedTool
    : tools.find((item) => availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  return <ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene" initialPaths={initialPaths} onWorkspaceSourceAction={onWorkspaceSourceAction} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange}
    onRun={async (paths, outputLocation, { runId }) => {
      const [inputPath] = paths;
      if (!inputPath || !session.current || session.current.path !== inputPath) throw new Error('Select exactly one open PDF before exporting.');
      const results = await invoke<ToolResult>('export_pdf_scene', { runId, request: { paths, scene: session.current.scene, outputLocation } });
      return results.map((result) => result.failure ? { ...result, failure: { ...result.failure, message: pdfEditorErrorMessage(result.failure.message, 'export') } } : result);
    }}>
    {({ files, run, loading, destination, browse, replaceFiles }) => <>
      <PDFSceneFiles files={files} title={utility.shortTitle ?? utility.title} replaceFiles={replaceFiles} onNavigate={onNavigate}>
        {(inputPath) => <PDFSceneSession key={inputPath ?? 'empty'} path={inputPath} initialTool={initialTool}
          exporting={loading} onExport={run} onBrowse={browse}
          onNavigate={onNavigate}
          onScene={(path, scene) => { session.current = path && scene ? { path, scene } : null; }} />}
      </PDFSceneFiles>
      <Card className="pdf-scene-destination py-0"><CardContent className="py-3">{destination}</CardContent></Card>
    </>}
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

function PDFSceneSession({ path, initialTool, exporting, onExport, onBrowse, onNavigate, onScene }: {
  path: string | null; initialTool: EditorTool | null; exporting: boolean; onExport: () => Promise<void>;
  onBrowse: () => Promise<void>; onNavigate?: PdfEditorNavigation;
  onScene: (path: string | null, scene: PdfScene | null) => void;
}) {
  const availability = useToolAvailability();
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [history, setHistory] = useState<History>({ past: [], present: { pages: [] }, future: [] });
  const [tool, setTool] = useState<EditorTool | null>(initialTool);
  const [thumbnailSize, setThumbnailSize] = useState<ThumbnailSize>('medium');
  const [visiblePages, setVisiblePages] = useState<number[]>([]);
  const [openFailure, setOpenFailure] = useState<OpenFailure | null>(null);
  const organizeGrid = useRef<HTMLDivElement>(null);
  // Organize grid thumbnails are the original pages, loaded as they scroll into view; page edits show as CSS rotation.
  const [, setSourceThumbnailVersion] = useState(0);
  const [sourceThumbnails, setSourceThumbnails] = useState<SourceThumbnailQueue | null>(null);
  useEffect(() => {
    if (!path) { setSourceThumbnails(null); return; }
    const queue = createSourceThumbnailQueue({
      fetch: (pageIndices) => invoke<SourceThumbnail[]>('render_pdf_source_thumbnails', { request: { path, pageIndices } }),
      onChange: () => setSourceThumbnailVersion((version) => version + 1),
    });
    setSourceThumbnails(queue);
    return () => queue.dispose();
  }, [path]);
  useEffect(() => setTool(initialTool), [initialTool]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  // Page selection is view state, not an edit, so it never adds an undo step.
  const [pageSelection, setPageSelection] = useState<PageSelection>(clearPageSelection);
  const [pageRange, setPageRange] = useState('');
  const [pageRangeError, setPageRangeError] = useState<string | null>(null);
  // The footer's polite live region says what the last reorder did. It falls back to the page count.
  const [announcement, setAnnouncement] = useState<string | null>(null);
  const [pageDrag, setPageDrag] = useState<PageDrag | null>(null);
  const [moveTo, setMoveTo] = useState<{ value: string; error: string | null } | null>(null);
  const moveToFromGrid = useRef(false);
  const suppressPageClick = useRef(false);
  const flipFrom = useRef<Map<string, DOMRect> | null>(null);
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
    if (!path) { onScene(null, null); return; }
    invoke<PdfDocument>('inspect_pdf_scene', { request: { path } }).then((value) => {
      if (!active) return;
      if (!value.pages.length) throw new Error('The PDF has no pages.');
      setDocument(value);
      const initial = sceneFromDocument(value);
      setHistory({ past: [], present: initial, future: [] });
      setCurrentId(initial.pages[0].id); setPageSelection(clearPageSelection()); setPageRange(''); setPageRangeError(null); setAnnouncement(null);
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
  useEffect(() => { onScene(path, document ? scene : null); }, [path, scene, document]);
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

  // The organize grid asks for thumbnails of the pages on or near the screen as the person scrolls.
  const pageOrderKey = scene.pages.map((item) => item.id).join(',');
  useEffect(() => {
    const grid = organizeGrid.current;
    if (tool !== 'organize' || !grid || typeof IntersectionObserver === 'undefined') return;
    const shown = new Set<HTMLElement>();
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const cell = entry.target as HTMLElement;
        if (entry.isIntersecting) shown.add(cell); else shown.delete(cell);
      }
      setVisiblePages([...shown].map((cell) => Number(cell.dataset.pageIndex)).sort((a, b) => a - b));
    }, { root: grid, rootMargin: '200px' });
    grid.querySelectorAll('[data-page-index]').forEach((cell) => observer.observe(cell));
    return () => observer.disconnect();
  }, [tool, pageOrderKey, Boolean(page)]);
  const wantedSourcePages = tool === 'organize'
    ? visiblePages.flatMap((index) => { const sourceIndex = scene.pages[index]?.sourceIndex; return sourceIndex === null || sourceIndex === undefined ? [] : [sourceIndex]; })
    : [];
  const wantedSourceKey = wantedSourcePages.join(',');
  useEffect(() => { sourceThumbnails?.want(wantedSourcePages); }, [sourceThumbnails, wantedSourceKey]);

  const commit = (next: PdfScene, mergeGroup: string | null = null) => {
    const merge = Boolean(mergeGroup && group.current === mergeGroup);
    group.current = mergeGroup;
    setHistory((previous) => same(previous.present, next) ? previous : ({
      past: merge ? previous.past : [...previous.past, previous.present].slice(-100), present: next, future: [],
    }));
    setPreview(null);
  };
  const commitPage = (next: ScenePage) => commit({ pages: scene.pages.map((item) => item.id === next.id ? next : item) });
  const undo = () => { group.current = null; setPreview(null); setAnnouncement(null); setHistory((value) => value.past.length ? {
    past: value.past.slice(0, -1), present: value.past[value.past.length - 1], future: [value.present, ...value.future],
  } : value); };
  const redo = () => { group.current = null; setPreview(null); setAnnouncement(null); setHistory((value) => value.future.length ? {
    past: [...value.past, value.present], present: value.future[0], future: value.future.slice(1),
  } : value); };
  const reset = () => { if (document) commit(sceneFromDocument(document)); setSelectedId(null); setAnnouncement(null); };
  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === 'undo') undo();
      else if (command === 'redo') redo();
      else if (command === 'export' && page && !exporting && !renderError && !rendering && exactPreview) void onExport();
    };
    window.addEventListener('toolbox:editor-command', onShellCommand);
    return () => window.removeEventListener('toolbox:editor-command', onShellCommand);
  }, [exactPreview, exporting, onExport, page, redo, renderError, rendering, undo]);
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
  const pageOrder = scene.pages.map((item) => item.id);
  const selectPage = (id: string, toggle = false, range = false) => {
    setPageSelection(clickPage(pageOrder, pageSelection, id, { toggle, range, focused: page?.id ?? null }));
    setCurrentId(id); setSelectedId(null); group.current = null;
  };
  const replacePageSelection = (next: PageSelection) => { setPageSelection(next); setPageRangeError(null); };
  const applyPageRange = () => {
    const result = selectPagesByRange(pageOrder, pageRange);
    if (result && 'message' in result) setPageRangeError(result.message);
    else if (result) replacePageSelection(result);
    else setPageRangeError(null);
  };
  const pageTargets = pageOrder.filter((id) => pageSelection.ids.includes(id));
  const targets = pageTargets.length ? pageTargets : page ? [page.id] : [];
  const deletePages = () => {
    if (!availability.allows('pdf-remove-pages')) return;
    const next = removePages(scene, targets);
    if (!next) return;
    commit(next); setCurrentId(next.pages[Math.min(currentIndex, next.pages.length - 1)].id); setPageSelection(clearPageSelection()); setSelectedId(null);
  };
  const movePages = (targetId: string) => {
    if (!availability.allows('pdf-organize')) return;
    const dragged = draggedPage.current; draggedPage.current = null;
    if (!dragged || dragged === targetId) return;
    commit(movePagesBefore(scene, targets.includes(dragged) ? targets : [dragged], targetId));
  };
  // Page view's Move buttons step the selected pages (or the current page) as one block, like Organize.
  const shiftPage = (delta: -1 | 1) => {
    if (!availability.allows('pdf-organize')) return;
    commit(movePagesBy(scene, targets, delta));
  };
  const pageNumbers = () => {
    if (!availability.allows('pdf-page-numbers')) return;
    commit({ pages: scene.pages.map((item, index) => {
    const b = visibleBounds(item);
    return { ...item, objects: [...item.objects, { ...makeObject('text', { x: b.x + b.width / 2 - 20, y: b.y + Math.max(0, b.height - 30), width: Math.min(40, b.width), height: Math.min(20, b.height) }), text: String(index + 1), fontSize: 12 }] };
    }) });
  };
  const dirty = document && !same(scene, sceneFromDocument(document));
  const pageViewTool = tools.find((item) => item.id !== 'organize' && availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  // Moving through the grid moves focus only. Selection changes on click, Space, Shift with an arrow key, or the Select buttons.
  const focusGridPage = (index: number, extend: boolean) => {
    const target = scene.pages[Math.max(0, Math.min(scene.pages.length - 1, index))];
    if (!target) return;
    if (extend) setPageSelection(extendPageSelection(pageOrder, pageSelection, page?.id ?? null, target.id));
    setCurrentId(target.id); setSelectedId(null); group.current = null;
    requestAnimationFrame(() => organizeGrid.current?.querySelector<HTMLButtonElement>(`[data-page-index="${scene.pages.indexOf(target)}"]`)?.focus());
  };
  const gridColumns = () => {
    const options = Array.from(organizeGrid.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? []);
    const firstRow = options.findIndex((option) => option.offsetTop !== options[0].offsetTop);
    return firstRow > 0 ? firstRow : Math.max(1, options.length);
  };
  const gridKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.altKey || !page) return;
    const primary = primaryModifierPressed(event);
    if (primary && event.key.toLowerCase() === 'a') {
      event.preventDefault(); replacePageSelection(selectAllPages(pageOrder)); return;
    }
    // {mod} with Left or Right steps the selected pages. ⌘↑ / ⌘↓ (Ctrl+Home / Ctrl+End on Windows) sends them to the start or end.
    if (primary && !event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
      event.preventDefault(); if (pageTargets.length) stepPages(event.key === 'ArrowLeft' ? -1 : 1); return;
    }
    const edge = !primary || event.shiftKey ? null : isWindows()
      ? (event.key === 'Home' ? 'start' : event.key === 'End' ? 'end' : null)
      : (event.key === 'ArrowUp' ? 'start' : event.key === 'ArrowDown' ? 'end' : null);
    if (edge) { event.preventDefault(); if (pageTargets.length) movePagesToEdge(edge); return; }
    // {mod}+R turns the selected pages right, Shift+{mod}+R turns them left. It never reloads the window.
    if (primary && event.key.toLowerCase() === 'r') { event.preventDefault(); rotateSelection(event.shiftKey ? -90 : 90); return; }
    if (event.metaKey || event.ctrlKey) return;
    if (event.key.toLowerCase() === 'm' && !event.shiftKey) {
      if (pageTargets.length && canReorder) { event.preventDefault(); openMoveTo(true); }
      return;
    }
    if (event.key === 'Escape') {
      // With nothing selected, Escape keeps its usual meaning and leaves the workspace.
      if (pageSelection.ids.length) { event.preventDefault(); replacePageSelection(clearPageSelection()); }
      return;
    }
    if (event.key === ' ') { event.preventDefault(); replacePageSelection(togglePage(pageOrder, pageSelection, page.id)); return; }
    if (event.key === 'Enter') { event.preventDefault(); if (pageViewTool) { endGroup(); setTool(pageViewTool); } return; }
    const next = event.key === 'ArrowRight' ? currentIndex + 1 : event.key === 'ArrowLeft' ? currentIndex - 1
      : event.key === 'ArrowDown' ? currentIndex + gridColumns() : event.key === 'ArrowUp' ? currentIndex - gridColumns()
        : event.key === 'Home' ? 0 : event.key === 'End' ? scene.pages.length - 1 : null;
    if (next === null) return;
    event.preventDefault();
    if (next < 0 || next >= scene.pages.length) return;
    focusGridPage(next, event.shiftKey);
  };
  // A focused button activates on Space key-up in some engines; the grid already handled Space on key-down.
  const gridKeyUp = (event: ReactKeyboardEvent<HTMLDivElement>) => { if (event.key === ' ') event.preventDefault(); };
  // Page edits in Organize: order and orientation. Each change is one undo step. Pages keep their ids, so the selection and focus follow them.
  const canReorder = Boolean(page) && !exporting && availability.allows('pdf-organize');
  const positionsOf = (ids: string[]) => scene.pages.flatMap((item, index) => ids.includes(item.id) ? [index + 1] : []);
  const gridCells = () => Array.from(organizeGrid.current?.querySelectorAll<HTMLElement>('[data-page-id]') ?? []);
  const reorder = (next: PdfScene, message: string, unchanged: string | null) => {
    if (!canReorder) return;
    if (next === scene) { if (unchanged) setAnnouncement(unchanged); return; }
    const focusInGrid = Boolean(organizeGrid.current?.contains(window.document.activeElement));
    flipFrom.current = new Map(gridCells().map((cell) => [cell.dataset.pageId!, cell.getBoundingClientRect()]));
    commit(next); setAnnouncement(message);
    // React moves the focused option in the DOM, which drops focus, so put it back on the same page.
    if (focusInGrid && page) requestAnimationFrame(() => organizeGrid.current?.querySelector<HTMLElement>(`[data-page-id="${page.id}"]`)?.focus());
  };
  const moveSelection = (next: PdfScene, where: string | null, ids = pageTargets) => {
    const label = pagePositionsLabel(positionsOf(ids));
    const start = next.pages.findIndex((item) => ids.includes(item.id)) + 1;
    reorder(next, `${label} moved to position ${start}`, where && `${label} ${ids.length === 1 ? 'is' : 'are'} already ${where}`);
  };
  const stepPages = (delta: -1 | 1) => moveSelection(movePagesBy(scene, pageTargets, delta), delta < 0 ? 'first' : 'last');
  const movePagesToEdge = (edge: 'start' | 'end') =>
    moveSelection(movePagesTo(scene, pageTargets, edge === 'start' ? 1 : scene.pages.length), edge === 'start' ? 'first' : 'last');
  // A turn adds to each selected page's rotation; export adds that to the page's own rotation. Thumbnails turn in CSS, with no new render.
  const backToOpened = (next: PdfScene) => document && same(next, sceneFromDocument(document)) ? '. It’s back to how it was opened.' : '';
  const rotateSelection = (degrees: 90 | 180 | -90) => {
    if (!pageTargets.length) return;
    const next = rotatePages(scene, pageTargets, degrees);
    reorder(next, rotationAnnouncement(positionsOf(pageTargets), degrees) + backToOpened(next), null);
  };
  const resetOrientation = () => {
    if (!pageTargets.length) return;
    const label = pagePositionsLabel(positionsOf(pageTargets));
    const next = resetPageRotation(scene, pageTargets);
    reorder(next, `${label} turned back to how ${pageTargets.length === 1 ? 'it was' : 'they were'} in the original${backToOpened(next)}`,
      `${label} ${pageTargets.length === 1 ? 'is' : 'are'} already as in the original`);
  };
  const reverseSelection = () => reorder(reversePages(scene, pageTargets), `${pagePositionsLabel(positionsOf(pageTargets))} reversed`, null);
  const reverseDocument = () => reorder(reversePages(scene, pageOrder), `All ${pageCount} reversed`, null);
  const interleaveScan = () => reorder(interleaveDuplexScan(scene), `Pages put in reading order: ${duplexExample(scene.pages.length)}`, null);
  const openMoveTo = (fromGrid: boolean) => {
    if (!canReorder || !pageTargets.length) return;
    moveToFromGrid.current = fromGrid;
    setMoveTo({ value: '1', error: null });
  };
  const submitMoveTo = () => {
    if (!moveTo) return;
    const position = /^\d+$/.test(moveTo.value.trim()) ? Number(moveTo.value.trim()) : NaN;
    if (!(position >= 1 && position <= scene.pages.length)) {
      setMoveTo({ ...moveTo, error: `Type a position from 1 to ${scene.pages.length}.` });
      return;
    }
    setMoveTo(null);
    moveSelection(movePagesTo(scene, pageTargets, position), `at position ${position}`);
  };
  const focusGridAfterPopup = (event: Event) => {
    if (!moveToFromGrid.current) return;
    moveToFromGrid.current = false;
    event.preventDefault();
    if (page) organizeGrid.current?.querySelector<HTMLElement>(`[data-page-id="${page.id}"]`)?.focus();
  };
  // A reorder slides each page from where it was. Reduced motion skips the slide, so pages jump.
  useLayoutEffect(() => {
    const from = flipFrom.current;
    flipFrom.current = null;
    if (!from || prefersReducedMotion()) return;
    for (const cell of gridCells()) {
      const before = from.get(cell.dataset.pageId!);
      const after = cell.getBoundingClientRect();
      if (!before || (before.left === after.left && before.top === after.top)) continue;
      cell.animate([{ transform: `translate(${before.left - after.left}px, ${before.top - after.top}px)` }, { transform: 'none' }], { duration: 150, easing: 'ease-out' });
    }
  }, [scene]);

  // Dragging uses pointer events rather than HTML drag and drop, so it works in every webview, shows a custom
  // stack, scrolls the grid near its edges, and cancels with Escape. Dragging a selected page carries the whole selection.
  const startPageDrag = (event: ReactPointerEvent<HTMLButtonElement>, item: ScenePage) => {
    const grid = organizeGrid.current;
    if (event.button !== 0 || event.shiftKey || event.metaKey || event.ctrlKey || event.altKey || !canReorder || !grid) return;
    const ids = pageTargets.includes(item.id) ? pageTargets : [item.id];
    const origin = { x: event.clientX, y: event.clientY };
    let pointer = origin;
    let active = false;
    let gap: number | null = null;
    let scrolling: PageDrag['scrolling'] = null;
    let frame = 0;
    let lastStep = 0;
    const reduced = prefersReducedMotion();
    const gapAt = () => {
      let best: { index: number; after: boolean; distance: number } | null = null;
      gridCells().forEach((cell, index) => {
        const rect = cell.getBoundingClientRect();
        const dx = Math.max(rect.left - pointer.x, 0, pointer.x - rect.right);
        const dy = Math.max(rect.top - pointer.y, 0, pointer.y - rect.bottom);
        // Rows count for more than columns, so the nearest page is on the pointer's row.
        const distance = Math.hypot(dx, dy * 3);
        if (!best || distance < best.distance) best = { index, after: pointer.x > rect.left + rect.width / 2, distance };
      });
      const nearest = best as { index: number; after: boolean } | null;
      return nearest ? nearest.index + (nearest.after ? 1 : 0) : null;
    };
    const show = () => setPageDrag({ ids, gap, x: pointer.x, y: pointer.y, scrolling });
    const comingIntoView = (direction: -1 | 1) => {
      const rect = grid.getBoundingClientRect();
      const hidden = gridCells().map((cell, index) => ({ index, box: cell.getBoundingClientRect() }))
        // Pages not yet fully in view, so a partly hidden row counts.
        .filter(({ box }) => direction > 0 ? box.bottom > rect.bottom + 1 : box.top < rect.top - 1);
      const row = direction > 0 ? hidden[0]?.box.top : hidden[hidden.length - 1]?.box.top;
      return hidden.filter(({ box }) => box.top === row).map(({ index }) => index + 1);
    };
    // Auto-scroll runs while the pointer is in a 44px band at the grid's top or bottom, faster nearer the edge.
    const tick = (time: number) => {
      const rect = grid.getBoundingClientRect();
      const depth = pointer.y < rect.top + scrollBand ? pointer.y - (rect.top + scrollBand)
        : pointer.y > rect.bottom - scrollBand ? pointer.y - (rect.bottom - scrollBand) : 0;
      const direction = depth < 0 ? -1 : depth > 0 ? 1 : 0;
      const room = direction < 0 ? grid.scrollTop > 0 : direction > 0 ? grid.scrollTop + grid.clientHeight < grid.scrollHeight - 1 : false;
      const next = room ? { direction: direction as -1 | 1, pages: comingIntoView(direction as -1 | 1) } : null;
      if (next) {
        if (!reduced) grid.scrollTop += direction * Math.ceil(Math.min(1, Math.abs(depth) / scrollBand) * 16);
        else if (time - lastStep > 400) { grid.scrollTop += direction * ((gridCells()[0]?.offsetHeight ?? 120) + 12); lastStep = time; }
        gap = gapAt();
      }
      if (next || scrolling) { scrolling = next; show(); }
      frame = requestAnimationFrame(tick);
    };
    const stop = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKey, true);
      cancelAnimationFrame(frame);
      setPageDrag(null);
    };
    // The click that follows a drag must not change the selection.
    const swallowNextClick = () => { suppressPageClick.current = true; window.setTimeout(() => { suppressPageClick.current = false; }, 0); };
    const onMove = (moveEvent: PointerEvent) => {
      pointer = { x: moveEvent.clientX, y: moveEvent.clientY };
      if (!active) {
        if (Math.hypot(pointer.x - origin.x, pointer.y - origin.y) < 5) return;
        active = true;
        if (!pageTargets.includes(item.id)) replacePageSelection({ ids: [item.id], anchor: item.id });
        setCurrentId(item.id); setSelectedId(null); group.current = null;
        frame = requestAnimationFrame(tick);
      }
      moveEvent.preventDefault();
      gap = gapAt();
      show();
    };
    const onUp = () => {
      stop();
      if (!active) return;
      swallowNextClick();
      if (gap !== null) moveSelection(movePagesToGap(scene, ids, gap), null, ids);
    };
    const onCancel = () => { stop(); if (active) setAnnouncement('Move cancelled. Nothing moved'); };
    const onKey = (keyEvent: KeyboardEvent) => {
      if (keyEvent.key !== 'Escape' || !active) return;
      keyEvent.preventDefault(); keyEvent.stopImmediatePropagation();
      stop();
      setAnnouncement('Move cancelled. Nothing moved');
      // The pointer is still down. Its release must not select anything.
      window.addEventListener('pointerup', swallowNextClick, { capture: true, once: true });
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKey, true);
  };
  const dragPreview = pageDrag && pageDrag.gap !== null && movePagesToGap(scene, pageDrag.ids, pageDrag.gap) !== scene ? pageDrag.gap : null;
  const dragStatus = pageDrag && (() => {
    const count = pageDrag.ids.length;
    const moving = `Moving ${count} ${count === 1 ? 'page' : 'pages'}.`;
    const them = count === 1 ? 'it' : 'them';
    const drop = dragPreview === null ? `Drag to another position to move ${them}.`
      : dragPreview >= scene.pages.length ? `Drop to place ${them} at the end.` : `Drop to place ${them} before position ${dragPreview + 1}.`;
    return `${moving} ${drop} Press Esc to cancel.`;
  })();
  const scrollBandText = (direction: -1 | 1) => {
    const word = direction < 0 ? 'up' : 'down';
    const pages = pageDrag?.scrolling?.direction === direction ? pageDrag.scrolling.pages : null;
    return pages?.length ? `Scrolling ${word}. ${pagePositionsLabel(pages)} ${pages.length === 1 ? 'is' : 'are'} coming into view` : `Drag here to scroll ${word}`;
  };
  const moveModifier = isWindows() ? 'Ctrl' : '⌘';
  const startKey = isWindows() ? 'Ctrl+Home' : '⌘↑';
  const endKey = isWindows() ? 'Ctrl+End' : '⌘↓';
  const rotateRightKey = isWindows() ? 'Ctrl+R' : '⌘R';
  const rotateLeftKey = isWindows() ? 'Ctrl+Shift+R' : '⇧⌘R';
  const selectionCount = selectionCountLabel(pageTargets.length);
  // The focused page shows its source thumbnail turned in CSS, so a rotation shows at once without waiting for a render.
  const focusSource = tool === 'organize' && page?.sourceIndex !== null && page?.sourceIndex !== undefined ? sourceThumbnails?.get(page.sourceIndex) : undefined;
  const focusThumbnail = focusSource?.status === 'ready' ? focusSource.value.dataUrl : null;
  const selectionLocked = exporting || !page;
  const documentName = path ? fileName(path) : '';
  const pageCount = `${scene.pages.length} ${scene.pages.length === 1 ? 'page' : 'pages'}`;
  const endGroup = () => { group.current = null; };
  const selectedKind = selected?.kind ?? tool;
  const selectedEnabled = Boolean(selected && availability.allows(toolFlagByObjectKind[selected.kind]));
  const canEditCanvas = tool !== null && availability.allows(toolFlagBySceneTool[tool])
    && (!selected || availability.allows(toolFlagByObjectKind[selected.kind]));
  const activeColor = selected?.color ?? (tool === 'highlight' ? highlightColor : color);
  const activeOpacity = selected?.opacity ?? (tool === 'highlight' ? highlightOpacity : opacity);
  const activeShape = selected?.shape ?? shape;
  const activeSignatureMode = selected?.signatureMode ?? signatureMode;
  const activeSignaturePath = selected?.signaturePath ?? signaturePath;
  const activeSignaturePreview = selected?.signaturePreview ?? signaturePreview;
  const activeSignatureText = selectedSignature ? signatureTextDraft : signatureText;
  const activeSignatureFont = selected?.fontFamily ?? signatureFont;
  const activeWatermarkPattern = selected?.watermarkPattern ?? watermarkPattern;
  const activeWatermarkText = selected?.text ?? watermarkText;
  const activeWatermarkFontSize = selected?.kind === 'watermark' ? selected.fontSize : watermarkFontSize;

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
      <Button variant="default" size="sm" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!page || !dirty || exporting || Boolean(renderError) || rendering || !exactPreview}
        {...shortcutTarget('run')} onClick={() => void onExport()}>{exporting ? 'Exporting…' : 'Export PDF'}<ShortcutHint id="run" /></Button>
    </div>
    {error && <p role="alert" className="scene-error">{error}</p>}
    {renderError && <p role="alert" className="scene-error">{renderError} Export is disabled until the preview can be verified.</p>}
    {!page ? <Card className="scene-empty"><CardHeader className="my-auto items-center gap-3 text-center">
      {openFailure ? <>
        {(openFailure.kind === 'tooManyPages' || openFailure.kind === 'tooLarge') && <TriangleAlert className="scene-empty-warning" aria-hidden="true" />}
        <CardTitle role="heading" aria-level={2}>{openFailureText(openFailure).title}</CardTitle>
        <CardDescription>{openFailureText(openFailure).description}</CardDescription>
        <div className="flex justify-center gap-2">
          {openFailure.kind === 'locked' && onNavigate && path && availability.allows('lock-unlock') && <Button onClick={() => onNavigate({ utilityId: 'lock-unlock', initialPaths: [path] })}>Open File Security</Button>}
          {openFailure.kind === 'tooManyPages' && onNavigate && path && availability.allows('pdf-split') && <Button onClick={() => onNavigate({ utilityId: 'pdf-split', initialPaths: [path] })}>Open Split</Button>}
          <Button variant="outline" onClick={() => void onBrowse()}>Open another PDF</Button>
        </div>
        <p role="status" className="scene-property-note">Couldn’t open {openFailure.name}. {openFailureText(openFailure).status}</p>
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
        <div className="scene-organize-actions" role="toolbar" aria-label={`Page actions, ${selectionCount}`} aria-orientation="horizontal">
          <span role="status" className="scene-organize-count">{selectionCount}</span>
          <div className="scene-organize-orientation" role="group" aria-label="Orientation">
            <Button variant="ghost" size="icon-sm" aria-label={`Rotate left (${rotateLeftKey})`} title={`Rotate left (${rotateLeftKey})`}
              aria-keyshortcuts={isWindows() ? 'Control+Shift+R' : 'Shift+Meta+R'} disabled={!canReorder || !pageTargets.length} onClick={() => rotateSelection(-90)}><RotateCcw aria-hidden="true" /></Button>
            <Button variant="ghost" size="icon-sm" aria-label={`Rotate right (${rotateRightKey})`} title={`Rotate right (${rotateRightKey})`}
              aria-keyshortcuts={isWindows() ? 'Control+R' : 'Meta+R'} disabled={!canReorder || !pageTargets.length} onClick={() => rotateSelection(90)}><RotateCw aria-hidden="true" /></Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-xs" aria-label="More orientation options" disabled={!canReorder || !pageTargets.length}><ChevronDown aria-hidden="true" /></Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-72" aria-label="Orientation">
                <DropdownMenuItem onSelect={() => rotateSelection(90)}><RotateCw aria-hidden="true" />Rotate right 90°<DropdownMenuShortcut>{rotateRightKey}</DropdownMenuShortcut></DropdownMenuItem>
                <DropdownMenuItem onSelect={() => rotateSelection(-90)}><RotateCcw aria-hidden="true" />Rotate left 90°<DropdownMenuShortcut>{rotateLeftKey}</DropdownMenuShortcut></DropdownMenuItem>
                <DropdownMenuItem onSelect={() => rotateSelection(180)}><RefreshCw aria-hidden="true" />Rotate 180°</DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem className="flex-col items-start gap-0.5" onSelect={resetOrientation}>
                  <span className="flex items-center gap-2"><Undo2 aria-hidden="true" />Reset orientation of {pageTargets.length} {pageTargets.length === 1 ? 'page' : 'pages'}</span>
                  <span className="text-xs text-muted-foreground">Turns the selected pages back to how they were in the original. Other changes stay.</span>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
          <span className="scene-toolbar-divider" aria-hidden="true" />
          <Button variant="ghost" size="icon-sm" aria-label={`Move earlier (${moveModifier}+←)`} title={`Move earlier (${moveModifier}+←)`}
            aria-keyshortcuts={isWindows() ? 'Control+ArrowLeft' : 'Meta+ArrowLeft'} disabled={!canReorder || !pageTargets.length} onClick={() => stepPages(-1)}><ArrowLeft aria-hidden="true" /></Button>
          <Button variant="ghost" size="icon-sm" aria-label={`Move later (${moveModifier}+→)`} title={`Move later (${moveModifier}+→)`}
            aria-keyshortcuts={isWindows() ? 'Control+ArrowRight' : 'Meta+ArrowRight'} disabled={!canReorder || !pageTargets.length} onClick={() => stepPages(1)}><ArrowRight aria-hidden="true" /></Button>
          <Popover open={moveTo !== null} onOpenChange={(open) => { if (open) openMoveTo(false); else setMoveTo(null); }}>
            <PopoverTrigger asChild>
              <Button variant={moveTo ? 'secondary' : 'ghost'} size="sm" aria-keyshortcuts="M" disabled={!canReorder || !pageTargets.length}>Move to…</Button>
            </PopoverTrigger>
            <PopoverContent align="start" className="scene-organize-move-to" aria-label={`Move ${pageTargets.length} ${pageTargets.length === 1 ? "page" : "pages"} to a position`} onCloseAutoFocus={focusGridAfterPopup}>
              <form className="scene-organize-move-to-form" noValidate onSubmit={(event) => { event.preventDefault(); submitMoveTo(); }}>
                <Label htmlFor="scene-move-to-position">Move {pageTargets.length ? pagePositionsLabel(positionsOf(pageTargets)).toLowerCase() : 'pages'} to position</Label>
                <div className="scene-organize-move-to-row">
                  <Input id="scene-move-to-position" className="w-20" inputMode="numeric" autoComplete="off" value={moveTo?.value ?? ''}
                    aria-invalid={moveTo?.error ? true : undefined} aria-describedby={moveTo?.error ? 'scene-move-to-error' : 'scene-move-to-hint'}
                    onChange={(event) => setMoveTo({ value: event.target.value, error: null })} />
                  <span className="scene-organize-hint">of {scene.pages.length}</span>
                </div>
                {moveTo?.error && <p id="scene-move-to-error" role="alert" className="scene-organize-move-to-error">{moveTo.error}</p>}
                <div className="scene-organize-move-to-row">
                  <Button type="button" variant="outline" size="sm" onClick={() => { setMoveTo(null); movePagesToEdge('start'); }}>Start<kbd aria-hidden="true" className="shortcut-hint">{startKey}</kbd></Button>
                  <Button type="button" variant="outline" size="sm" onClick={() => { setMoveTo(null); movePagesToEdge('end'); }}>End<kbd aria-hidden="true" className="shortcut-hint">{endKey}</kbd></Button>
                </div>
                <p id="scene-move-to-hint" className="scene-organize-hint">The pages stay together in the same order. Position 1 puts them first.</p>
                <div className="scene-organize-move-to-actions">
                  <Button type="button" variant="ghost" size="sm" onClick={() => setMoveTo(null)}>Cancel</Button>
                  <Button type="submit" size="sm">Move</Button>
                </div>
              </form>
            </PopoverContent>
          </Popover>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" disabled={!canReorder}>Page order<ChevronDown aria-hidden="true" /></Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-80" aria-label="Page order">
              <DropdownMenuItem disabled={pageTargets.length < 2} onSelect={reverseSelection}>
                {pageTargets.length < 2 ? 'Reverse the selected pages' : `Reverse the ${pageTargets.length} selected pages`}
              </DropdownMenuItem>
              <DropdownMenuItem disabled={scene.pages.length < 2} onSelect={reverseDocument}>Reverse the whole document</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem className="flex-col items-start gap-0.5" disabled={scene.pages.length < 3} onSelect={interleaveScan}>
                <span>Interleave a two-sided scan</span>
                <span className="text-xs text-muted-foreground">For fronts scanned first, then backs scanned in reverse. Puts the pages in reading order: {duplexExample(scene.pages.length)}</span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <div className="scene-organize-select">
        <span className="scene-organize-hint" id="scene-select-by-label">Select</span>
        <div className="scene-organize-select-group" role="group" aria-labelledby="scene-select-by-label">
          <Button variant="ghost" size="sm" disabled={selectionLocked} onClick={() => replacePageSelection(selectAllPages(pageOrder))}>All<ShortcutHint id="selectAll" /></Button>
          <Button variant="ghost" size="sm" disabled={selectionLocked} onClick={() => replacePageSelection(selectPagesByParity(pageOrder, 'odd'))}>Odd</Button>
          <Button variant="ghost" size="sm" disabled={selectionLocked} onClick={() => replacePageSelection(selectPagesByParity(pageOrder, 'even'))}>Even</Button>
          <Button variant="ghost" size="sm" disabled={selectionLocked} onClick={() => replacePageSelection(clearPageSelection())}>None<ShortcutHint id="cancel" /></Button>
        </div>
        <span className="scene-toolbar-divider" aria-hidden="true" />
        <div className="scene-organize-range">
          <Label htmlFor="scene-page-range" className="scene-organize-hint">Pages</Label>
          <Input id="scene-page-range" className="scene-organize-range-input h-7" placeholder="For example 1-3, 7" value={pageRange} disabled={selectionLocked}
            aria-invalid={pageRangeError ? true : undefined} aria-describedby={pageRangeError ? 'scene-page-range-error' : undefined}
            onChange={(event) => { setPageRange(event.target.value); setPageRangeError(null); }}
            onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); applyPageRange(); } }} />
          {pageRangeError && <p id="scene-page-range-error" role="alert" className="scene-organize-range-error">{pageRangeError}</p>}
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
        <div className="scene-organize-grid-frame">
        <div ref={organizeGrid} className="scene-organize-grid" role="listbox" aria-multiselectable="true" aria-label={`Pages of ${documentName}, in order`}
          aria-describedby="scene-organize-grid-help" data-size={thumbnailSize} data-dragging={pageDrag ? true : undefined} onKeyDown={gridKeyDown} onKeyUp={gridKeyUp}>
          {scene.pages.map((item, index) => {
            const position = index + 1;
            const original = item.sourceIndex === null ? null : item.sourceIndex + 1;
            const source = item.sourceIndex === null ? null : sourceThumbnails?.get(item.sourceIndex) ?? { status: 'loading' as const };
            const thumb = source?.status === 'ready' ? source.value.dataUrl : null;
            const focused = item.id === page.id;
            const isSelected = pageTargets.includes(item.id);
            const insertion = dragPreview === index ? 'before' : dragPreview === scene.pages.length && index === scene.pages.length - 1 ? 'after' : undefined;
            return <Button key={item.id} role="option" aria-selected={isSelected} tabIndex={focused ? 0 : -1} data-page-index={index} data-page-id={item.id}
              data-moving={pageDrag?.ids.includes(item.id) || undefined}
              variant={isSelected ? 'secondary' : 'ghost'} className="scene-organize-page relative h-auto flex-col gap-2 p-2"
              aria-label={pageOptionName(position, item, isSelected, source && source.status !== 'ready' ? source.status : undefined)}
              onPointerDown={(event) => startPageDrag(event, item)} onDragStart={(event) => event.preventDefault()}
              onClick={(event) => { if (suppressPageClick.current) return; selectPage(item.id, primaryModifierPressed(event), event.shiftKey); }}>
              {insertion && <span className="scene-organize-insertion" data-side={insertion} aria-hidden="true" />}
              <span className="scene-organize-thumb" data-selected={isSelected || undefined} data-placeholder={source && !thumb ? source.status : undefined} style={{ aspectRatio: thumbnailAspect(item) }}>
                {thumb ? <img alt="" src={thumb} style={thumbnailImageStyle(item)} /> : source && <span aria-hidden="true">{position}</span>}
                {isSelected && <span className="scene-organize-check" aria-hidden="true">✓</span>}
                {orientationBadge(item.rotation) && <Badge variant="outline" className="scene-organize-orientation-badge" aria-hidden="true">{orientationBadge(item.rotation)}</Badge>}
              </span>
              <span className="scene-organize-label"><strong>{position}</strong>{original !== null && original !== position && <span> · was page {original}</span>}</span>
            </Button>;
          })}
        </div>
        {pageDrag && <>
          <div className="scene-organize-scroll-band" data-edge="top" data-active={pageDrag.scrolling?.direction === -1 || undefined} aria-hidden="true"><ArrowUp />{scrollBandText(-1)}</div>
          <div className="scene-organize-scroll-band" data-edge="bottom" data-active={pageDrag.scrolling?.direction === 1 || undefined} aria-hidden="true"><ArrowDown />{scrollBandText(1)}</div>
        </>}
        </div>
        <p id="scene-organize-grid-help" className="sr-only">Arrow keys move focus. Space selects. Shift with an arrow key extends the selection. {moveModifier} with an arrow key moves the selected pages. Enter opens the page in page view.</p>
        {pageDrag && createPortal(<div className="scene-organize-ghost" style={{ left: pageDrag.x, top: pageDrag.y }} aria-hidden="true">
          {pageDrag.ids.length > 2 && <span className="scene-organize-ghost-sheet" data-layer="back" />}
          {pageDrag.ids.length > 1 && <span className="scene-organize-ghost-sheet" data-layer="middle" />}
          <span className="scene-organize-ghost-sheet" data-layer="front">{(() => {
            const lead = scene.pages.find((item) => pageDrag.ids.includes(item.id));
            const leadThumbnail = lead && lead.sourceIndex !== null ? sourceThumbnails?.get(lead.sourceIndex) : undefined;
            const src = leadThumbnail?.status === 'ready' ? leadThumbnail.value.dataUrl : null;
            return src ? <img alt="" src={src} /> : null;
          })()}</span>
          <Badge className="scene-organize-ghost-count">{pageDrag.ids.length}</Badge>
        </div>, window.document.body)}
        <aside className="scene-organize-focus" aria-label="Preview of the focused page">
          <div className="scene-organize-focus-heading"><strong>Page {currentIndex + 1}</strong><span>of {scene.pages.length}</span></div>
          <div className="scene-organize-focus-frame">
            {focusThumbnail
              ? <span className="scene-organize-focus-page" data-shape={(page.rotation % 180 ? page.height >= page.width : page.width >= page.height) ? 'wide' : 'tall'} style={{ aspectRatio: thumbnailAspect(page) }}>
                <img alt={`Preview of page ${currentIndex + 1}`} src={focusThumbnail} style={thumbnailImageStyle(page)} />
              </span>
              : exactPreview ? <img alt={`Preview of page ${currentIndex + 1}`} src={exactPreview.dataUrl} /> : <span>{rendering ? 'Loading preview…' : 'No preview yet'}</span>}
          </div>
          <dl className="scene-organize-facts">
            <dt>Position</dt><dd>{currentIndex + 1} of {scene.pages.length}</dd>
            <dt>Original page</dt><dd>{page.sourceIndex === null ? 'None' : page.sourceIndex + 1}</dd>
            <dt>Orientation</dt><dd>{orientationLabel(page.rotation)}</dd>
          </dl>
          <Button variant="outline" size="sm" disabled={!pageViewTool} onClick={() => { if (pageViewTool) { endGroup(); setTool(pageViewTool); } }}>Open in page view</Button>
        </aside>
      </div>
      <div className="scene-organize-status">
        <span role="status" aria-live="polite">{dragStatus ?? announcement ?? `Showing ${pageCount}. Previews load as you scroll.`}</span>
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
              {selectedKind === 'text' && <div className="scene-inspector-field">
                <Label htmlFor="scene-text-size">Font size</Label>
                <Input id="scene-text-size" type="number" min="6" max="144" value={selected?.fontSize ?? fontSize}
                  onChange={(event) => { const value = Math.max(6, Math.min(144, Number(event.target.value) || 6)); selected ? updateObject({ fontSize: value }, 'font') : setFontSize(value); }} onBlur={endGroup} />
              </div>}
              {selectedKind === 'shape' && <div className="scene-inspector-field">
                <Label htmlFor="scene-shape-type">Shape type</Label>
                <NativeSelect id="scene-shape-type" size="sm" value={activeShape}
                  onChange={(event) => selected ? updateObject({ shape: event.target.value as SceneShape }, 'shape') : setShape(event.target.value as SceneShape)}>
                  <option value="square">Square</option><option value="round">Round</option><option value="triangle">Triangle</option>
                  <option value="line">Line</option><option value="dotted-line">Dotted line</option><option value="arrow-left">Arrow left</option>
                  <option value="arrow-right">Arrow right</option><option value="arrow-up">Arrow up</option><option value="arrow-down">Arrow down</option>
                </NativeSelect>
              </div>}
              {selectedKind === 'signature' && <>
                <div className="scene-inspector-field"><Label htmlFor="scene-signature-mode">Signature mode</Label>
                  <NativeSelect id="scene-signature-mode" size="sm" value={activeSignatureMode}
                    onChange={(event) => { const value = event.target.value as SignatureMode; selected ? updateObject({ signatureMode: value, strokes: [] }, 'signature-mode') : setSignatureMode(value); }}>
                    <option value="image">Choose image</option><option value="text">Type signature</option>
                  </NativeSelect>
                </div>
                {activeSignatureMode === 'image' ? <>
                  <Button variant="outline" size="sm" onClick={() => void chooseSignatureImage()}>Choose signature image</Button>
                  <p className="scene-property-note">{activeSignaturePath ? activeSignaturePath.split(/[\\/]/).pop() : 'No image chosen'}</p>
                </> : <>
                  <div className="scene-inspector-field"><Label htmlFor="scene-signature-text">Signature text</Label>
                    <Input id="scene-signature-text" value={activeSignatureText}
                      onChange={(event) => selectedSignature ? setSignatureTextDraft(event.target.value) : setSignatureText(event.target.value)}
                      onBlur={() => { if (selectedSignature && signatureTextDraft !== selectedSignature.text) updateObject({ text: signatureTextDraft }, 'signature-text'); endGroup(); }} />
                  </div>
                  <div className="scene-inspector-field"><Label htmlFor="scene-signature-font">Signature font</Label>
                    <NativeSelect id="scene-signature-font" size="sm" value={activeSignatureFont}
                      onChange={(event) => selected ? updateObject({ fontFamily: event.target.value }, 'signature-font') : setSignatureFont(event.target.value)}>
                      {signatureFontOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                    </NativeSelect>
                  </div>
                </>}
                <div className="scene-inspector-field"><Label htmlFor="scene-signature-size">Font size</Label>
                  <Input id="scene-signature-size" type="number" min="8" max="144" value={selected?.fontSize ?? Math.max(18, fontSize)}
                    onChange={(event) => { const value = Math.max(8, Math.min(144, Number(event.target.value) || 8)); selected ? updateObject({ fontSize: value }, 'signature-font-size') : setFontSize(value); }} onBlur={endGroup} />
                </div>
                {activeSignatureMode === 'image' && activeSignaturePreview && <img className="scene-signature-mini-preview" alt="Selected signature preview" src={activeSignaturePreview} />}
              </>}
              {selectedKind === 'watermark' && <>
                <div className="scene-inspector-field"><Label htmlFor="scene-watermark-text">Watermark text</Label>
                  <Input id="scene-watermark-text" value={activeWatermarkText} onChange={(event) => selected ? updateObject({ text: event.target.value }, 'watermark-text') : setWatermarkText(event.target.value)} onBlur={endGroup} />
                </div>
                <div className="scene-inspector-field"><Label htmlFor="scene-watermark-size">Watermark font size</Label>
                  <NativeSelect id="scene-watermark-size" size="sm" value={String(activeWatermarkFontSize)} onChange={(event) => {
                    const value = Number(event.target.value); selected ? updateObject({ fontSize: value }, 'watermark-font-size') : setWatermarkFontSize(value);
                  }}>{watermarkFontSizes.map((value) => <option key={value} value={value}>{value} pt</option>)}</NativeSelect>
                </div>
                <div className="scene-inspector-field"><Label htmlFor="scene-watermark-pattern">Watermark pattern</Label>
                  <NativeSelect id="scene-watermark-pattern" size="sm" value={activeWatermarkPattern}
                    onChange={(event) => selected ? updateObject({ watermarkPattern: event.target.value as WatermarkPattern }, 'watermark-pattern') : setWatermarkPattern(event.target.value as WatermarkPattern)}>
                    <option value="across-page">Across page</option><option value="bottom-right-to-top-left">Bottom-right to top-left</option>
                    <option value="top-right-to-bottom-left">Top-right to bottom-left</option><option value="center-horizontal">Centre horizontal</option>
                    <option value="center-vertical">Centre vertical</option>
                  </NativeSelect>
                </div>
                {selected && <Button variant="outline" size="sm" onClick={addFixedWatermark}>Add fixed watermark</Button>}
              </>}
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
              {tool === 'crop' && availability.allows('pdf-crop') && <>
                <p className="scene-property-note">Drag a rectangle on the page to crop.</p>
                <Button variant="outline" size="sm" disabled={!page.crop} onClick={() => { if (availability.allows('pdf-crop')) commitPage({ ...page, crop: null }); }}>Remove crop</Button>
              </>}
              {tool === 'select' && !selected && <p className="scene-property-note">Select an object on the page to move it or adjust its properties.</p>}
              {tool === 'highlight' && <p className="scene-property-note">{document?.pages[page.sourceIndex ?? -1]?.textRuns?.length ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</p>}
              {tool === 'signature' && !selected && <p className="scene-property-note">{signatureMode === 'image' ? 'Choose an image, then drag its box onto the page.' : 'Type a signature, choose its font, then drag a box onto the page.'}</p>}
              {tool === 'watermark' && availability.allows('pdf-watermark') && !selected && <>
                <p className="scene-property-note">Fixed placement: choose a pattern and add it to the page.</p>
                <Button size="sm" onClick={addFixedWatermark}>Add fixed watermark</Button>
              </>}
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
        <Button variant="ghost" size="sm" aria-label="Move page earlier" disabled={movePagesBy(scene, targets, -1) === scene} onClick={() => shiftPage(-1)}>Move ←</Button>
        <Button variant="ghost" size="sm" aria-label="Move page later" disabled={movePagesBy(scene, targets, 1) === scene} onClick={() => shiftPage(1)}>Move →</Button></>}
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
