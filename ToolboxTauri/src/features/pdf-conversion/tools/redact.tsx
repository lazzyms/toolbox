import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { failedOutcome } from "../../../jobOutcome";
import { displayedBoxToPdfRect, type DisplayedBox } from "../../pdf-editor";
import { Button } from "@/components/ui/button";
import type { ConversionTool, OptionsPanelProps } from "../tool";

type Options = { box: DisplayedBox | null };

const RedactOptions = ({ options, setOptions, inspectedDocument, selectedPages }: OptionsPanelProps<Options>) => {
  useEffect(() => setOptions({ box: null }), [inspectedDocument, setOptions]);
  const redactPage = inspectedDocument?.pages[selectedPages[0] ?? -1];
  if (!redactPage?.preview) return null;
  return (
    <RedactionArea page={redactPage.index} preview={redactPage.preview} aspectRatio={redactPage.width / redactPage.height} box={options.box} onChange={(box) => setOptions({ box })} />
  );
};

export const redact: ConversionTool<Options> = {
  initialOptions: { box: null },
  inspectsPdf: true,
  // Blacking out every page by accident is easy to miss, so redaction starts on the first page only.
  initialPages: (document) => document.pages.slice(0, 1).map((page) => page.index),
  outputPreview: () => "Output preview unavailable. Export saves a copy with the boxed area removed for good. Redacted pages become images.",
  exportBlocked: ({ box }) => !box,
  OptionsPanel: RedactOptions,
  request: (paths, outputLocation, { runId }, { box }, { document, selectedPages }) => {
    const drawnOn = document?.pages[selectedPages[0] ?? 0];
    if (!box || !drawnOn) {
      return Promise.resolve([failedOutcome(paths[0] ?? "", "invalidInput", "Draw a box over what you want to hide first.")]);
    }
    const allPages = selectedPages.length === document.pages.length;
    return invoke<ToolResult>("redact_pdf", { runId,
      request: {
        paths,
        rectangle: displayedBoxToPdfRect(box, drawnOn),
        scope: allPages ? "all" : { selected: { pages: selectedPages } },
        outputLocation,
      },
    });
  },
};

const clampFraction = (value: number) => Math.min(Math.max(value, 0), 1);

const RedactionArea = ({ page, preview, aspectRatio, box, onChange }: { page: number; preview: string; aspectRatio: number; box: DisplayedBox | null; onChange: (box: DisplayedBox | null) => void }) => {
  const [start, setStart] = useState<{ x: number; y: number } | null>(null);
  const fractionAt = (event: React.PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    return { x: clampFraction((event.clientX - bounds.left) / bounds.width), y: clampFraction((event.clientY - bounds.top) / bounds.height) };
  };
  const boxFrom = (from: { x: number; y: number }, to: { x: number; y: number }): DisplayedBox => ({
    left: Math.min(from.x, to.x), top: Math.min(from.y, to.y), right: Math.max(from.x, to.x), bottom: Math.max(from.y, to.y),
  });
  return (
    <div className="grid gap-2">
      <p className="workspace-note">Drag over the details you want to hide on page {page + 1}. The same area is removed on every checked page.</p>
      <div
        role="region"
        aria-label="Redaction area"
        className="relative cursor-crosshair touch-none select-none border"
        style={{ aspectRatio }}
        onPointerDown={(event) => { event.currentTarget.setPointerCapture(event.pointerId); const point = fractionAt(event); setStart(point); onChange(null); }}
        onPointerMove={(event) => { if (start) onChange(boxFrom(start, fractionAt(event))); }}
        onPointerUp={(event) => {
          if (!start) return;
          const next = boxFrom(start, fractionAt(event));
          setStart(null);
          onChange(next.right - next.left > 0.005 && next.bottom - next.top > 0.005 ? next : null);
        }}
        onPointerCancel={() => { setStart(null); onChange(null); }}
      >
        <img src={preview} alt={`Page ${page + 1} to redact`} draggable={false} className="block size-full" />
        {box && (
          <div
            aria-label="Redaction box"
            className="absolute bg-black/80"
            style={{ left: `${box.left * 100}%`, top: `${box.top * 100}%`, width: `${(box.right - box.left) * 100}%`, height: `${(box.bottom - box.top) * 100}%` }}
          />
        )}
      </div>
      {box && <Button variant="outline" size="sm" className="justify-self-start" onClick={() => onChange(null)}>Clear box</Button>}
    </div>
  );
};
