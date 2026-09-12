import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent as ReactMouseEvent, PointerEvent } from 'react';
import { visibleBounds } from './scene';
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
  mode: 'create' | 'move' | 'resize' | 'crop' | 'text-selection' | 'pan'; id?: string; corner?: string; points: ScenePoint[]; changed: boolean;
  clientStart?: ScenePoint; scrollStart?: ScenePoint };
const rectangle = (a: ScenePoint, b: ScenePoint): SceneRect => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.max(1, Math.abs(a.x - b.x)), height: Math.max(1, Math.abs(a.y - b.y)) });

const cssFont = (font?: string) => font?.startsWith('Times') ? 'Times New Roman, serif' : font?.startsWith('Courier') ? 'Courier New, monospace' : 'Helvetica, Arial, sans-serif';
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
  const pattern: WatermarkPattern = object.watermarkPattern ?? 'across-page';
  const textWidth = Math.max(1, object.text.length) * object.fontSize * 0.6;
  const centerX = area.x + area.width / 2, centerY = area.y + area.height / 2;
  if (pattern === 'bottom-right-to-top-left') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: -45 }];
  if (pattern === 'top-right-to-bottom-left') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 45 }];
  if (pattern === 'center-horizontal') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 0 }];
  if (pattern === 'center-vertical') return [{ x: centerX - textWidth / 2, y: centerY + object.fontSize / 2, angle: 90 }];
  const stepX = Math.max(100, textWidth + object.fontSize * 2), stepY = Math.max(80, object.fontSize * 4);
  const placements: { x: number; y: number; angle: number }[] = [];
  for (let y = area.y - area.height; y <= area.y + area.height * 2; y += stepY) {
    const offset = Math.round((y - area.y) / stepY) * stepX * 0.45;
    for (let x = area.x - area.width + offset; x <= area.x + area.width * 2; x += stepX) placements.push({ x, y, angle: -35 });
  }
  return placements;
}

function Artwork({ page, object }: { page: ScenePage; object: SceneObject }) {
  const { rect: r } = object;
  if (object.kind === 'signature') {
    if (object.signatureMode === 'image') return object.signaturePreview
      ? <image href={object.signaturePreview} x={r.x} y={r.y} width={r.width} height={r.height} preserveAspectRatio="xMidYMid meet" opacity={object.opacity} />
      : <rect {...r} fill="var(--scene-image-placeholder, #eef2ff)" stroke={object.color} strokeDasharray="5 4" opacity={object.opacity} />;
    if (object.signatureMode === 'text') return <svg x={r.x} y={r.y} width={r.width} height={r.height} overflow="hidden">
      <text fill={object.color} opacity={object.opacity} fontSize={object.fontSize} fontFamily={cssFont(object.fontFamily)} fontWeight={fontWeight(object.fontFamily)} fontStyle={fontStyle(object.fontFamily)} dominantBaseline="hanging">
        {object.text.split('\n').map((line, i) => <tspan key={i} x={0} y={i * object.fontSize * 1.2}>{line}</tspan>)}
      </text>
    </svg>;
    return <g fill="none" stroke={object.color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" opacity={object.opacity}>
      {object.strokes.map((stroke, i) => <polyline key={i} points={stroke.map(p => `${r.x + p.x * r.width},${r.y + p.y * r.height}`).join(' ')} />)}
    </g>;
  }
  if (object.kind === 'text') return <svg x={r.x} y={r.y} width={r.width} height={r.height} overflow="hidden">
    <text fill={object.color} opacity={object.opacity} fontSize={object.fontSize} fontFamily={cssFont(object.fontFamily)} fontWeight={fontWeight(object.fontFamily)} fontStyle={fontStyle(object.fontFamily)} dominantBaseline="hanging">
      {object.text.split('\n').map((line, i) => <tspan key={i} x={0} y={i * object.fontSize * 1.2}>{line}</tspan>)}
    </text>
  </svg>;
  if (object.kind === 'watermark') return <g fill={object.color} opacity={object.opacity} fontSize={object.fontSize} fontFamily={cssFont(object.fontFamily)} fontWeight={fontWeight(object.fontFamily)} fontStyle={fontStyle(object.fontFamily)}>
    {watermarkPlacements(page, object).map(({ x, y, angle }, index) => <text key={index} transform={`rotate(${angle} ${x} ${y})`} x={x} y={y} dominantBaseline="alphabetic">{object.text}</text>)}
  </g>;
  if (object.kind === 'highlight') return <rect {...r} fill={object.color} stroke="none" opacity={object.opacity} />;
  return shapeArtwork(object.shape, r, object.color, object.opacity);
}

export function SceneCanvas({ page, sourcePreview, textRuns = [], renderedPreview, tool, selectedId, zoom, onSelect, onCommit, onInteraction, makeObject }: SceneCanvasProps) {
  const host = useRef<HTMLDivElement>(null);
  const surface = useRef<SVGSVGElement>(null);
  const coordinates = useRef<SVGGElement>(null);
  const gesture = useRef<Gesture | null>(null);
  const spacePressed = useRef(false);
  const editInput = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState<ScenePage | null>(null);
  const [size, setSize] = useState({ width: 600, height: 600 });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState('');
  const [spaceHeld, setSpaceHeld] = useState(false);
  const [panning, setPanning] = useState(false);
  useLayoutEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { gesture.current = null; setDraft(null); setEditingId(null); setPanning(false); window.getSelection()?.removeAllRanges(); }, [page]);
  useEffect(() => {
    const clearGesture = () => {
      const active = gesture.current;
      if (!active) return;
      gesture.current = null;
      setDraft(null);
      setPanning(false);
      if (active.mode === 'text-selection') window.getSelection()?.removeAllRanges();
      if (surface.current?.hasPointerCapture(active.pointerId)) surface.current.releasePointerCapture(active.pointerId);
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
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
      clearGesture();
    };
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    window.addEventListener('pointerup', clearGesture);
    window.addEventListener('pointercancel', clearGesture);
    window.addEventListener('blur', onBlur);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
      window.removeEventListener('pointerup', clearGesture);
      window.removeEventListener('pointercancel', clearGesture);
      window.removeEventListener('blur', onBlur);
      const active = gesture.current;
      gesture.current = null;
      if (active && surface.current?.hasPointerCapture(active.pointerId)) surface.current.releasePointerCapture(active.pointerId);
      window.getSelection()?.removeAllRanges();
    };
  }, []);
  useEffect(() => {
    if (!editingId) return;
    requestAnimationFrame(() => { editInput.current?.focus(); editInput.current?.select(); });
  }, [editingId]);
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
  function beginTextEdit(event: Pick<ReactMouseEvent<SVGElement>, 'preventDefault' | 'stopPropagation'>, object: SceneObject) {
    if (object.kind !== 'text') return;
    event.preventDefault(); event.stopPropagation();
    onSelect(object.id); setEditingText(object.text); setEditingId(object.id);
  }
  function doubleClick(event: ReactMouseEvent<SVGSVGElement>) {
    // Pointer capture can retarget the second click to the SVG surface. Keep
    // the selected object as the fallback so double-clicking still enters the
    // inline editor after the first click selected its hit box.
    const id = (event.target as Element).closest('[data-object]')?.getAttribute('data-object') ?? selectedId;
    const object = id ? page.objects.find(item => item.id === id) : undefined;
    if (object) beginTextEdit(event, object);
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
    const textTarget = tool === 'highlight' && target.closest('[data-text-run]');
    const start = point(event);
    if (textTarget) {
      // Leave the browser's selection gesture intact. The selection is turned
      // into one or more scene highlights when the pointer is released.
      gesture.current = { pointerId: event.pointerId, start, base: page, next: page, mode: 'text-selection', points: [start], changed: false };
      surface.current?.setPointerCapture(event.pointerId);
      return;
    }
    const id = target.closest('[data-object]')?.getAttribute('data-object') ?? undefined;
    const corner = target.getAttribute('data-corner') ?? undefined;
    let next = page;
    let mode: Gesture['mode'];
    let objectId = id;
    const existing = id ? page.objects.find(object => object.id === id) : undefined;
    if (existing?.kind === 'watermark') { onSelect(id ?? null); event.preventDefault(); return; }
    if (id && tool !== 'crop' && (tool === 'select' || selectedId === id || corner)) { mode = corner ? 'resize' : 'move'; onSelect(id); }
    else if (tool === 'select') { onSelect(null); return; }
    else if (tool === 'crop') { mode = 'crop'; onSelect(null); }
    else if (tool === 'watermark') { onSelect(null); return; }
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
    onInteraction();
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
    else g.next = { ...g.next, objects: g.next.objects.map(object => {
      if (object.id !== g.id) return object;
      const original = g.base.objects.find(o => o.id === g.id) ?? object;
      let rect = rectangle(g.start, p);
      if (g.mode === 'move') rect = { ...original.rect,
        x: Math.max(b.x, Math.min(b.x + b.width - original.rect.width, original.rect.x + dx)),
        y: Math.max(b.y, Math.min(b.y + b.height - original.rect.height, original.rect.y + dy)) };
      if (g.mode === 'resize') {
        const r = original.rect;
        rect = rectangle({ x: g.corner?.includes('w') ? r.x + r.width : r.x, y: g.corner?.includes('n') ? r.y + r.height : r.y }, p);
      }
      if (g.mode === 'create' && object.kind === 'signature' && !object.signatureMode) {
        g.points.push(p);
        const xs = g.points.map(v => v.x), ys = g.points.map(v => v.y);
        rect = rectangle({ x: Math.min(...xs), y: Math.min(...ys) }, { x: Math.max(...xs), y: Math.max(...ys) });
        return { ...object, rect, strokes: [g.points.map(v => ({ x: (v.x - rect.x) / rect.width, y: (v.y - rect.y) / rect.height }))] };
      }
      return { ...object, rect };
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
    if (!cancel && g.changed) onCommit(g.next);
    else if (g.mode === 'create') onSelect(null);
  }
  function key(event: KeyboardEvent<SVGElement>, object: SceneObject, corner?: string) {
    if (event.key === 'Escape') { gesture.current = null; setDraft(null); onSelect(null); return; }
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onSelect(object.id); return; }
    if (object.kind === 'watermark') return;
    const steps: Record<string, ScenePoint> = { ArrowLeft: { x: -1, y: 0 }, ArrowRight: { x: 1, y: 0 }, ArrowUp: { x: 0, y: -1 }, ArrowDown: { x: 0, y: 1 } };
    const step = steps[event.key];
    if (!step && event.key !== 'Delete' && event.key !== 'Backspace') return;
    event.preventDefault(); event.stopPropagation(); onInteraction(); onSelect(object.id);
    const angle = page.rotation * Math.PI / 180, amount = event.shiftKey ? 10 : 1;
    const dx = step ? Math.round((step.x * Math.cos(angle) + step.y * Math.sin(angle)) * amount) : 0;
    const dy = step ? Math.round((-step.x * Math.sin(angle) + step.y * Math.cos(angle)) * amount) : 0;
    if (!step) { onSelect(null); onCommit({ ...page, objects: page.objects.filter(o => o.id !== object.id) }); return; }
    const r = object.rect;
    const rect = corner ? rectangle({ x: corner.includes('w') ? r.x + r.width : r.x, y: corner.includes('n') ? r.y + r.height : r.y },
      { x: Math.max(b.x, Math.min(b.x + b.width, (corner.includes('w') ? r.x : r.x + r.width) + dx)), y: Math.max(b.y, Math.min(b.y + b.height, (corner.includes('n') ? r.y : r.y + r.height) + dy)) })
      : { ...r, x: Math.max(b.x, Math.min(b.x + b.width - r.width, r.x + dx)), y: Math.max(b.y, Math.min(b.y + b.height - r.height, r.y + dy)) };
    onCommit({ ...page, objects: page.objects.map(o => o.id === object.id ? { ...o, rect } : o) });
  }
  const exact = renderedPreview && !draft && !editingId;
  return <div className="scene-canvas" ref={host}>
    <svg ref={surface} className={`scene-canvas-page scene-tool-${tool}${panning ? ' scene-pan-active' : spaceHeld ? ' scene-space-pan' : ''}`} aria-label="PDF page canvas" role="group" width={transform.width * scale} height={transform.height * scale} viewBox={`0 0 ${transform.width} ${transform.height}`} onPointerDown={down} onPointerMove={move} onPointerUp={e => finish(e)} onPointerCancel={e => finish(e, true)} onLostPointerCapture={e => finish(e, true)} onDoubleClick={doubleClick}>
      <rect width={transform.width} height={transform.height} fill="white" />
      {exact && <image href={renderedPreview.dataUrl} width={transform.width} height={transform.height} preserveAspectRatio="none" pointerEvents="none" />}
      <g ref={coordinates} transform={transform.matrix}>
        {!exact && <g pointerEvents="none">
          {sourcePreview && <image href={sourcePreview} x={0} y={0} width={page.width} height={page.height} preserveAspectRatio="none" />}
          {shown.objects.map(object => <Artwork key={object.id} page={page} object={object} />)}
        </g>}
        {tool === 'highlight' && textRuns.length > 0 && <g className="scene-text-layer" aria-label="Selectable PDF text">
          {textRuns.map((run, index) => <text key={`${index}-${run.x}-${run.y}`} className="scene-text-run" data-text-run="true" x={run.x} y={run.y + run.height} fontSize={Math.max(6, run.height)} fontFamily="Helvetica, Arial, sans-serif" fill="#000" opacity="0.001">{run.text}</text>)}
        </g>}
        {shown.objects.map((object, index) => <g key={object.id}>
          <rect {...object.rect} data-object={object.id} tabIndex={0} role="button" aria-label={`${object.kind} object ${index + 1}`} aria-pressed={selectedId === object.id} className={`scene-object-hit scene-object-${object.kind}`} onFocus={() => onSelect(object.id)} onKeyDown={e => key(e, object)} onDoubleClick={e => beginTextEdit(e, object)} />
          {selectedId === object.id && object.kind === 'text' && editingId === object.id && <foreignObject {...object.rect} data-text-editor="true" className="scene-inline-text-editor">
            <textarea ref={editInput} aria-label="Edit text object" value={editingText} onChange={event => setEditingText(event.target.value)} onBlur={() => finishTextEdit(true)} onPointerDown={event => event.stopPropagation()} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); finishTextEdit(false); } else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') { event.preventDefault(); finishTextEdit(true); } }} />
          </foreignObject>}
          {selectedId === object.id && editingId !== object.id && <g className="scene-selection">
            <rect {...object.rect} fill="none" stroke="var(--scene-accent, #2563eb)" strokeWidth={1.5 / scale} pointerEvents="none" />
            {object.kind !== 'watermark' && (['nw', 'ne', 'sw', 'se'] as const).map(corner => <rect key={corner} data-object={object.id} data-corner={corner} x={(corner.includes('w') ? object.rect.x : object.rect.x + object.rect.width) - 5 / scale} y={(corner.includes('n') ? object.rect.y : object.rect.y + object.rect.height) - 5 / scale} width={10 / scale} height={10 / scale} tabIndex={0} role="button" aria-label={`Resize ${object.kind} ${corner}`} className="scene-resize-handle" strokeWidth={1 / scale} onKeyDown={e => key(e, object, corner)} />)}
          </g>}
        </g>)}
        {draft && gesture.current?.mode === 'crop' && draft.crop && <rect {...draft.crop} className="scene-crop-outline" strokeWidth={2 / scale} pointerEvents="none" />}
      </g>
    </svg>
  </div>;
}
