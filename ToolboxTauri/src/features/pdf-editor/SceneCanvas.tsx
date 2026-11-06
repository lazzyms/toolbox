import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent as ReactMouseEvent, PointerEvent } from 'react';
import { Checkbox } from '../../components/ui/checkbox';
import { Input } from '../../components/ui/input';
import { Textarea } from '../../components/ui/textarea';
import { notEditableReasons, overflowMessage, textOverflowWords, visibleBounds } from './scene';
import type { SceneCanvasProps, SceneObject, ScenePage, ScenePoint, SceneRect, SceneShape, WatermarkPattern } from './scene';
import './scene-canvas.css';

export function sceneTransform(page: ScenePage) {
  const b = visibleBounds(page);
  const angle = ((page.rotation % 360) + 360) % 360;
  const radians = angle * Math.PI / 180;
  const c = Math.round(Math.cos(radians) * 1e12) / 1e12;
  const s = Math.round(Math.sin(radians) * 1e12) / 1e12;
  const corners = [[0, 0], [b.width, 0], [0, b.height], [b.width, b.height]];
  const xs = corners.map(([x, y]) => c * x - s * y);
  const ys = corners.map(([x, y]) => s * x + c * y);
  return { width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys),
    matrix: `matrix(${c} ${s} ${-s} ${c} ${-Math.min(...xs) - c * b.x + s * b.y} ${-Math.min(...ys) - s * b.x - c * b.y})` };
}

type Gesture = { pointerId: number; start: ScenePoint; base: ScenePage; next: ScenePage;
  mode: 'create' | 'move' | 'resize' | 'crop' | 'text-selection' | 'pan' | 'redact'; id?: string; corner?: string; points: ScenePoint[]; changed: boolean;
  clientStart?: ScenePoint; scrollStart?: ScenePoint };
const rectangle = (a: ScenePoint, b: ScenePoint): SceneRect => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.max(1, Math.abs(a.x - b.x)), height: Math.max(1, Math.abs(a.y - b.y)) });
// Drags a corner to p while the opposite corner stays put; signatures keep their proportions.
function cornerRect(r: SceneRect, corner: string, p: ScenePoint, keepRatio: boolean): SceneRect {
  const anchor = { x: corner.includes('w') ? r.x + r.width : r.x, y: corner.includes('n') ? r.y + r.height : r.y };
  if (!keepRatio) return rectangle(anchor, p);
  const sx = Math.abs(p.x - anchor.x) / r.width, sy = Math.abs(p.y - anchor.y) / r.height;
  const scale = Math.abs(sx - 1) >= Math.abs(sy - 1) ? sx : sy;
  const width = Math.max(1, r.width * scale), height = Math.max(1, r.height * scale);
  return { x: p.x < anchor.x ? anchor.x - width : anchor.x, y: p.y < anchor.y ? anchor.y - height : anchor.y, width, height };
}

const cssFont = (font?: string) => font?.startsWith('Noto') ? `"Toolbox ${font}", ${font.endsWith('Mono') ? 'monospace' : font.endsWith('Serif') ? 'serif' : 'sans-serif'}`
  : font === 'Pacifico' ? 'Toolbox Pacifico, cursive'
    : font?.startsWith('Times') ? 'Times New Roman, serif' : font?.startsWith('Courier') ? 'Courier New, monospace' : 'Helvetica, Arial, sans-serif';
const fontWeight = (font?: string) => font?.includes('Bold') ? 700 : 400;
const fontStyle = (font?: string) => font?.includes('Oblique') || font?.includes('Italic') ? 'italic' : 'normal';

function shapeArtwork(shape: SceneShape | undefined, r: SceneRect, color: string, opacity: number) {
  const selected = shape ?? 'square';
  const common = { fill: 'none', stroke: color, strokeWidth: 2, opacity };
  const midX = r.x + r.width / 2, midY = r.y + r.height / 2;
  if (selected === 'round') return <rect {...r} {...common} rx={Math.min(r.width, r.height) / 2} />;
  if (selected === 'triangle') return <polygon points={`${r.x},${r.y + r.height} ${midX},${r.y} ${r.x + r.width},${r.y + r.height}`} {...common} />;
  if (selected === 'line' || selected === 'dotted-line') return <line x1={r.x} y1={midY} x2={r.x + r.width} y2={midY} {...common} strokeDasharray={selected === 'dotted-line' ? '3 4' : undefined} />;
  if (selected === 'arrow-left' || selected === 'arrow-right') {
    const right = selected === 'arrow-right';
    const tip = right ? r.x + r.width : r.x;
    const base = right ? r.x + r.width - 10 : r.x + 10;
    return <g {...common} fill={color}>
      <line x1={right ? r.x : r.x + 10} y1={midY} x2={right ? r.x + r.width - 10 : r.x + r.width} y2={midY} />
      <polygon points={`${tip},${midY} ${base},${midY - 7} ${base},${midY + 7}`} />
    </g>;
  }
  if (selected === 'arrow-up' || selected === 'arrow-down') {
    const down = selected === 'arrow-down';
    const tip = down ? r.y + r.height : r.y;
    const base = down ? r.y + r.height - 10 : r.y + 10;
    return <g {...common} fill={color}>
      <line x1={midX} y1={down ? r.y : r.y + 10} x2={midX} y2={down ? r.y + r.height - 10 : r.y + r.height} />
      <polygon points={`${midX},${tip} ${midX - 7},${base} ${midX + 7},${base}`} />
    </g>;
  }
  return <rect {...r} {...common} />;
}

function watermarkPlacements(page: ScenePage, object: SceneObject): { x: number; y: number; angle: number }[] {
  const area = visibleBounds(page);
  const anchor = object.rect;
  const pattern: WatermarkPattern = object.watermarkPattern ?? 'across-page';
  const textWidth = Math.max(1, object.text.length) * object.fontSize * 0.6;
  const centerX = anchor.x + anchor.width / 2, centerY = anchor.y + anchor.height / 2;
  if (pattern === 'bottom-right-to-top-left') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: -45 }];
  if (pattern === 'top-right-to-bottom-left') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 45 }];
  if (pattern === 'center-horizontal') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 0 }];
  if (pattern === 'center-vertical') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 90 }];
  const stepX = Math.max(100, textWidth + object.fontSize * 2), stepY = Math.max(80, object.fontSize * 4);
  const placements: { x: number; y: number; angle: number }[] = [];
  const offsetX = anchor.x - (area.x + area.width * 0.14);
  const offsetY = anchor.y - (area.y + area.height * 0.33);
  for (let y = area.y - area.height + offsetY; y <= area.y + area.height * 2 + offsetY; y += stepY) {
    const offset = Math.round((y - (area.y + offsetY)) / stepY) * stepX * 0.45;
    for (let x = area.x - area.width + offsetX + offset; x <= area.x + area.width * 2 + offsetX; x += stepX) placements.push({ x, y, angle: -35 });
  }
  return placements;
}

function resizedObject(object: SceneObject, rect: SceneRect, original: SceneObject): SceneObject {
  if (object.kind === 'watermark') return original;
  return { ...object, rect };
}

// Mirrors the exporter: the name is drawn at 96 pt with 8 pt padding and fitted inside the box.
export function TypedSignature({ text, color, ...box }: { text: string; color: string; x?: number; y?: number; width: number | string; height: number | string; opacity?: number }) {
  const lines = text.split('\n');
  const context = typeof document === 'undefined' ? null : document.createElement('canvas').getContext('2d');
  if (context) context.font = '96px "Toolbox Pacifico", cursive';
  const measure = (line: string) => context?.measureText(line);
  const ascent = measure('Ag')?.fontBoundingBoxAscent || 125, descent = measure('Ag')?.fontBoundingBoxDescent || 36;
  const lineHeight = Math.max(ascent + descent, 105.6);
  const width = Math.max(...lines.map(line => measure(line)?.width ?? line.length * 48)) + 16;
  return <svg {...box} viewBox={`0 0 ${Math.max(1, width)} ${lineHeight * lines.length + 16}`} preserveAspectRatio="xMidYMid meet" overflow="hidden">
    <text fill={color} fontSize={96} fontFamily={cssFont('Pacifico')}>
      {lines.map((line, i) => <tspan key={i} x={8} y={8 + ascent + i * lineHeight}>{line}</tspan>)}
    </text>
  </svg>;
}

function Artwork({ page, object }: { page: ScenePage; object: SceneObject }) {
  const { rect: r } = object;
  if (object.kind === 'signature') {
    if (object.signatureMode === 'image') return object.signaturePreview
      ? <image href={object.signaturePreview} x={r.x} y={r.y} width={r.width} height={r.height} preserveAspectRatio="xMidYMid meet" opacity={object.opacity} />
      : <rect {...r} fill="var(--pdf-scene-placeholder)" stroke={object.color} strokeDasharray="5 4" opacity={object.opacity} />;
    return <TypedSignature text={object.text} color={object.color} opacity={object.opacity} x={r.x} y={r.y} width={r.width} height={r.height} />;
  }
  // While drafting, wrap like the native exporter; the rendered preview replaces this once it is ready.
  if (object.kind === 'text') return <foreignObject {...r}>
    <div className="scene-text-draft" style={{ color: object.color, opacity: object.opacity, fontSize: object.fontSize, fontFamily: cssFont(object.fontFamily ?? 'Noto Sans'),
      fontWeight: object.bold ? 700 : 400, fontStyle: object.italic ? 'italic' : 'normal', textAlign: object.align ?? 'left' }}>{object.text}</div>
  </foreignObject>;
  if (object.kind === 'watermark') return <g fill={object.color} opacity={object.opacity} fontSize={object.fontSize} fontFamily={cssFont(object.fontFamily)} fontWeight={fontWeight(object.fontFamily)} fontStyle={fontStyle(object.fontFamily)}>
    {watermarkPlacements(page, object).map(({ x, y, angle }, index) => <text key={index} transform={`rotate(${angle} ${x} ${y})`} x={x} y={y} dominantBaseline="alphabetic">{object.text}</text>)}
  </g>;
  if (object.kind === 'highlight') return <rect {...r} fill={object.color} stroke="none" opacity={object.opacity} />;
  return shapeArtwork(object.shape, r, object.color, object.opacity);
}

export function SceneCanvas({ page, sourcePreview, textRuns = [], renderedPreview, tool, selectedId, zoom, onSelect, onCommit, makeObject, formFields = [], formValue, onFormValue,
  redactionFill = '#000000', redactionPreview = false, editRuns, textEdits, selectedRun, onSelectRun, onEditRun }: SceneCanvasProps) {
  const host = useRef<HTMLDivElement>(null);
  const surface = useRef<SVGSVGElement>(null);
  const coordinates = useRef<SVGGElement>(null);
  const gesture = useRef<Gesture | null>(null);
  const onSelectRef = useRef(onSelect);
  onSelectRef.current = onSelect;
  const lastObjectId = useRef<string | null>(null);
  const spacePressed = useRef(false);
  const [draft, setDraft] = useState<ScenePage | null>(null);
  const [size, setSize] = useState({ width: 600, height: 600 });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState('');
  const [spaceHeld, setSpaceHeld] = useState(false);
  const [panning, setPanning] = useState(false);
  const [editingRun, setEditingRun] = useState<{ index: number; text: string } | null>(null);
  function cancelGesture() {
    const active = gesture.current;
    if (!active) return;
    gesture.current = null;
    setDraft(null);
    setPanning(false);
    if (active.mode === 'text-selection') window.getSelection()?.removeAllRanges();
    if (surface.current?.hasPointerCapture(active.pointerId)) surface.current.releasePointerCapture(active.pointerId);
  }
  useLayoutEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { gesture.current = null; setDraft(null); setEditingId(null); setEditingRun(null); setPanning(false); window.getSelection()?.removeAllRanges(); }, [page]);
  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape' && gesture.current) {
        event.preventDefault();
        cancelGesture();
        onSelectRef.current(null);
        return;
      }
      if (event.key !== ' ') return;
      spacePressed.current = true;
      setSpaceHeld(true);
    };
    const onKeyUp = (event: globalThis.KeyboardEvent) => {
      if (event.key !== ' ') return;
      spacePressed.current = false;
      setSpaceHeld(false);
    };
    const onBlur = () => {
      spacePressed.current = false;
      setSpaceHeld(false);
      cancelGesture();
    };
    window.addEventListener('keydown', onKeyDown, true);
    window.addEventListener('keyup', onKeyUp);
    window.addEventListener('pointerup', cancelGesture);
    window.addEventListener('pointercancel', cancelGesture);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('keydown', onKeyDown, true);
      window.removeEventListener('keyup', onKeyUp);
      window.removeEventListener('pointerup', cancelGesture);
      window.removeEventListener('pointercancel', cancelGesture);
      window.removeEventListener('blur', onBlur);
      const active = gesture.current;
      gesture.current = null;
      if (active && surface.current?.hasPointerCapture(active.pointerId)) surface.current.releasePointerCapture(active.pointerId);
      window.getSelection()?.removeAllRanges();
    };
  }, []);
  const shown = draft ?? page;
  const transform = sceneTransform(page);
  const scale = Math.max(0.01, Math.min((size.width - 24) / transform.width, (size.height - 24) / transform.height)) * Math.max(0.1, zoom);
  const b = visibleBounds(page);
  const pointFromClient = (clientX: number, clientY: number): ScenePoint => {
    const matrix = coordinates.current?.getScreenCTM();
    if (!matrix) return { x: b.x, y: b.y };
    const p = new DOMPoint(clientX, clientY).matrixTransform(matrix.inverse());
    return { x: Math.max(b.x, Math.min(b.x + b.width, p.x)), y: Math.max(b.y, Math.min(b.y + b.height, p.y)) };
  };
  const point = (event: PointerEvent): ScenePoint => pointFromClient(event.clientX, event.clientY);

  function finishTextEdit(save: boolean) {
    if (!editingId) return;
    const object = page.objects.find(item => item.id === editingId);
    const value = editingText;
    setEditingId(null);
    if (save && object && value !== object.text) onCommit({ ...page, objects: page.objects.map(item => item.id === editingId ? { ...item, text: value } : item) });
  }
  function beginTextEdit(event: Pick<ReactMouseEvent<SVGElement> | KeyboardEvent<SVGElement>, 'preventDefault' | 'stopPropagation'>, object: SceneObject) {
    if (object.kind !== 'text') return;
    event.preventDefault(); event.stopPropagation();
    onSelect(object.id); setEditingText(object.text); setEditingId(object.id);
  }
  function doubleClick(event: ReactMouseEvent<SVGSVGElement>) {
    // Pointer capture can retarget the second click to the SVG surface. Keep
    // the selected object as the fallback so double-clicking still enters the
    // inline editor after the first click selected its hit box.
    const hit = Array.from(document.elementsFromPoint(event.clientX, event.clientY)).find((element) => element.closest('[data-object]'))?.closest('[data-object]');
    const id = (event.target as Element).closest('[data-object]')?.getAttribute('data-object') ?? hit?.getAttribute('data-object') ?? lastObjectId.current ?? selectedId;
    const object = id ? page.objects.find(item => item.id === id) : undefined;
    if (object) beginTextEdit(event, object);
  }
  const replacementOf = (index: number) => textEdits.find((edit) => edit.run === index)?.replacement;
  function openRun(index: number) {
    const run = editRuns.find((item) => item.index === index);
    onSelectRun(index);
    setEditingRun(run && !run.reason ? { index, text: replacementOf(index) ?? run.text } : null);
  }
  function finishRunEdit(save: boolean) {
    const run = editingRun && editRuns.find((item) => item.index === editingRun.index);
    // Clicking another run blurs this editor after the click already opened the next one.
    setEditingRun((current) => current?.index === editingRun?.index ? null : current);
    if (save && run && editingRun && editingRun.text !== (replacementOf(run.index) ?? run.text)) onEditRun(run, editingRun.text);
  }
  function selectedTextRects(): SceneRect[] {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !selection.toString().trim() || !selection.rangeCount) return [];
    return Array.from(selection.getRangeAt(0).getClientRects()).map(rect => rectangle(pointFromClient(rect.left, rect.top), pointFromClient(rect.right, rect.bottom))).filter(rect => rect.width > 2 && rect.height > 2);
  }
  function down(event: PointerEvent<SVGSVGElement>) {
    if (event.button === 1 || (event.button === 0 && spacePressed.current)) {
      event.preventDefault();
      const scrollable = host.current;
      surface.current?.setPointerCapture(event.pointerId);
      gesture.current = { pointerId: event.pointerId, start: point(event), base: page, next: page, mode: 'pan', points: [], changed: false,
        clientStart: { x: event.clientX, y: event.clientY }, scrollStart: { x: scrollable?.scrollLeft ?? 0, y: scrollable?.scrollTop ?? 0 } };
      setPanning(true);
      return;
    }
    if (event.button !== 0 || gesture.current) return;
    const target = event.target as Element;
    if (target.closest('[data-text-editor]')) return;
    if (tool === 'form') { if (!target.closest('[data-form-field]')) onSelect(null); return; }
    if (tool === 'edit-text') {
      const index = target.closest('[data-edit-run]')?.getAttribute('data-edit-run');
      if (index == null) { onSelectRun(null); setEditingRun(null); } else { event.preventDefault(); openRun(Number(index)); }
      return;
    }
    const textTarget = tool === 'highlight' && target.closest('[data-text-run]');
    const start = point(event);
    if (textTarget) {
      // Leave the browser's selection gesture intact. The selection is turned
      // into one or more scene highlights when the pointer is released.
      gesture.current = { pointerId: event.pointerId, start, base: page, next: page, mode: 'text-selection', points: [start], changed: false };
      surface.current?.setPointerCapture(event.pointerId);
      return;
    }
    if (tool === 'redact') {
      const markId = target.closest('[data-redaction]')?.getAttribute('data-redaction');
      if (markId) { onSelect(markId); return; }
      const mark = { id: crypto.randomUUID(), rect: { x: start.x, y: start.y, width: 1, height: 1 }, color: redactionFill };
      const next = { ...page, redactions: [...(page.redactions ?? []), mark] };
      event.preventDefault();
      surface.current?.setPointerCapture(event.pointerId);
      onSelect(mark.id);
      gesture.current = { pointerId: event.pointerId, start, base: page, next, mode: 'redact', id: mark.id, points: [start], changed: false };
      setDraft(next);
      return;
    }
    const id = target.closest('[data-object]')?.getAttribute('data-object') ?? undefined;
    const corner = target.getAttribute('data-corner') ?? undefined;
    let next = page;
    let mode: Gesture['mode'];
    let objectId = id;
    const hitObject = id ? page.objects.find(object => object.id === id) : undefined;
    if (hitObject?.kind === 'watermark' && tool !== 'crop') {
      lastObjectId.current = id ?? null;
      onSelect(id ?? null);
      return;
    }
    if (id && tool !== 'crop' && (tool === 'select' || selectedId === id || corner)) { mode = corner ? 'resize' : 'move'; lastObjectId.current = id; onSelect(id); }
    else if (tool === 'select') { lastObjectId.current = null; onSelect(null); return; }
    else if (tool === 'crop') { mode = 'crop'; onSelect(null); }
    else if (tool === 'watermark') { lastObjectId.current = null; onSelect(null); return; }
    else {
      mode = 'create';
      const object = makeObject(tool, { x: start.x, y: start.y, width: 1, height: 1 });
      objectId = object.id;
      next = { ...page, objects: [...page.objects, object] };
      onSelect(object.id);
    }
    event.preventDefault();
    surface.current?.setPointerCapture(event.pointerId);
    (target.closest('[data-object]') as SVGElement | null)?.focus();
    gesture.current = { pointerId: event.pointerId, start, base: page, next, mode, id: objectId, corner, points: [start], changed: false };
    setDraft(next);
  }
  function move(event: PointerEvent<SVGSVGElement>) {
    const g = gesture.current;
    if (!g || g.pointerId !== event.pointerId || g.mode === 'text-selection') return;
    if (g.mode === 'pan') {
      const scrollable = host.current;
      if (!scrollable || !g.clientStart || !g.scrollStart) return;
      scrollable.scrollLeft = g.scrollStart.x - event.clientX + g.clientStart.x;
      scrollable.scrollTop = g.scrollStart.y - event.clientY + g.clientStart.y;
      g.changed ||= Math.abs(event.clientX - g.clientStart.x) + Math.abs(event.clientY - g.clientStart.y) > 0.5;
      return;
    }
    const p = point(event);
    const dx = p.x - g.start.x, dy = p.y - g.start.y;
    g.changed ||= Math.abs(dx) + Math.abs(dy) > 0.5;
    if (g.mode === 'crop') g.next = { ...g.base, crop: rectangle(g.start, p) };
    else if (g.mode === 'redact') g.next = { ...g.next, redactions: g.next.redactions?.map(mark => mark.id === g.id ? { ...mark, rect: rectangle(g.start, p) } : mark) };
    else g.next = { ...g.next, objects: g.next.objects.map(object => {
      if (object.id !== g.id) return object;
      const original = g.base.objects.find(o => o.id === g.id) ?? object;
      let rect = rectangle(g.start, p);
      if (g.mode === 'move') rect = { ...original.rect,
        x: Math.max(b.x, Math.min(b.x + b.width - original.rect.width, original.rect.x + dx)),
        y: Math.max(b.y, Math.min(b.y + b.height - original.rect.height, original.rect.y + dy)) };
      if (g.mode === 'resize') rect = cornerRect(original.rect, g.corner ?? 'se', p, object.kind === 'signature');
      return g.mode === 'resize' ? resizedObject(object, rect, original) : { ...object, rect };
    }) };
    setDraft(g.next);
  }
  function finish(event: PointerEvent<SVGSVGElement>, cancel = false) {
    const g = gesture.current;
    if (!g || g.pointerId !== event.pointerId) return;
    gesture.current = null;
    setPanning(false);
    if (surface.current?.hasPointerCapture(event.pointerId)) surface.current.releasePointerCapture(event.pointerId);
    if (g.mode === 'pan') return;
    if (g.mode === 'text-selection') {
      if (!cancel) {
        const rects = selectedTextRects();
        if (rects.length) onCommit({ ...g.base, objects: [...g.base.objects, ...rects.map(rect => ({ ...makeObject('highlight', rect), rect, highlightMode: 'text-selection' as const }))] });
      }
      window.getSelection()?.removeAllRanges();
      return;
    }
    setDraft(null);
    const created = g.mode === 'create' ? g.next.objects.find(o => o.id === g.id) : undefined;
    if (!cancel && g.changed) onCommit(g.next);
    else if (!cancel && created?.kind === 'signature') {
      // A click places a signature at a default size, centred on the click.
      const width = Math.min(160, b.width), height = Math.min(56, b.height);
      const rect = { x: Math.max(b.x, Math.min(b.x + b.width - width, g.start.x - width / 2)), y: Math.max(b.y, Math.min(b.y + b.height - height, g.start.y - height / 2)), width, height };
      onCommit({ ...g.next, objects: g.next.objects.map(o => o.id === created.id ? { ...o, rect } : o) });
    }
    else if (g.mode === 'create' || g.mode === 'redact') onSelect(null);
  }
  function markKey(event: KeyboardEvent<SVGElement>, id: string) {
    if (event.key !== 'Delete' && event.key !== 'Backspace') return;
    event.preventDefault(); event.stopPropagation(); onSelect(null);
    onCommit({ ...page, redactions: page.redactions?.filter(mark => mark.id !== id) });
  }
  function key(event: KeyboardEvent<SVGElement>, object: SceneObject, corner?: string) {
    if (event.key === 'Escape') { cancelGesture(); onSelect(null); return; }
    if (event.key === 'Enter' && object.kind === 'text' && !corner) { beginTextEdit(event, object); return; }
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onSelect(object.id); return; }
    const steps: Record<string, ScenePoint> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } };
    const step = steps[event.key];
    if (!step && event.key !== 'Delete' && event.key !== 'Backspace') return;
    if (object.kind === 'watermark' && step) { event.preventDefault(); event.stopPropagation(); return; }
    event.preventDefault(); event.stopPropagation(); onSelect(object.id);
    const angle = page.rotation * Math.PI / 180, amount = event.shiftKey ? 10 : 1;
    const dx = step ? Math.round((step.x * Math.cos(angle) + step.y * Math.sin(angle)) * amount) : 0;
    const dy = step ? Math.round((-step.x * Math.sin(angle) + step.y * Math.cos(angle)) * amount) : 0;
    if (!step) { onSelect(null); onCommit({ ...page, objects: page.objects.filter(o => o.id !== object.id) }); return; }
    const r = object.rect;
    const rect = corner ? cornerRect(r, corner,
      { x: Math.max(b.x, Math.min(b.x + b.width, (corner.includes('w') ? r.x : r.x + r.width) + dx)), y: Math.max(b.y, Math.min(b.y + b.height, (corner.includes('n') ? r.y : r.y + r.height) + dy)) }, object.kind === 'signature')
      : { ...r, x: Math.max(b.x, Math.min(b.x + b.width - r.width, r.x + dx)), y: Math.max(b.y, Math.min(b.y + b.height - r.height, r.y + dy)) };
    onCommit({ ...page, objects: page.objects.map(o => o.id === object.id ? (corner ? resizedObject(o, rect, o) : { ...o, rect }) : o) });
  }
  const exact = renderedPreview && !draft && !editingId && !editingRun;
  const overflowRun = tool === 'edit-text' && selectedRun !== null ? editRuns.find((run) => run.index === selectedRun) : undefined;
  const overflowEdit = overflowRun && textEdits.find((edit) => edit.run === overflowRun.index);
  const overflowWords = overflowEdit ? textOverflowWords(overflowEdit.original, overflowEdit.replacement) : 0;
  return <div className="scene-canvas" ref={host}>
    <svg ref={surface} className={`scene-canvas-page scene-tool-${tool}${panning ? ' scene-pan-active' : spaceHeld ? ' scene-space-pan' : ''}`} aria-label="PDF page canvas" role="group" width={transform.width * scale} height={transform.height * scale} viewBox={`0 0 ${transform.width} ${transform.height}`} onPointerDown={down} onPointerMove={move} onPointerUp={e => finish(e)} onPointerCancel={e => finish(e, true)} onLostPointerCapture={e => finish(e, true)} onDoubleClick={doubleClick}>
      <rect width={transform.width} height={transform.height} fill="var(--pdf-document-page)" />
      {exact && <image href={renderedPreview.dataUrl} width={transform.width} height={transform.height} preserveAspectRatio="none" pointerEvents="none" />}
      <g ref={coordinates} transform={transform.matrix}>
        {!exact && <g pointerEvents="none">
          {sourcePreview && <image href={sourcePreview} x={0} y={0} width={page.width} height={page.height} preserveAspectRatio="none" />}
          {shown.objects.map(object => <Artwork key={object.id} page={page} object={object} />)}
        </g>}
        {tool === 'highlight' && textRuns.length > 0 && <g className="scene-text-layer" aria-label="Selectable PDF text">
          {textRuns.map((run, index) => <text key={`${index}-${run.x}-${run.y}`} className="scene-text-run" data-text-run="true" x={run.x} y={run.y + run.height} fontSize={Math.max(6, run.height)} fontFamily="Helvetica, Arial, sans-serif" fill="currentColor" opacity="0.001">{run.text}</text>)}
        </g>}
        {shown.objects.map((object, index) => <g key={object.id}>
          <rect {...object.rect} data-object={object.id} tabIndex={0} role="button" aria-label={`${object.kind} object ${index + 1}`} aria-pressed={selectedId === object.id} className={`scene-object-hit scene-object-${object.kind}`} onFocus={() => onSelect(object.id)} onKeyDown={e => key(e, object)} onDoubleClick={e => beginTextEdit(e, object)} />
          {selectedId === object.id && object.kind === 'text' && editingId === object.id && <foreignObject {...object.rect} data-text-editor="true" className="scene-inline-text-editor">
            <Textarea autoFocus onFocus={event => event.currentTarget.select()} className="h-full w-full min-h-0 resize-none" aria-label="Text box text" style={{ fontFamily: cssFont(object.fontFamily ?? 'Noto Sans'), fontWeight: object.bold ? 700 : 400, fontStyle: object.italic ? 'italic' : 'normal', textAlign: object.align ?? 'left' }} value={editingText} onChange={event => setEditingText(event.target.value)} onBlur={() => finishTextEdit(true)} onPointerDown={event => event.stopPropagation()} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); finishTextEdit(false); } else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') { event.preventDefault(); finishTextEdit(true); } }} />
          </foreignObject>}
          {selectedId === object.id && editingId !== object.id && <g className="scene-selection">
            <rect {...object.rect} fill="none" stroke="var(--pdf-scene-selection)" strokeWidth={1.5 / scale} pointerEvents="none" />
            {object.kind !== 'watermark' && (['nw', 'ne', 'sw', 'se'] as const).map(corner => <rect key={corner} data-object={object.id} data-corner={corner} x={(corner.includes('w') ? object.rect.x : object.rect.x + object.rect.width) - 5 / scale} y={(corner.includes('n') ? object.rect.y : object.rect.y + object.rect.height) - 5 / scale} width={10 / scale} height={10 / scale} tabIndex={0} role="button" aria-label={`Resize ${object.kind} ${corner}`} className="scene-resize-handle" strokeWidth={1 / scale} onKeyDown={e => key(e, object, corner)} />)}
          </g>}
        </g>)}
        {shown.redactions?.map((mark, index) => <rect key={mark.id} {...mark.rect} data-redaction={mark.id} data-preview={redactionPreview}
          className="scene-redaction" style={redactionPreview ? { fill: mark.color } : undefined}
          tabIndex={tool === 'redact' ? 0 : -1} role="button" aria-label={`Redaction mark ${index + 1}`} aria-pressed={selectedId === mark.id}
          onFocus={() => { if (tool === 'redact') onSelect(mark.id); }} onKeyDown={e => markKey(e, mark.id)} />)}
        {tool === 'edit-text' && <g className="scene-edit-layer" aria-label="PDF text you can edit">
          {editRuns.map((run) => {
            const edited = replacementOf(run.index);
            const label = run.reason ? `${run.text}. ${notEditableReasons[run.reason].short}` : `Edit text: ${edited ?? run.text}`;
            return <rect key={run.index} data-edit-run={run.index} x={run.x} y={run.y} width={Math.max(1, run.width)} height={Math.max(1, run.height)}
              className={run.reason ? 'scene-edit-run-blocked' : edited !== undefined ? 'scene-edit-run scene-edit-run-changed' : 'scene-edit-run'}
              strokeWidth={1.5 / scale} tabIndex={0} role="button" aria-label={label} aria-pressed={selectedRun === run.index}
              onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); openRun(run.index); } }}>
              {run.reason && <title>{notEditableReasons[run.reason].short}</title>}
            </rect>;
          })}
          {editingRun && (() => {
            const run = editRuns.find((item) => item.index === editingRun.index);
            return run && <foreignObject x={run.x} y={run.y - 2} width={Math.max(run.width, 40)} height={run.height + 4} data-text-editor="true" className="scene-inline-text-editor">
              <Input autoFocus className="h-full w-full min-h-0 rounded-sm px-1 py-0" style={{ fontSize: Math.max(6, run.height * 0.8) }} aria-label="Replacement text"
                value={editingRun.text} onChange={(event) => setEditingRun({ index: run.index, text: event.target.value })} onBlur={() => finishRunEdit(true)}
                onPointerDown={(event) => event.stopPropagation()}
                onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); finishRunEdit(true); } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finishRunEdit(false); } }} />
            </foreignObject>;
          })()}
          {overflowRun && overflowWords > 0 && <text className="scene-edit-overflow" x={overflowRun.x} y={overflowRun.y + overflowRun.height + 10} fontSize={8}>{overflowMessage(overflowWords)}</text>}
        </g>}
        {tool === 'form' && formValue && onFormValue && formFields.flatMap(field => field.widgets.filter(widget => widget.page === page.sourceIndex).map((widget, index) => {
          const value = formValue(field);
          const label = field.label || field.name;
          const fontSize = Math.max(6, Math.min(12, widget.height * 0.6));
          return <foreignObject key={`${field.name}:${index}`} x={widget.x} y={widget.y} width={widget.width} height={widget.height} className="scene-form-field" data-form-field={field.name}>
            {field.type === 'checkbox'
              ? <Checkbox aria-label={label} checked={value === true} disabled={field.readOnly} aria-required={field.required} style={{ width: Math.min(widget.width, widget.height), height: Math.min(widget.width, widget.height) }}
                onCheckedChange={checked => onFormValue(field, checked === true)} />
              : field.multiline
                ? <Textarea aria-label={label} value={String(value)} disabled={field.readOnly} required={field.required} maxLength={field.maxLen ?? undefined} className="h-full w-full min-h-0 resize-none rounded-none px-1 py-0" style={{ fontSize }} onChange={event => onFormValue(field, event.target.value)} />
                : <Input aria-label={label} value={String(value)} disabled={field.readOnly} required={field.required} maxLength={field.maxLen ?? undefined} className="h-full w-full min-h-0 rounded-none px-1 py-0" style={{ fontSize }} onChange={event => onFormValue(field, event.target.value)} />}
          </foreignObject>;
        }))}
        {draft && gesture.current?.mode === 'crop' && draft.crop && <rect {...draft.crop} className="scene-crop-outline" strokeWidth={2 / scale} pointerEvents="none" />}
      </g>
    </svg>
  </div>;
}
