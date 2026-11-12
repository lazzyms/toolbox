import { useRef, useState, type ReactNode } from "react";
import type { ImagePreview } from "../../contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { fileName } from "../../components/intake";
import { isWindows, matchShortcut } from "../../shortcuts";

/** One frame of the GIF: its list entry, and its delay when that is in range. */
export interface GifFrame {
  key: string;
  path: string;
  own: boolean;
  delayMs: number | null;
}

const delayText = (frame: GifFrame) => frame.delayMs === null ? "Check delay" : `${frame.delayMs} ms`;
const frameLabel = (frame: GifFrame, index: number) =>
  `Frame ${index + 1}, ${fileName(frame.path)}, ${delayText(frame)}${frame.own ? ", own delay" : ""}`;
const loopLength = (ms: number) => `${Number((ms / 1000).toFixed(2))} s per loop`;
const DRAG_THRESHOLD_PX = 4;

/**
 * The frames of a new GIF, in play order. A frame moves by dragging it or with
 * the modifier and an arrow key, through the shared file list's ordering.
 */
export const GifFrameStrip = ({ frames, previews, totalMs, supportsPreview, selectedIndex, selectIndex, moveFrame, removeFrame, disabled, notice, children }: {
  frames: GifFrame[];
  /** Each path's preview, from the panel, so the strip and the playback share one fetch. */
  previews: Record<string, ImagePreview | null>;
  /** The sum of every frame's delay, or null while a delay is out of range. */
  totalMs: number | null;
  supportsPreview: boolean;
  selectedIndex: number;
  selectIndex: (index: number) => void;
  moveFrame: (index: number, toIndex: number) => void;
  removeFrame: (index: number) => void;
  disabled: boolean;
  /** A warning shown above the frames, such as a large GIF. */
  notice?: ReactNode;
  /** Shown below the frames, such as the playback preview. */
  children?: ReactNode;
}) => {
  const [drag, setDrag] = useState<{ index: number; over: number | null } | null>(null);
  const pointer = useRef<{ index: number; x: number; y: number; moved: boolean } | null>(null);
  const clickAfterDrag = useRef(false);
  const tiles = useRef(new Map<string, HTMLButtonElement>());
  const focusFrame = (key: string | undefined) => requestAnimationFrame(() => key && tiles.current.get(key)?.focus());
  const frameAt = (x: number, y: number) => {
    const key = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-frame-key]")?.dataset.frameKey;
    const index = frames.findIndex((frame) => frame.key === key);
    return index >= 0 ? index : null;
  };
  const endDrag = () => {
    pointer.current = null;
    setDrag(null);
  };

  const keyDown = (event: React.KeyboardEvent<HTMLOListElement>) => {
    if (disabled || frames.length === 0) return;
    const delta = event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : 0;
    if (delta && !event.altKey) {
      event.preventDefault();
      const to = selectedIndex + delta;
      if (to < 0 || to >= frames.length) return;
      if (event.metaKey || event.ctrlKey) {
        moveFrame(selectedIndex, to);
        focusFrame(frames[selectedIndex].key);
      } else {
        selectIndex(to);
        focusFrame(frames[to].key);
      }
      return;
    }
    if (matchShortcut(event.nativeEvent, "file-list") === "remove") {
      event.preventDefault();
      removeFrame(selectedIndex);
    }
  };

  const selected = frames[selectedIndex];
  const dragged = drag && frames[drag.index];
  const status = dragged
    ? `Dragging ${fileName(dragged.path)}.${drag.over === null ? "" : ` Drop it at position ${drag.over + 1}.`}`
    : selected
      ? `Frame ${selectedIndex + 1}, ${fileName(selected.path)}, selected.${selected.own && selected.delayMs !== null ? ` Own delay ${selected.delayMs} ms.` : ""}`
      : "";
  const modifier = isWindows() ? "Ctrl" : "⌘";

  return (
    <Card className="media-frame-order py-0">
      <CardContent className="media-frame-order-content py-4">
        <div className="media-frame-heading">
          <p className="workspace-panel-label">{frames.length === 1 ? "1 frame" : `${frames.length} frames`}</p>
          {frames.length > 0 && totalMs !== null && <p className="workspace-note">{loopLength(totalMs)}</p>}
        </div>
        {notice}
        <p className="workspace-panel-copy">Frames play in this order. Drag a frame, or select it and press {modifier}+Arrow to move it. Delete removes it.</p>
        {frames.length === 0 ? (
          <p className="workspace-note">Select frames to arrange them here.</p>
        ) : (
          <ol className="media-frame-grid" role="listbox" aria-label="Frames, in play order" aria-orientation="horizontal" onKeyDown={keyDown} data-dragging={drag ? "true" : undefined}>
            {frames.map((frame, index) => {
              const isSelected = index === selectedIndex;
              const label = frameLabel(frame, index);
              const preview = previews[frame.path];
              return (
                <li
                  key={frame.key}
                  role="option"
                  aria-selected={isSelected}
                  aria-label={label}
                  data-frame-key={frame.key}
                  data-dragging={drag?.index === index ? "true" : undefined}
                  data-drop-target={drag && drag.over === index && drag.index !== index ? (drag.over < drag.index ? "before" : "after") : undefined}
                  className="media-frame-tile"
                >
                  <Button
                    ref={(element) => {
                      if (element) tiles.current.set(frame.key, element);
                      else tiles.current.delete(frame.key);
                    }}
                    variant={isSelected ? "secondary" : "ghost"}
                    className="media-frame-tile-button"
                    aria-label={label}
                    tabIndex={isSelected ? 0 : -1}
                    disabled={disabled}
                    onClick={() => {
                      if (clickAfterDrag.current) {
                        clickAfterDrag.current = false;
                        return;
                      }
                      selectIndex(index);
                    }}
                    onPointerDown={(event) => {
                      clickAfterDrag.current = false;
                      if (event.button !== 0) return;
                      event.currentTarget.setPointerCapture(event.pointerId);
                      pointer.current = { index, x: event.clientX, y: event.clientY, moved: false };
                    }}
                    onPointerMove={(event) => {
                      const start = pointer.current;
                      if (!start) return;
                      if (!start.moved && Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD_PX) return;
                      start.moved = true;
                      setDrag({ index: start.index, over: frameAt(event.clientX, event.clientY) });
                    }}
                    onPointerUp={(event) => {
                      const start = pointer.current;
                      endDrag();
                      if (!start?.moved) return;
                      clickAfterDrag.current = true;
                      const to = frameAt(event.clientX, event.clientY);
                      if (to === null || to === start.index) return;
                      moveFrame(start.index, to);
                      focusFrame(frames[start.index].key);
                    }}
                    onPointerCancel={endDrag}
                  >
                    <span className="media-frame-picture" aria-hidden="true">
                      {supportsPreview && preview && <img src={preview.dataUrl} alt="" draggable={false} />}
                    </span>
                    <span className="media-frame-meta" aria-hidden="true">
                      <span>{index + 1}</span>
                      <span data-own={frame.own ? "true" : undefined} data-invalid={frame.delayMs === null ? "true" : undefined}>{delayText(frame)}</span>
                    </span>
                  </Button>
                  {index === 0 && <Badge variant="outline" className="media-frame-tag">Sets size</Badge>}
                </li>
              );
            })}
          </ol>
        )}
        {status && <p className="workspace-note" role="status" aria-live="polite">{status}</p>}
        {children}
      </CardContent>
    </Card>
  );
};
