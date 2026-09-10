import { useEffect, useRef, useState } from 'react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ToolScaffold } from '../components/ToolScaffold';
import type { ToolDefinition, ToolResult } from '../contracts';
import type { PdfDocument } from '../features/pdf-editor/contracts';
import { SceneCanvas } from '../features/pdf-editor/SceneCanvas';
import { sceneFromDocument, visibleBounds } from '../features/pdf-editor/scene';
import type { PdfScene, SceneObject, ScenePage, ScenePreview, SceneRect, SceneShape, SceneTool, SignatureMode, WatermarkPattern } from '../features/pdf-editor/scene';

const tools: { id: SceneTool; label: string; glyph: string }[] = [
  { id: 'select', label: 'Select', glyph: '↖' }, { id: 'text', label: 'Text', glyph: 'T' },
  { id: 'highlight', label: 'Highlight', glyph: '▰' }, { id: 'shape', label: 'Shape', glyph: '□' },
  { id: 'signature', label: 'Signature', glyph: '〰' }, { id: 'watermark', label: 'Watermark', glyph: 'W' },
  { id: 'crop', label: 'Crop', glyph: '⌗' },
];
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
type History = { past: PdfScene[]; present: PdfScene; future: PdfScene[] };

export const PDFEditorWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const session = useRef<{ path: string; scene: PdfScene } | null>(null);
  const initialTool: SceneTool = utility.id === 'pdf-crop' ? 'crop' : utility.id === 'pdf-watermark' ? 'watermark' : utility.id === 'pdf-sign' ? 'signature' : 'select';
  return <ToolScaffold utility={utility} variant="workspace" sessionKey="pdf-editor-scene"
    onRun={async (paths) => {
      if (!session.current || session.current.path !== paths[0]) throw new Error('Open a PDF before exporting.');
      return invoke<ToolResult>('export_pdf_scene', { request: { paths, scene: session.current.scene, outputLocation: 'alongsideInput' } });
    }}>
    {({ files, run, loading }) => <PDFSceneSession key={files[0] ?? 'empty'} path={files[0] ?? null} initialTool={initialTool}
      exporting={loading} onExport={run} onScene={(path, scene) => { session.current = path && scene ? { path, scene } : null; }} />}
  </ToolScaffold>;
};

function PDFSceneSession({ path, initialTool, exporting, onExport, onScene }: {
  path: string | null; initialTool: SceneTool; exporting: boolean; onExport: () => Promise<void>;
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
  const [color, setColor] = useState('#202020');
  const [opacity, setOpacity] = useState(1);
  const [highlightColor, setHighlightColor] = useState('#ffda36');
  const [highlightOpacity, setHighlightOpacity] = useState(0.35);
  const [shape, setShape] = useState<SceneShape>('square');
  const [signatureMode, setSignatureMode] = useState<SignatureMode>('text');
  const [signaturePath, setSignaturePath] = useState<string | null>(null);
  const [signaturePreview, setSignaturePreview] = useState<string | null>(null);
  const [signatureText, setSignatureText] = useState('Your signature');
  const [signatureFont, setSignatureFont] = useState('Helvetica-Oblique');
  const [watermarkText, setWatermarkText] = useState('DRAFT');
  const [watermarkPattern, setWatermarkPattern] = useState<WatermarkPattern>('across-page');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(Boolean(path));
  const [rendering, setRendering] = useState(false);
  const [preview, setPreview] = useState<{ key: string; value: ScenePreview } | null>(null);
  const [thumbnails, setThumbnails] = useState<Record<string, { key: string; value: ScenePreview }>>({});
  const [interaction, setInteraction] = useState(0);
  const [renderError, setRenderError] = useState<string | null>(null);
  const group = useRef<string | null>(null);
  const draggedPage = useRef<string | null>(null);
  const scene = history.present;
  const page = scene.pages.find((item) => item.id === currentId) ?? scene.pages[0];
  const selected = page?.objects.find((object) => object.id === selectedId);
  const currentIndex = scene.pages.findIndex((item) => item.id === page?.id);
  const previewKey = `${JSON.stringify(scene)}:${currentIndex}`;

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
    }).catch((reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [path]);
  useEffect(() => { onScene(path, document ? scene : null); }, [path, scene, document]);
  useEffect(() => {
    if (!path || !page) return;
    let active = true;
    setRenderError(null); setRendering(true);
    const timer = window.setTimeout(() => {
      invoke<ScenePreview>('preview_pdf_scene', { request: { path, scene, pageIndex: currentIndex } }).then((value) => {
        if (!active) return;
        setPreview({ key: previewKey, value });
        setThumbnails((existing) => ({ ...existing, [page.id]: { key: JSON.stringify(page), value } }));
      }).catch((reason) => { if (active) setRenderError(String(reason)); }).finally(() => { if (active) setRendering(false); });
    }, 200);
    return () => { active = false; window.clearTimeout(timer); };
  }, [path, previewKey, interaction]);

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
    fontSize: kind === 'signature' ? Math.max(18, fontSize) : fontSize,
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
      let previewSource = picked;
      try { previewSource = convertFileSrc(picked); } catch { /* The native export still has the original local path. */ }
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
    const blank: ScenePage = { id: crypto.randomUUID(), sourceIndex: null, width: page.width, height: page.height, rotation: 0, crop: null, objects: [] };
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
  const activeSignatureText = selected?.text ?? signatureText;
  const activeSignatureFont = selected?.fontFamily ?? signatureFont;
  const activeWatermarkPattern = selected?.watermarkPattern ?? watermarkPattern;
  const activeWatermarkText = selected?.text ?? watermarkText;

  return <section className="pdf-scene-workspace" aria-label="PDF editing session" onKeyDown={(event) => {
    const input = event.target instanceof HTMLElement && (['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName) || event.target.isContentEditable);
    if (!input && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); event.shiftKey ? redo() : undo(); }
  }}>
    <div className="scene-tools" role="toolbar" aria-label="PDF editor tools">
      {tools.map((item) => <button key={item.id} type="button" aria-label={item.label} title={item.label} aria-pressed={tool === item.id}
        onClick={() => { endGroup(); setTool(item.id); setSelectedId(null); }} disabled={!page}><span aria-hidden="true">{item.glyph}</span>{item.label}</button>)}
      <span className="scene-toolbar-divider" />
      <button type="button" aria-label="Undo" title="Undo (⌘Z / Ctrl+Z)" disabled={!history.past.length} onClick={undo}>↶</button>
      <button type="button" aria-label="Redo" title="Redo (⇧⌘Z / Ctrl+Shift+Z)" disabled={!history.future.length} onClick={redo}>↷</button>
      <button type="button" aria-label="Reset edits" disabled={!dirty} onClick={reset}>Reset</button>
      <button type="button" className="workspace-primary-action scene-export" aria-label="Export PDF" disabled={!page || exporting || Boolean(renderError) || rendering || !exactPreview}
        onClick={() => void onExport()}>{exporting ? 'Exporting…' : 'Export PDF'}</button>
    </div>
    <div className="scene-properties" aria-label="Object properties">
      {selectedKind === 'text' && <>
        <span className="scene-inline-hint">Double-click text on the page to edit</span>
        <label>Size<input aria-label="Font size" type="number" min="6" max="144" value={selected?.fontSize ?? fontSize}
          onChange={(event) => { const value = Math.max(6, Math.min(144, Number(event.target.value) || 6)); selected ? updateObject({ fontSize: value }, 'font') : setFontSize(value); }} onBlur={endGroup} /></label>
      </>}
      {selectedKind === 'shape' && <label>Shape<select aria-label="Shape type" value={activeShape} onChange={(event) => selected ? updateObject({ shape: event.target.value as SceneShape }, 'shape') : setShape(event.target.value as SceneShape)}>
        <option value="square">Square</option><option value="round">Round</option><option value="triangle">Triangle</option><option value="line">Line</option><option value="dotted-line">Dotted line</option>
        <option value="arrow-left">Arrow left</option><option value="arrow-right">Arrow right</option><option value="arrow-up">Arrow up</option><option value="arrow-down">Arrow down</option>
      </select></label>}
      {selectedKind === 'signature' && <>
        <label>Mode<select aria-label="Signature mode" value={activeSignatureMode} onChange={(event) => { const value = event.target.value as SignatureMode; selected ? updateObject({ signatureMode: value, strokes: [] }, 'signature-mode') : setSignatureMode(value); }}>
          <option value="image">Choose image</option><option value="text">Type signature</option>
        </select></label>
        {activeSignatureMode === 'image' ? <>
          <button type="button" onClick={() => void chooseSignatureImage()}>Choose signature image</button>
          <span className="scene-property-note">{activeSignaturePath ? activeSignaturePath.split(/[\\/]/).pop() : 'No image chosen'}</span>
        </> : <>
          <label>Signature<input aria-label="Signature text" value={activeSignatureText} onChange={(event) => selected ? updateObject({ text: event.target.value }, 'signature-text') : setSignatureText(event.target.value)} onBlur={endGroup} /></label>
          <label>Font<select aria-label="Signature font" value={activeSignatureFont} onChange={(event) => selected ? updateObject({ fontFamily: event.target.value }, 'signature-font') : setSignatureFont(event.target.value)}>
            <option value="Helvetica-Oblique">Helvetica italic</option><option value="Times-Italic">Times italic</option><option value="Courier-Oblique">Courier italic</option>
            <option value="Helvetica">Helvetica</option><option value="Times-Roman">Times</option><option value="Courier">Courier</option>
          </select></label>
        </>}
        <label>Size<input aria-label="Font size" type="number" min="8" max="144" value={selected?.fontSize ?? Math.max(18, fontSize)}
          onChange={(event) => { const value = Math.max(8, Math.min(144, Number(event.target.value) || 8)); selected ? updateObject({ fontSize: value }, 'signature-font-size') : setFontSize(value); }} onBlur={endGroup} /></label>
        {activeSignatureMode === 'image' && activeSignaturePreview && <img className="scene-signature-mini-preview" alt="Selected signature preview" src={activeSignaturePreview} />}
      </>}
      {(selectedKind === 'watermark') && <>
        <label>Text<input aria-label="Watermark text" value={activeWatermarkText} onChange={(event) => selected ? updateObject({ text: event.target.value }, 'watermark-text') : setWatermarkText(event.target.value)} onBlur={endGroup} /></label>
        <label>Pattern<select aria-label="Watermark pattern" value={activeWatermarkPattern} onChange={(event) => selected ? updateObject({ watermarkPattern: event.target.value as WatermarkPattern }, 'watermark-pattern') : setWatermarkPattern(event.target.value as WatermarkPattern)}>
          <option value="across-page">Across page</option><option value="bottom-right-to-top-left">Bottom-right to top-left</option><option value="top-right-to-bottom-left">Top-right to bottom-left</option><option value="center-horizontal">Centre horizontal</option><option value="center-vertical">Centre vertical</option>
        </select></label>
        {selected && <button type="button" onClick={addFixedWatermark}>Add fixed watermark</button>}
      </>}
      {(selected || !['select', 'crop'].includes(tool)) && <>
        <label>Color<input type="color" aria-label="Object color" value={activeColor} onChange={(event) => {
          if (selected) updateObject({ color: event.target.value }, 'color'); else if (tool === 'highlight') setHighlightColor(event.target.value); else setColor(event.target.value);
        }} onBlur={endGroup} /></label>
        <label>Opacity<input aria-label="Object opacity" type="range" min="5" max="100" value={Math.round(activeOpacity * 100)}
          onChange={(event) => { const value = Number(event.target.value) / 100; if (selected) updateObject({ opacity: value }, 'opacity'); else if (tool === 'highlight') setHighlightOpacity(value); else setOpacity(value); }} onBlur={endGroup} /></label>
      </>}
      {selected && <><button type="button" onClick={() => { commitPage({ ...page, objects: page.objects.filter((item) => item.id !== selected.id) }); setSelectedId(null); }}>Delete object</button>
        <button type="button" onClick={() => commitPage({ ...page, objects: [...page.objects.filter((item) => item.id !== selected.id), selected] })}>Bring to front</button></>}
      {tool === 'crop' && <><span>Drag a rectangle on the page to crop.</span><button type="button" disabled={!page?.crop} onClick={() => commitPage({ ...page, crop: null })}>Remove crop</button></>}
      {tool === 'select' && !selected && <span>Select an object to move or resize it. Shift-click thumbnails to select pages.</span>}
      {tool === 'highlight' && <span>{document?.pages[page?.sourceIndex ?? -1]?.textRuns?.length ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</span>}
      {tool === 'signature' && !selected && <span>{signatureMode === 'image' ? 'Choose an image, then drag its box onto the page.' : 'Type a signature, choose its font, then drag a box onto the page.'}</span>}
      {tool === 'watermark' && !selected && <><span>Fixed placement: choose a pattern and add it to the page.</span><button type="button" onClick={addFixedWatermark}>Add fixed watermark</button></>}
      <span className="scene-render-status" role="status">{rendering ? 'Rendering on device…' : exactPreview ? 'Export preview · on device' : 'Local document'}</span>
    </div>
    {error && <p role="alert" className="scene-error">{error}</p>}
    {renderError && <p role="alert" className="scene-error">{renderError} Export is disabled until the preview can be verified.</p>}
    {!page ? <div className="scene-empty"><strong>{loading ? 'Opening PDF…' : 'Open a PDF to edit'}</strong><span>Text, markup, signatures and page edits in one document.<br />Files stay on your device. Export creates a new copy.</span></div> : <>
      <div className="scene-document">
        <aside className="scene-thumbnails" aria-label="PDF page thumbnails">
          <div className="scene-page-actions"><button type="button" aria-label="Add blank page" title="Insert blank page after this page" onClick={insertPage}>＋</button>
            <button type="button" aria-label="Delete selected pages" title="Delete selected pages" disabled={targets.length >= scene.pages.length} onClick={deletePages}>−</button></div>
          {scene.pages.map((item, index) => {
            const cached = thumbnails[item.id];
            const thumb = cached?.key === JSON.stringify(item) ? cached.value.dataUrl : item.sourceIndex === null ? null : document?.pages[item.sourceIndex]?.preview;
            return <button key={item.id} type="button" className="scene-thumbnail" aria-label={`Page ${index + 1}${item.sourceIndex === null ? ', blank' : ''}`}
              aria-current={page.id === item.id ? 'page' : undefined} aria-pressed={targets.includes(item.id)} draggable
              onDragStart={() => { draggedPage.current = item.id; }} onDragEnd={() => { draggedPage.current = null; }} onDragOver={(event) => event.preventDefault()} onDrop={() => movePages(item.id)}
              onClick={(event) => selectPage(item.id, event.metaKey || event.ctrlKey, event.shiftKey)}>
              {thumb ? <img alt={`Thumbnail of page ${index + 1}`} src={thumb} /> : <span className="scene-blank-thumb" />}
              <span>{index + 1}</span><small>{item.objects.length ? `${item.objects.length} marks` : item.sourceIndex === null ? 'Blank' : ''}</small>
            </button>;
          })}
        </aside>
        <div className="scene-canvas-column">
          <SceneCanvas page={page} sourcePreview={page.sourceIndex === null ? null : document?.pages[page.sourceIndex]?.preview ?? null}
            textRuns={page.sourceIndex === null ? [] : document?.pages[page.sourceIndex]?.textRuns ?? []}
            renderedPreview={exactPreview} tool={tool} selectedId={selectedId} zoom={zoom} onSelect={(id) => { endGroup(); setSelectedId(id); }}
            onCommit={commitPage} onInteraction={() => { setPreview(null); setInteraction((value) => value + 1); }} makeObject={makeObject} />
        </div>
      </div>
      <footer className="scene-bottom-bar">
        <button type="button" aria-label="Previous page" disabled={currentIndex <= 0} onClick={() => selectPage(scene.pages[currentIndex - 1].id)}>‹</button>
        <span>Page {currentIndex + 1} of {scene.pages.length}</span>
        <button type="button" aria-label="Next page" disabled={currentIndex >= scene.pages.length - 1} onClick={() => selectPage(scene.pages[currentIndex + 1].id)}>›</button>
        <span className="scene-toolbar-divider" />
        <button type="button" aria-label="Zoom out" disabled={zoom <= 0.5} onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}>−</button>
        <button type="button" aria-label="Fit page" onClick={() => setZoom(1)}>{Math.round(zoom * 100)}% · Fit</button>
        <button type="button" aria-label="Zoom in" disabled={zoom >= 3} onClick={() => setZoom((value) => Math.min(3, value + 0.25))}>＋</button>
        <button type="button" aria-label="Rotate selected pages" onClick={() => commit({ pages: scene.pages.map((item) => targets.includes(item.id) ? { ...item, rotation: (item.rotation + 90) % 360 } : item) })}>Rotate ↻</button>
        <button type="button" aria-label="Move page earlier" disabled={currentIndex <= 0} onClick={() => shiftPage(-1)}>Move ←</button>
        <button type="button" aria-label="Move page later" disabled={currentIndex >= scene.pages.length - 1} onClick={() => shiftPage(1)}>Move →</button>
        <button type="button" onClick={pageNumbers}>Page numbers</button><span className="scene-original-note">Original unchanged</span>
      </footer>
    </>}
  </section>;
}
