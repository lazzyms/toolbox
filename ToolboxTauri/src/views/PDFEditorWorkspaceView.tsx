import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { WorkspaceSourceAction } from '../components/ToolScaffold';
import type { ShellEvent } from '../hooks/useShellBridge';
import type { AtomicToolId, ToolDefinition, ToolResult } from '../contracts';
import { useToolAvailability } from '../ToolAvailabilityContext';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Label } from '../components/ui/label';
import { Slider } from '../components/ui/slider';
import type { PdfDocument, PdfFlattenChoice, PdfFormEntry, PdfFormField, PdfFormInspection, PdfFormValue, PdfPage, PdfTextRun } from '../features/pdf-editor/contracts';
import { formEntryFlag, isFormEntryEmpty } from '../features/pdf-editor/form-checks';
import { colorTokens, readColorToken } from '../features/pdf-editor/colorTokens';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { RedactDisclosure, RedactInspector, listMarks, redactionFills, searchStatus } from '../features/pdf-editor/RedactInspector';
import type { ListedMark, TextMatch, TextSearch } from '../features/pdf-editor/RedactInspector';
import { subToolInspectors } from '../features/pdf-editor/SubToolInspectors';
import type { SubToolInspectorProps } from '../features/pdf-editor/SubToolInspectors';
import { planSceneExport, sceneContributors } from '../features/pdf-editor/subtools';
import { sceneFromDocument, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, TextStyle, WatermarkPattern } from '../features/pdf-editor/scene';
import { PREVIEW_CACHE_MAX_ENTRIES, PREVIEW_RENDER_SETTINGS, createPreviewCache, pdfEditorErrorMessage, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';
import type { GenerationState } from './pdfPreviewHelpers';
import { ShortcutHint } from '../components/ShortcutHint';
import { shortcutTarget } from '../shortcuts';

export { createPreviewCache, previewCacheKey, requestWithGeneration } from './pdfPreviewHelpers';

type IndexedScenePreview = { pageIndex: number; preview: ScenePreview };
type NativeTextMatch = { pageIndex: number; rects: SceneRect[]; before: string; text: string; after: string };

const clipped = (rect: SceneRect, bounds: SceneRect): SceneRect | null => {
  const x = Math.max(rect.x, bounds.x), y = Math.max(rect.y, bounds.y);
  const width = Math.min(rect.x + rect.width, bounds.x + bounds.width) - x, height = Math.min(rect.y + rect.height, bounds.y + bounds.height) - y;
  return width > 0 && height > 0 ? { x, y, width, height } : null;
};

const tools: { id: SceneTool; label: string; glyph: string }[] = [
  { id: 'select', label: 'Select', glyph: '↖' }, { id: 'text', label: 'Text', glyph: 'T' },
  { id: 'highlight', label: 'Highlight', glyph: '▰' }, { id: 'shape', label: 'Shape', glyph: '□' },
  { id: 'signature', label: 'Signature', glyph: '〰' }, { id: 'form', label: 'Fill form', glyph: '☑' }, { id: 'watermark', label: 'Watermark', glyph: 'W' },
  { id: 'crop', label: 'Crop', glyph: '⌗' }, { id: 'redact', label: 'Redact', glyph: '▮' },
];
const toolFlagBySceneTool: Record<SceneTool, AtomicToolId> = {
  select: 'pdf-edit', text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', form: 'pdf-edit', watermark: 'pdf-watermark', crop: 'pdf-crop', redact: 'pdf-redact',
};
const toolFlagByObjectKind: Record<SceneObject['kind'], AtomicToolId> = {
  text: 'pdf-edit', highlight: 'pdf-edit', shape: 'pdf-edit',
  signature: 'pdf-sign', watermark: 'pdf-watermark',
};
const toolGuidance: Record<SceneTool, string> = {
  select: 'Select, move, or resize an object.', text: 'Drag to add text. Double-click text to edit it.',
  highlight: 'Drag across text or draw a highlight area.', shape: 'Drag to draw a shape.',
  signature: 'Choose a signature, then drag its box onto the page.', form: 'Type into the form fields on the page.', watermark: 'Choose a pattern, then add a watermark.',
  crop: 'Drag a rectangle on the page to crop it.', redact: 'Drag over anything you want removed.',
};
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
  const session = useRef<{ path: string; scene: PdfScene; baseline: PdfScene; formValues: PdfFormValue[]; flatten: PdfFlattenChoice | null } | null>(null);
  const requestedTool: SceneTool = utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  const initialTool = availability.allows(toolFlagBySceneTool[requestedTool])
    ? requestedTool
    : tools.find((item) => availability.allows(toolFlagBySceneTool[item.id]))?.id ?? null;
  return <ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene" initialPaths={initialPaths} onWorkspaceSourceAction={onWorkspaceSourceAction} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange}
    onRun={async (paths, outputLocation, { runId }) => {
      const [inputPath] = paths;
      if (!inputPath || !session.current || session.current.path !== inputPath) throw new Error('Select exactly one open PDF before exporting.');
      const { request } = planSceneExport({ paths, scene: session.current.scene, baseline: session.current.baseline, outputLocation, formValues: session.current.formValues, flatten: session.current.flatten });
      const results = await invoke<ToolResult>('export_pdf_scene', { runId, request });
      return results.map((result) => result.failure ? { ...result, failure: { ...result.failure, message: pdfEditorErrorMessage(result.failure.message, 'export') } } : result);
    }}>
    {({ files, run, loading, destination }) => {
      const inputPath = files.length === 1 ? files[0] ?? null : null;
      return <>
        <PDFSceneSession key={inputPath ?? 'empty'} path={inputPath} initialTool={initialTool}
          exporting={loading} onExport={run}
          onNavigate={onNavigate}
          onScene={(path, scene, baseline, formValues, flatten) => { session.current = path && scene && baseline ? { path, scene, baseline, formValues, flatten } : null; }} />
        <Card className="pdf-scene-destination py-0"><CardContent className="py-3">{destination}</CardContent></Card>
      </>;
    }}
  </ToolScaffold>;
};

function PDFSceneSession({ path, initialTool, exporting, onExport, onNavigate, onScene }: {
  path: string | null; initialTool: SceneTool | null; exporting: boolean; onExport: () => Promise<void>;
  onNavigate?: PdfEditorNavigation;
  onScene: (path: string | null, scene: PdfScene | null, baseline: PdfScene | null, formValues: PdfFormValue[], flatten: PdfFlattenChoice | null) => void;
}) {
  const availability = useToolAvailability();
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [history, setHistory] = useState<History>({ past: [], present: { pages: [] }, future: [] });
  const [tool, setTool] = useState<SceneTool | null>(initialTool);
  useEffect(() => setTool(initialTool), [initialTool]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [selectedPages, setSelectedPages] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
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
  const [signatureFont, setSignatureFont] = useState('Satisfy');
  const [watermarkText, setWatermarkText] = useState('DRAFT');
  const [watermarkFontSize, setWatermarkFontSize] = useState(40);
  const [watermarkPattern, setWatermarkPattern] = useState<WatermarkPattern>('across-page');
  const [redactionFill, setRedactionFill] = useState<string>(redactionFills.Black);
  const [reviewing, setReviewing] = useState(false);
  const [previewRedactions, setPreviewRedactions] = useState(true);
  const [disclosure, setDisclosure] = useState(false);
  const [search, setSearch] = useState<TextSearch | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Form entries and the flatten choice live outside the scene history: Reset clears them, Undo does not.
  const [form, setForm] = useState<PdfFormInspection | null>(null);
  const [formEntries, setFormEntries] = useState<Record<string, PdfFormEntry>>({});
  const [formLeft, setFormLeft] = useState<ReadonlySet<string>>(new Set());
  const [formFocus, setFormFocus] = useState<string | null>(null);
  const [formNotice, setFormNotice] = useState<string | null>(null);
  const [flatten, setFlatten] = useState<PdfFlattenChoice | null>(null);
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
  const selected = page?.objects.find((object) => object.id === selectedId);
  const currentIndex = scene.pages.findIndex((item) => item.id === page?.id);
  const serializedScene = JSON.stringify(scene);
  const previewKey = path ? previewCacheKey(path, currentIndex, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
  const exactPreview = preview?.key === previewKey ? preview.value : null;
  const selectedSignature = selected?.kind === 'signature' ? selected : null;
  const marks = listMarks(scene);
  const requestExport = () => { if (marks.length) setDisclosure(true); else void onExport(); };

  useEffect(() => {
    let active = true;
    if (!path) { onScene(null, null, null, [], null); return; }
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
  useEffect(() => {
    if (tool !== 'form' || !path || formRequested.current) return;
    formRequested.current = true;
    invoke<PdfFormInspection | null>('inspect_pdf_form', { request: { path } })
      .then((value) => setForm(value ?? { fields: [], unsupported: null, comments: 0 }))
      .catch(() => setForm({ fields: [], unsupported: "This form can't be filled here. Toolbox couldn't read its fields.", comments: 0 }));
  }, [tool, path]);
  const formValue = (field: PdfFormField) => formEntries[field.name] ?? field.value;
  // Flags show once the user has left a field: an entry the export would refuse, or a required field still empty.
  const formFlag = (field: PdfFormField) => !formLeft.has(field.name) ? null
    : formEntryFlag(field, formValue(field)) ?? (field.required && isFormEntryEmpty(formValue(field)) ? 'Required' : null);
  const formLabel = (field: PdfFormField) => field.label || field.name;
  const leaveFormField = (field: PdfFormField) => {
    setFormLeft((left) => new Set(left).add(field.name));
    const flag = formEntryFlag(field, formValue(field));
    setFormNotice(flag ? `${formLabel(field)}: ${flag.toLowerCase()}.` : field.required && isFormEntryEmpty(formValue(field)) ? `${formLabel(field)} is required.` : null);
  };
  const formValues = useMemo<PdfFormValue[]>(() => (form?.fields ?? [])
    .filter((field) => field.name in formEntries && JSON.stringify(formEntries[field.name]) !== JSON.stringify(field.value))
    .map((field) => ({ name: field.name, value: formEntries[field.name] })), [form, formEntries]);
  useEffect(() => { onScene(path, document ? scene : null, document ? sceneFromDocument(document) : null, formValues, flatten); }, [path, scene, document, formValues, flatten]);
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
  // An unchanged scene keeps its preview, so Export works again after resetting only form entries or Flatten.
  const reset = () => { if (document && !same(scene, sceneFromDocument(document))) commit(sceneFromDocument(document)); setFormEntries({}); setFlatten(null); setFormLeft(new Set()); setFormNotice(null); setSelectedId(null); };
  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === 'undo') undo();
      else if (command === 'redo') redo();
      else if (command === 'export' && page && !exporting && !renderError && !rendering && exactPreview) requestExport();
    };
    window.addEventListener('toolbox:editor-command', onShellCommand);
    return () => window.removeEventListener('toolbox:editor-command', onShellCommand);
  }, [exactPreview, exporting, onExport, page, redo, renderError, rendering, undo, marks.length]);
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
  // Matches belong to the page layout they were found in, so reordering, removing or cropping pages clears them.
  const searchLayout = JSON.stringify(scene.pages.map((item) => [item.id, item.sourceIndex, item.crop]));
  useEffect(() => setSearch(null), [searchLayout]);
  const findText = async (query: string, matchCase: boolean) => {
    if (!path) return;
    const found = await invoke<NativeTextMatch[]>('find_pdf_scene_text', { request: { path, query, matchCase } });
    const matches: TextMatch[] = scene.pages.flatMap((item, index) => found.filter((match) => match.pageIndex === item.sourceIndex).flatMap((match) => {
      const rects = match.rects.map((rect) => clipped(rect, visibleBounds(item))).filter((rect): rect is SceneRect => rect !== null);
      return rects.length ? [{ pageId: item.id, page: index + 1, rects, before: match.before, text: match.text, after: match.after }] : [];
    }));
    setSearch({ query, matchCase, matches, selected: matches.map(() => true), marked: null, lastMarked: 0 });
  };
  const markMatches = (matches: TextMatch[]) => {
    if (!availability.allows('pdf-redact') || !matches.length) return [];
    const added = matches.map((match) => ({ match, marks: match.rects.map((rect) => ({ id: crypto.randomUUID(), rect, color: redactionFill, label: `“${match.text}”` })) }));
    commit({ pages: scene.pages.map((item) => {
      const marks = added.filter(({ match }) => match.pageId === item.id).flatMap(({ marks }) => marks);
      return marks.length ? { ...item, redactions: [...(item.redactions ?? []), ...marks] } : item;
    }) });
    return added.map(({ marks }) => marks.map((mark) => mark.id));
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
    ...(kind === 'text' ? textStyle : {}),
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
  const insertPage = () => {
    if (!page || !availability.allows('pdf-organize')) return;
    const blank: ScenePage = { id: crypto.randomUUID(), sourceIndex: null, width: page.width, height: page.height, rotation: 0, crop: null, sourceRotation: null, sourceBox: null, objects: [] };
    const next = [...scene.pages]; next.splice(currentIndex + 1, 0, blank);
    commit({ pages: next }); selectPage(blank.id);
  };
  const deletePages = () => {
    if (!availability.allows('pdf-remove-pages') || targets.length >= scene.pages.length) return;
    const next = scene.pages.filter((item) => !targets.includes(item.id));
    commit({ pages: next }); setCurrentId(next[Math.min(currentIndex, next.length - 1)].id); setSelectedPages([]); setSelectedId(null);
  };
  const movePages = (targetId: string) => {
    if (!availability.allows('pdf-organize')) return;
    const dragged = draggedPage.current; draggedPage.current = null;
    if (!dragged || dragged === targetId) return;
    const ids = targets.includes(dragged) ? targets : [dragged];
    if (ids.includes(targetId)) return;
    const moving = scene.pages.filter((item) => ids.includes(item.id));
    const remaining = scene.pages.filter((item) => !ids.includes(item.id));
    remaining.splice(remaining.findIndex((item) => item.id === targetId), 0, ...moving); commit({ pages: remaining });
  };
  const shiftPage = (delta: number) => {
    if (!availability.allows('pdf-organize')) return;
    const to = currentIndex + delta;
    if (to < 0 || to >= scene.pages.length) return;
    const next = [...scene.pages]; [next[currentIndex], next[to]] = [next[to], next[currentIndex]]; commit({ pages: next });
  };
  const pageNumbers = () => {
    if (!availability.allows('pdf-page-numbers')) return;
    commit({ pages: scene.pages.map((item, index) => {
    const b = visibleBounds(item);
    return { ...item, objects: [...item.objects, { ...makeObject('text', { x: b.x + b.width / 2 - 20, y: b.y + Math.max(0, b.height - 30), width: Math.min(40, b.width), height: Math.min(20, b.height) }), text: String(index + 1), fontSize: 12 }] };
    }) });
  };
  const dirty = document && sceneContributors(scene, sceneFromDocument(document), formValues, flatten).length > 0;
  const endGroup = () => { group.current = null; };
  const selectedKind = selected?.kind ?? tool;
  const selectedEnabled = Boolean(selected && availability.allows(toolFlagByObjectKind[selected.kind]));
  const canEditCanvas = tool !== null && availability.allows(toolFlagBySceneTool[tool])
    && (!selected || availability.allows(toolFlagByObjectKind[selected.kind]));
  const activeColor = selected?.color ?? (tool === 'highlight' ? highlightColor : color);
  const activeOpacity = selected?.opacity ?? (tool === 'highlight' ? highlightOpacity : opacity);

  const inspector: SubToolInspectorProps | null = page ? {
    page, pageNumber: currentIndex + 1, selected, hasTextRuns: Boolean(document?.pages[page.sourceIndex ?? -1]?.textRuns?.length), updateObject, commitPage, endGroup,
    fontSize, setFontSize, textStyle, setTextStyle, shape, setShape,
    signatureMode, setSignatureMode, signaturePath, signaturePreview, signatureText, setSignatureText,
    signatureTextDraft, setSignatureTextDraft, signatureFont, setSignatureFont, chooseSignatureImage,
    watermarkText, setWatermarkText, watermarkFontSize, setWatermarkFontSize, watermarkPattern, setWatermarkPattern, addFixedWatermark,
    form, formValue, formFlag, goToField, goToNextRequired, flatten, setFlatten, switchToAddText: () => { endGroup(); setTool('text'); setSelectedId(null); },
  } : null;
  const Fields = selectedKind ? subToolInspectors[selectedKind]?.Fields : undefined;
  const Guide = tool ? subToolInspectors[tool]?.Guide : undefined;

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
    if (!input && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); event.shiftKey ? redo() : undo(); }
  }}>
    <div ref={toolsToolbar} className="scene-tools" role="toolbar" aria-label="PDF editor tools" aria-orientation="horizontal" onFocusCapture={handleToolbarFocus} onKeyDown={handleToolbarKeyDown}>
      {tools.filter((item) => availability.allows(toolFlagBySceneTool[item.id])).map((item) => <Button key={item.id} variant={tool === item.id ? "secondary" : "ghost"} size="sm" aria-label={item.label} aria-description={toolGuidance[item.id]} title={toolGuidance[item.id]} aria-pressed={tool === item.id}
        onClick={() => { if (!availability.allows(toolFlagBySceneTool[item.id])) return; endGroup(); setTool(item.id); setSelectedId(null); }} disabled={!page}><span aria-hidden="true">{item.glyph}</span>{item.label}</Button>)}
      <span className="scene-toolbar-divider" />
      <Button variant="ghost" size="icon-sm" aria-label="Undo" title="Undo (⌘Z / Ctrl+Z)" disabled={!history.past.length} onClick={undo}>↶</Button>
      <Button variant="ghost" size="icon-sm" aria-label="Redo" title="Redo (⇧⌘Z / Ctrl+Shift+Z)" disabled={!history.future.length} onClick={redo}>↷</Button>
      <Button variant="destructive" size="sm" aria-label="Reset edits" disabled={!dirty} onClick={reset}>Reset</Button>
      <Button variant="default" size="sm" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!page || !dirty || exporting || Boolean(renderError) || rendering || !exactPreview}
        {...shortcutTarget('run')} onClick={requestExport}>{exporting ? 'Exporting…' : 'Export PDF'}<ShortcutHint id="run" /></Button>
    </div>
    {error && <p role="alert" className="scene-error">{error}</p>}
    {renderError && <p role="alert" className="scene-error">{renderError} Export is disabled until the preview can be verified.</p>}
    {!page ? <Card className="scene-empty"><CardHeader className="my-auto items-center text-center"><CardTitle role="heading" aria-level={2}>{loading ? 'Opening PDF…' : 'Open a PDF to edit'}</CardTitle><CardDescription>Text, markup, signatures and page edits in one document.<br />Files stay on your device. Export creates a new copy.</CardDescription></CardHeader></Card> : <>
      <div className="scene-document">
        <Card className="scene-thumbnails py-0" role="complementary" aria-label="PDF page thumbnails">
          <CardContent className="scene-thumbnails-content p-2">
          <div className="scene-page-actions">
            {availability.allows('pdf-organize') && <Button variant="ghost" size="icon-sm" aria-label="Add blank page" title="Insert blank page after this page" onClick={insertPage}>＋</Button>}
            {availability.allows('pdf-remove-pages') && <Button variant="destructive" size="icon-sm" aria-label="Delete selected pages" title="Delete selected pages" disabled={targets.length >= scene.pages.length} onClick={deletePages}>−</Button>}
            <PageActionsMenu sourcePath={path} onNavigate={onNavigate} />
          </div>
          {scene.pages.map((item, index) => {
            const cached = thumbnails[item.id];
            const thumbnailKey = path ? previewCacheKey(path, index, serializedScene, PREVIEW_RENDER_SETTINGS) : null;
            const thumb = cached?.key === thumbnailKey ? cached.value.dataUrl : item.sourceIndex === null ? null : document?.pages[item.sourceIndex]?.preview;
            return <Button key={item.id} variant={targets.includes(item.id) ? "secondary" : "ghost"} size="sm" className="scene-thumbnail h-auto min-h-0" aria-label={'Page ' + (index + 1) + (item.sourceIndex === null ? ', blank' : '')}
              aria-current={page.id === item.id ? 'page' : undefined} aria-pressed={targets.includes(item.id)} draggable={availability.allows('pdf-organize')}
              onDragStart={() => { if (availability.allows('pdf-organize')) draggedPage.current = item.id; }} onDragEnd={() => { draggedPage.current = null; }} onDragOver={(event) => { if (availability.allows('pdf-organize')) event.preventDefault(); }} onDrop={() => movePages(item.id)}
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
            renderedPreview={exactPreview} tool={tool ?? 'select'} selectedId={selectedId} zoom={zoom} onSelect={(id) => { endGroup(); setSelectedId(id); }}
            onCommit={(next) => { if (canEditCanvas) commitPage(next); }} makeObject={makeObject}
            formFields={form?.unsupported ? [] : form?.fields} formValue={formValue}
            onFormValue={(field, value) => { if (availability.allows('pdf-edit') && !field.readOnly) setFormEntries((entries) => ({ ...entries, [field.name]: value })); }}
            formFlag={formFlag} onFormFocus={(field) => setFormFocus(field.name)} onFormLeave={leaveFormField}
            redactionFill={redactionFill} redactionPreview={tool === 'redact' && reviewing && previewRedactions}
            searchMatches={search && !search.marked && !reviewing ? search.matches.filter((match, index) => match.pageId === page.id && search.selected[index]).flatMap((match) => match.rects) : []} />
        </div>
        <aside className="scene-inspector" aria-label="PDF editor inspector">
          <Card role="region" aria-label="Properties" className="scene-inspector-panel gap-4 overflow-y-auto py-4">
            <CardHeader className="gap-1 px-4 py-0">
              <CardTitle>{selected ? `${selected.kind[0].toUpperCase()}${selected.kind.slice(1)} properties` : tool === 'form' ? 'Fill form' : tool === 'redact' ? 'Redact' : 'Properties'}</CardTitle>
              <CardDescription>{selected ? 'Adjust the selected item.' : tool === 'form' ? 'Fill in the fields this PDF already has.' : tool === 'redact' ? 'Drag over anything you want removed. Marks are only previews until you export.' : tool === 'select' ? 'Select a mark to edit its properties.' : tool ? `New ${tool} settings` : 'No page markup tools are enabled.'}</CardDescription>
            </CardHeader>
            <CardContent className="scene-inspector-content flex flex-col gap-3 px-4">
              {tool === 'redact' && !selected && <RedactInspector marks={marks} selectedId={selectedId} reviewing={reviewing} preview={previewRedactions} fill={redactionFill} search={search}
                onSelect={selectMark} onRemove={removeMarks} onReview={review} onPreview={setPreviewRedactions} onFill={recolorMarks}
                onFind={findText} onSearch={setSearch} onMark={markMatches} />}
              {Fields && inspector && <Fields {...inspector} />}
              {(selected || (tool !== null && !['select', 'crop', 'form', 'redact'].includes(tool))) && <>
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
              <p className="scene-render-status" role="status">{rendering ? 'Rendering on device…' : tool === 'redact' && !reviewing && searchStatus(search) ? searchStatus(search) : exactPreview ? 'Export preview · on device' : 'Local document'}</p>
            </CardContent>
          </Card>
        </aside>
      </div>
      <RedactDisclosure marks={marks} open={disclosure} onOpenChange={setDisclosure}
        onReview={() => { setDisclosure(false); setTool('redact'); review(true); }}
        onContinue={() => { setDisclosure(false); void onExport(); }} />
      <div ref={bottomToolbar} className="scene-bottom-bar" role="toolbar" aria-label="PDF page and zoom controls" aria-orientation="horizontal" onFocusCapture={handleToolbarFocus} onKeyDown={handleToolbarKeyDown}>
        <Button variant="ghost" size="icon-sm" aria-label="Previous page" disabled={currentIndex <= 0} onClick={() => selectPage(scene.pages[currentIndex - 1].id)}>‹</Button>
        <span>Page {currentIndex + 1} of {scene.pages.length}</span>
        <Button variant="ghost" size="icon-sm" aria-label="Next page" disabled={currentIndex >= scene.pages.length - 1} onClick={() => selectPage(scene.pages[currentIndex + 1].id)}>›</Button>
        <span className="scene-toolbar-divider" />
        <Button variant="ghost" size="icon-sm" aria-label="Zoom out" disabled={zoom <= 0.5} onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}>−</Button>
        <Button variant="ghost" size="sm" aria-label="Fit page" onClick={() => setZoom(1)}>{Math.round(zoom * 100)}% · Fit</Button>
        <Button variant="ghost" size="icon-sm" aria-label="Zoom in" disabled={zoom >= 3} onClick={() => setZoom((value) => Math.min(3, value + 0.25))}>＋</Button>
        {availability.allows('pdf-organize') && <><Button variant="ghost" size="sm" aria-label="Rotate selected pages" onClick={() => { if (availability.allows('pdf-organize')) commit({ pages: scene.pages.map((item) => targets.includes(item.id) ? { ...item, rotation: (item.rotation + 90) % 360 } : item) }); }}>Rotate ↻</Button>
        <Button variant="ghost" size="sm" aria-label="Move page earlier" disabled={currentIndex <= 0} onClick={() => shiftPage(-1)}>Move ←</Button>
        <Button variant="ghost" size="sm" aria-label="Move page later" disabled={currentIndex >= scene.pages.length - 1} onClick={() => shiftPage(1)}>Move →</Button></>}
        {availability.allows('pdf-page-numbers') && <Button variant="ghost" size="sm" onClick={pageNumbers}>Page numbers</Button>}<span className="scene-original-note">{tool === 'form' && formUnavailable && <><span role="status">Fill form is unavailable for this PDF.</span><span className="scene-toolbar-divider" /></>}{tool === 'form' && formNotice && <><span role="status">{formNotice}</span><span className="scene-toolbar-divider" /></>}{tool === 'form' ? 'Fields are filled in the copy only' : 'Original unchanged'}</span>
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
      setMenuPosition({
        top: Math.max(8, Math.min(bounds.bottom + 5, window.innerHeight - 92)),
        left: Math.max(8, Math.min(bounds.right + 8, window.innerWidth - 162)),
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
