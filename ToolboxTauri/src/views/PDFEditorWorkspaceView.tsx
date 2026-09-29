import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { WorkspaceSourceAction } from '../components/ToolScaffold';
import type { ToolDefinition, ToolResult } from '../contracts';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { NativeSelect } from '../components/ui/native-select';
import { Slider } from '../components/ui/slider';
import type { PdfDocument, PdfPage, PdfTextRun } from '../features/pdf-editor/contracts';
import { colorTokens, readColorToken } from '../features/pdf-editor/colorTokens';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { sceneFromDocument, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, WatermarkPattern } from '../features/pdf-editor/scene';
import { PREVIEW_CACHE_MAX_ENTRIES, PREVIEW_RENDER_SETTINGS, createPreviewCache, pdfEditorErrorMessage, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';
import type { GenerationState } from './pdfPreviewHelpers';

export { createPreviewCache, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';

type IndexedScenePreview = { pageIndex: number; preview: ScenePreview };

const tools: { id: SceneTool; label: string; glyph: string }[] = [
  { id: 'select', label: 'Select', glyph: '↖' }, { id: 'text', label: 'Text', glyph: 'T' },
  { id: 'highlight', label: 'Highlight', glyph: '▰' }, { id: 'shape', label: 'Shape', glyph: '□' },
  { id: 'signature', label: 'Signature', glyph: '〰' }, { id: 'watermark', label: 'Watermark', glyph: 'W' },
  { id: 'crop', label: 'Crop', glyph: '⌗' },
];
const toolGuidance: Record<SceneTool, string> = {
  select: 'Select, move, or resize an object.', text: 'Drag to add text. Double-click text to edit it.',
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
export type ExistingPdfUtilityId = 'pdf-merge' | 'pdf-split';
export type PdfEditorNavigation = (navigation: {
  utilityId: ExistingPdfUtilityId;
  initialPaths: readonly [string, ...string[]];
}) => void;

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

export const PDFEditorWorkspaceView = ({ utility, onNavigate, onWorkspaceSourceAction }: {
  utility: ToolDefinition;
  onNavigate?: PdfEditorNavigation;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
}) => {
  const session = useRef<{ path: string; scene: PdfScene } | null>(null);
  const initialTool: SceneTool = utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  return <ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene" onWorkspaceSourceAction={onWorkspaceSourceAction}
    onRun={async (paths) => {
      const [inputPath] = paths;
      if (!inputPath || !session.current || session.current.path !== inputPath) throw new Error('Select exactly one open PDF before exporting.');
      const results = await invoke<ToolResult>('export_pdf_scene', { request: { paths, scene: session.current.scene, outputLocation: 'alongsideInput' } });
      return results.map((result) => result.failure ? { ...result, failure: { ...result.failure, message: pdfEditorErrorMessage(result.failure.message, 'export') } } : result);
    }}>
    {({ files, run, loading }) => {
      const inputPath = files.length === 1 ? files[0] ?? null : null;
      return <PDFSceneSession key={inputPath ?? 'empty'} path={inputPath} initialTool={initialTool}
      exporting={loading} onExport={run}
      onNavigate={onNavigate}
      onScene={(path, scene) => { session.current = path && scene ? { path, scene } : null; }} />}
    }
  </ToolScaffold>;
};

function PDFSceneSession({ path, initialTool, exporting, onExport, onNavigate, onScene }: {
  path: string | null; initialTool: SceneTool; exporting: boolean; onExport: () => Promise<void>;
  onNavigate?: PdfEditorNavigation;
  onScene: (path: string | null, scene: PdfScene | null) => void;
}) {
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [history, setHistory] = useState<History>({ past: [], present: { pages: [] }, future: [] });
  const [tool, setTool] = useState<SceneTool>(initialTool);
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
  const selectedSignature = selected?.kind === 'signature' ? selected : null;

  useEffect(() => {
    let active = true;
    if (!path) { onScene(null, null); return; }
    invoke<PdfDocument>('inspect_pdf_scene', { request: { path } }).then((value) => {
      if (!active) return;
      if (!value.pages.length) throw new Error('The PDF has no pages.');
      setDocument(value);
      const initial = sceneFromDocument(value);
      setHistory({ past: [], present: initial, future: [] });
      setCurrentId(initial.pages[0].id); setSelectedPages([initial.pages[0].id]);
    }).catch((reason) => { if (active) setError(pdfEditorErrorMessage(reason, 'open')); }).finally(() => { if (active) setLoading(false); });
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
    const updateThumbnail = (item: ScenePage, key: string, value: ScenePreview) => {
      setThumbnails((existing) => {
        const next = { ...existing, [item.id]: { key, value } };
        const staleIds = Object.keys(next).slice(0, Math.max(0, Object.keys(next).length - PREVIEW_CACHE_MAX_ENTRIES));
        for (const staleId of staleIds) delete next[staleId];
        return next;
      });
    };
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
  const updateObject = (patch: Partial<SceneObject>, field: string) => {
    if (!selected || !page) return;
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
    if (!page) return;
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
  const insertPage = () => {
    const blank: ScenePage = { id: crypto.randomUUID(), sourceIndex: null, width: page.width, height: page.height, rotation: 0, crop: null, sourceRotation: null, sourceBox: null, objects: [] };
    const next = [...scene.pages]; next.splice(currentIndex + 1, 0, blank);
    commit({ pages: next }); selectPage(blank.id);
  };
  const deletePages = () => {
    if (targets.length >= scene.pages.length) return;
    const next = scene.pages.filter((item) => !targets.includes(item.id));
    commit({ pages: next }); setCurrentId(next[Math.min(currentIndex, next.length - 1)].id); setSelectedPages([]); setSelectedId(null);
  };
  const movePages = (targetId: string) => {
    const dragged = draggedPage.current; draggedPage.current = null;
    if (!dragged || dragged === targetId) return;
    const ids = targets.includes(dragged) ? targets : [dragged];
    if (ids.includes(targetId)) return;
    const moving = scene.pages.filter((item) => ids.includes(item.id));
    const remaining = scene.pages.filter((item) => !ids.includes(item.id));
    remaining.splice(remaining.findIndex((item) => item.id === targetId), 0, ...moving); commit({ pages: remaining });
  };
  const shiftPage = (delta: number) => {
    const to = currentIndex + delta;
    if (to < 0 || to >= scene.pages.length) return;
    const next = [...scene.pages]; [next[currentIndex], next[to]] = [next[to], next[currentIndex]]; commit({ pages: next });
  };
  const pageNumbers = () => commit({ pages: scene.pages.map((item, index) => {
    const b = visibleBounds(item);
    return { ...item, objects: [...item.objects, { ...makeObject('text', { x: b.x + b.width / 2 - 20, y: b.y + Math.max(0, b.height - 30), width: Math.min(40, b.width), height: Math.min(20, b.height) }), text: String(index + 1), fontSize: 12 }] };
  }) });
  const dirty = document && !same(scene, sceneFromDocument(document));
  const exactPreview = preview?.key === previewKey ? preview.value : null;
  const endGroup = () => { group.current = null; };
  const selectedKind = selected?.kind ?? tool;
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
      {tools.map((item) => <Button key={item.id} variant={tool === item.id ? "secondary" : "ghost"} size="sm" aria-label={item.label} aria-description={toolGuidance[item.id]} title={toolGuidance[item.id]} aria-pressed={tool === item.id}
        onClick={() => { endGroup(); setTool(item.id); setSelectedId(null); }} disabled={!page}><span aria-hidden="true">{item.glyph}</span>{item.label}</Button>)}
      <span className="scene-toolbar-divider" />
      <Button variant="ghost" size="icon-sm" aria-label="Undo" title="Undo (⌘Z / Ctrl+Z)" disabled={!history.past.length} onClick={undo}>↶</Button>
      <Button variant="ghost" size="icon-sm" aria-label="Redo" title="Redo (⇧⌘Z / Ctrl+Shift+Z)" disabled={!history.future.length} onClick={redo}>↷</Button>
      <Button variant="destructive" size="sm" aria-label="Reset edits" disabled={!dirty} onClick={reset}>Reset</Button>
      <Button variant="default" size="sm" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!page || exporting || Boolean(renderError) || rendering || !exactPreview}
        onClick={() => void onExport()}>{exporting ? 'Exporting…' : 'Export PDF'}</Button>
    </div>
    {error && <p role="alert" className="scene-error">{error}</p>}
    {renderError && <p role="alert" className="scene-error">{renderError} Export is disabled until the preview can be verified.</p>}
    {!page ? <Card className="scene-empty"><CardHeader className="my-auto items-center text-center"><CardTitle role="heading" aria-level={2}>{loading ? 'Opening PDF…' : 'Open a PDF to edit'}</CardTitle><CardDescription>Text, markup, signatures and page edits in one document.<br />Files stay on your device. Export creates a new copy.</CardDescription></CardHeader></Card> : <>
      <div className="scene-document">
        <Card className="scene-thumbnails py-0" role="complementary" aria-label="PDF page thumbnails">
          <CardContent className="scene-thumbnails-content p-2">
          <div className="scene-page-actions">
            <Button variant="ghost" size="icon-sm" aria-label="Add blank page" title="Insert blank page after this page" onClick={insertPage}>＋</Button>
            <Button variant="destructive" size="icon-sm" aria-label="Delete selected pages" title="Delete selected pages" disabled={targets.length >= scene.pages.length} onClick={deletePages}>−</Button>
            <PageActionsMenu sourcePath={path} onNavigate={onNavigate} />
          </div>
          {scene.pages.map((item, index) => {
            const cached = thumbnails[item.id];
            const thumbnailKey = path ? previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
            const thumb = cached?.key === thumbnailKey ? cached.value.dataUrl : item.sourceIndex === null ? null : document?.pages[item.sourceIndex]?.preview;
            return <Button key={item.id} variant={targets.includes(item.id) ? "secondary" : "ghost"} size="sm" className="scene-thumbnail h-auto min-h-0" aria-label={'Page ' + (index + 1) + (item.sourceIndex === null ? ', blank' : '')}
              aria-current={page.id === item.id ? 'page' : undefined} aria-pressed={targets.includes(item.id)} draggable
              onDragStart={() => { draggedPage.current = item.id; }} onDragEnd={() => { draggedPage.current = null; }} onDragOver={(event) => event.preventDefault()} onDrop={() => movePages(item.id)}
              onClick={(event) => selectPage(item.id, event.metaKey || event.ctrlKey, event.shiftKey)}>
              {thumb ? <img alt={`Thumbnail of page ${index + 1}`} src={thumb} /> : <span className="scene-blank-thumb" />}
              <span>{index + 1}</span><small>{item.objects.length ? `${item.objects.length} marks` : item.sourceIndex === null ? 'Blank' : ''}</small>
            </Button>;
          })}
          </CardContent>
        </Card>
        <div className="scene-canvas-column">
          <SceneCanvas page={page} sourcePreview={page.sourceIndex === null ? null : document?.pages[page.sourceIndex]?.preview ?? null}
            textRuns={page.sourceIndex === null ? [] : document?.pages[page.sourceIndex]?.textRuns ?? []}
            renderedPreview={exactPreview} tool={tool} selectedId={selectedId} zoom={zoom} onSelect={(id) => { endGroup(); setSelectedId(id); }}
            onCommit={commitPage} makeObject={makeObject} />
        </div>
        <aside className="scene-inspector" aria-label="PDF editor inspector">
          <Card role="region" aria-label="Properties" className="scene-inspector-panel gap-4 overflow-y-auto py-4">
            <CardHeader className="gap-1 px-4 py-0">
              <CardTitle>{selected ? `${selected.kind[0].toUpperCase()}${selected.kind.slice(1)} properties` : 'Properties'}</CardTitle>
              <CardDescription>{selected ? 'Adjust the selected item.' : tool === 'select' ? 'Select a mark to edit its properties.' : `New ${tool} settings`}</CardDescription>
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
              {(selected || !['select', 'crop'].includes(tool)) && <>
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
              {selected && <div className="scene-inspector-actions">
                <Button variant="destructive" size="sm" onClick={() => { commitPage({ ...page, objects: page.objects.filter((item) => item.id !== selected.id) }); setSelectedId(null); }}>Delete object</Button>
                <Button variant="outline" size="sm" onClick={() => commitPage({ ...page, objects: [...page.objects.filter((item) => item.id !== selected.id), selected] })}>Bring to front</Button>
              </div>}
              {tool === 'crop' && <>
                <p className="scene-property-note">Drag a rectangle on the page to crop.</p>
                <Button variant="outline" size="sm" disabled={!page.crop} onClick={() => commitPage({ ...page, crop: null })}>Remove crop</Button>
              </>}
              {tool === 'select' && !selected && <p className="scene-property-note">Select an object on the page to move it or adjust its properties.</p>}
              {tool === 'highlight' && <p className="scene-property-note">{document?.pages[page.sourceIndex ?? -1]?.textRuns?.length ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</p>}
              {tool === 'signature' && !selected && <p className="scene-property-note">{signatureMode === 'image' ? 'Choose an image, then drag its box onto the page.' : 'Type a signature, choose its font, then drag a box onto the page.'}</p>}
              {tool === 'watermark' && !selected && <>
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
        <Button variant="ghost" size="sm" aria-label="Rotate selected pages" onClick={() => commit({ pages: scene.pages.map((item) => targets.includes(item.id) ? { ...item, rotation: (item.rotation + 90) % 360 } : item) })}>Rotate ↻</Button>
        <Button variant="ghost" size="sm" aria-label="Move page earlier" disabled={currentIndex <= 0} onClick={() => shiftPage(-1)}>Move ←</Button>
        <Button variant="ghost" size="sm" aria-label="Move page later" disabled={currentIndex >= scene.pages.length - 1} onClick={() => shiftPage(1)}>Move →</Button>
        <Button variant="ghost" size="sm" onClick={pageNumbers}>Page numbers</Button><span className="scene-original-note">Original unchanged</span>
      </div>
    </>}
  </section>;
}

function PageActionsMenu({ sourcePath, onNavigate }: { sourcePath: string | null; onNavigate?: PdfEditorNavigation }) {
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
    if (!sourcePath || !onNavigate) return;
    close(false);
    onNavigate({ utilityId, initialPaths: [sourcePath] });
  };
  const toggleMenu = () => {
    const bounds = triggerRef.current?.getBoundingClientRect();
    if (bounds) {
      setMenuPosition({
        top: Math.max(8, Math.min(bounds.bottom + 5, window.innerHeight - 92)),
        left: Math.max(8, Math.min(bounds.right + 8, window.innerWidth - 162)),
      });
    }
    setOpen((value) => !value);
  };

  return <>
    <Button ref={triggerRef} variant="ghost" size="icon-sm" aria-label="Page actions" aria-haspopup="menu" aria-expanded={open} aria-controls="pdf-page-actions-menu"
      title="Merge or split this PDF" disabled={!sourcePath || !onNavigate} onClick={toggleMenu}>⋯</Button>
    {open && menuPosition && typeof document !== 'undefined' && createPortal(
      <div id="pdf-page-actions-menu" className="scene-page-actions-popover fixed z-50" role="menu" aria-label="Page actions" ref={menuRef} onKeyDown={handleKeyDown} style={menuPosition}>
        <Card className="w-40 gap-1 p-1">
          <Button role="menuitem" tabIndex={-1} variant="ghost" size="sm" className="w-full justify-start" onClick={() => navigate('pdf-merge')}>Merge PDF</Button>
          <Button role="menuitem" tabIndex={-1} variant="ghost" size="sm" className="w-full justify-start" onClick={() => navigate('pdf-split')}>Split PDF</Button>
        </Card>
      </div>, document.body,
    )}
  </>;
}
