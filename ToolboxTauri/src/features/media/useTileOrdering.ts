import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { matchShortcut } from "../../shortcuts";

const DRAG_THRESHOLD_PX = 4;

export interface TileDrag {
  index: number;
  over: number | null;
}

/**
 * The shared ordering pattern for a grid of tiles, such as GIF frames or TIFF pages: arrows select,
 * the modifier and an arrow moves the selected tile, the remove shortcut takes it out, and a tile moves
 * by dragging it onto another. Each tile is a button inside an option, with one tab stop on the selection.
 */
export const useTileOrdering = ({ keys, selectedIndex, selectIndex, move, remove, disabled = false }: {
  /** Each tile's key, in order. */
  keys: string[];
  selectedIndex: number;
  selectIndex: (index: number) => void;
  move: (index: number, toIndex: number) => void;
  remove: (index: number) => void;
  disabled?: boolean;
}) => {
  const [drag, setDrag] = useState<TileDrag | null>(null);
  const pointer = useRef<{ index: number; x: number; y: number; moved: boolean } | null>(null);
  const clickAfterDrag = useRef(false);
  const tiles = useRef(new Map<string, HTMLButtonElement>());
  const focusTile = (key: string | undefined) => requestAnimationFrame(() => key && tiles.current.get(key)?.focus());
  const tileAt = (x: number, y: number) => {
    const key = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-frame-key]")?.dataset.frameKey;
    const index = key === undefined ? -1 : keys.indexOf(key);
    return index >= 0 ? index : null;
  };
  const endDrag = () => {
    pointer.current = null;
    setDrag(null);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (disabled || keys.length === 0) return;
    const delta = event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : 0;
    if (delta && !event.altKey) {
      event.preventDefault();
      const to = selectedIndex + delta;
      if (to < 0 || to >= keys.length) return;
      if (event.metaKey || event.ctrlKey) {
        move(selectedIndex, to);
        focusTile(keys[selectedIndex]);
      } else {
        selectIndex(to);
        focusTile(keys[to]);
      }
      return;
    }
    if (matchShortcut(event.nativeEvent, "file-list") === "remove") {
      event.preventDefault();
      const next = keys[selectedIndex + 1] ?? keys[selectedIndex - 1];
      remove(selectedIndex);
      focusTile(next);
    }
  };

  /** The option's attributes: its key, and whether it is dragged or a drop target. */
  const itemProps = (index: number) => ({
    "data-frame-key": keys[index],
    "data-dragging": drag?.index === index ? "true" : undefined,
    "data-drop-target": drag && drag.over === index && drag.index !== index ? (drag.over < drag.index ? "before" : "after") : undefined,
  });

  /** The tile button's focus, click and pointer handling. */
  const buttonProps = (index: number) => ({
    ref: (element: HTMLButtonElement | null) => {
      if (element) tiles.current.set(keys[index], element);
      else tiles.current.delete(keys[index]);
    },
    tabIndex: index === selectedIndex ? 0 : -1,
    disabled,
    onClick: () => {
      if (clickAfterDrag.current) {
        clickAfterDrag.current = false;
        return;
      }
      selectIndex(index);
    },
    onPointerDown: (event: PointerEvent<HTMLButtonElement>) => {
      clickAfterDrag.current = false;
      if (event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      pointer.current = { index, x: event.clientX, y: event.clientY, moved: false };
    },
    onPointerMove: (event: PointerEvent<HTMLButtonElement>) => {
      const start = pointer.current;
      if (!start) return;
      if (!start.moved && Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD_PX) return;
      start.moved = true;
      setDrag({ index: start.index, over: tileAt(event.clientX, event.clientY) });
    },
    onPointerUp: (event: PointerEvent<HTMLButtonElement>) => {
      const start = pointer.current;
      endDrag();
      if (!start?.moved) return;
      clickAfterDrag.current = true;
      const to = tileAt(event.clientX, event.clientY);
      if (to === null || to === start.index) return;
      move(start.index, to);
      focusTile(keys[start.index]);
    },
    onPointerCancel: endDrag,
  });

  return { drag, onKeyDown, itemProps, buttonProps };
};
