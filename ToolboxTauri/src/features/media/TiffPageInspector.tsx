import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ImagePreview } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

export type TiffPage = { key: string; path: string; page: number; preview: ImagePreview };
export type TiffInspection =
  | { kind: "idle" }
  | { kind: "loading"; key: string }
  | { kind: "ready"; key: string; frames: TiffPage[] }
  | { kind: "error"; key: string; message: string };

/** Split writes each file's pages separately; Combine joins every page into one TIFF. Never inferred from file count. */
export type TiffMode = "split" | "combine";

export interface TiffPagesState {
  mode: TiffMode;
  inspection: TiffInspection;
  selectedIndex: number;
}

export const tiffInspectionKey = (files: string[]) => files.join("\u0000");
export const isCurrentTiffInspection = (inspection: TiffInspection, files: string[]): inspection is Extract<TiffInspection, { kind: "ready" }> =>
  inspection.kind === "ready" && inspection.key === tiffInspectionKey(files);

/** Reads every page of the selected TIFF files and lets the person reorder the pages. */
export const TiffPageInspector = ({
  files,
  supportsPreview,
  state,
  setState,
}: {
  files: string[];
  supportsPreview: boolean;
  state: TiffPagesState;
  setState: (update: (current: TiffPagesState) => TiffPagesState) => void;
}) => {
  const frameKey = tiffInspectionKey(files);
  const { inspection, selectedIndex } = state;

  useEffect(() => {
    if (!frameKey) {
      setState((latest) => ({ ...latest, inspection: { kind: "idle" }, selectedIndex: 0 }));
      return;
    }
    let current = true;
    setState((latest) => ({ ...latest, inspection: { kind: "loading", key: frameKey }, selectedIndex: 0 }));
    void Promise.all(files.map(async (path) => {
      const pages = await invoke<ImagePreview[]>("inspect_tiff_pages", { request: { path } });
      return pages.map((preview, page) => ({ key: path + "\u0000" + page, path, page, preview }));
    })).then((groups) => {
      if (current) setState((latest) => ({ ...latest, inspection: { kind: "ready", key: frameKey, frames: groups.flat() } }));
    }).catch((error) => {
      if (current) setState((latest) => ({ ...latest, inspection: { kind: "error", key: frameKey, message: String(error) } }));
    });
    return () => {
      current = false;
    };
  }, [files, frameKey, setState]);

  // Pages are read again whenever the inspector returns, so none outlive it.
  useEffect(() => () => setState((latest) => ({ ...latest, inspection: { kind: "idle" } })), [setState]);

  const movePage = (delta: -1 | 1) => setState((latest) => {
    if (latest.inspection.kind !== "ready") return latest;
    const nextIndex = latest.selectedIndex + delta;
    if (nextIndex < 0 || nextIndex >= latest.inspection.frames.length) return latest;
    const frames = [...latest.inspection.frames];
    [frames[latest.selectedIndex], frames[nextIndex]] = [frames[nextIndex], frames[latest.selectedIndex]];
    return { ...latest, inspection: { ...latest.inspection, frames }, selectedIndex: nextIndex };
  });

  const pages = isCurrentTiffInspection(inspection, files) ? inspection.frames : [];

  return (
    <Card className="media-frame-order py-0" role="region" aria-label="Frame order">
      <CardContent className="media-frame-order-content py-4">
      <p className="workspace-panel-label">Frame order</p>
      <p className="workspace-panel-copy">The output follows this order. Select a frame, then move it with the arrows.</p>
      {inspection.kind === "loading" && <p className="workspace-note" role="status">Reading TIFF pages on this device.</p>}
      {inspection.kind === "error" && <p className="workspace-note" role="alert">TIFF pages could not be read. {inspection.message}</p>}
      <ol className="media-frame-list">
        {pages.map(({ key, path, page, preview }, index) => (
          <li key={key} data-selected={index === selectedIndex ? "true" : undefined}>
            <Button variant={index === selectedIndex ? "secondary" : "ghost"} size="sm" aria-pressed={index === selectedIndex} onClick={() => setState((latest) => ({ ...latest, selectedIndex: index }))} aria-label={"Frame " + (index + 1)}>
              <span>{index + 1}</span>
              {supportsPreview && preview && <img className="media-frame-preview" src={preview.dataUrl} alt={"Preview of frame " + (index + 1)} />}
              {path.split(/[\\/]/).pop()}{" · page " + (page + 1)}
            </Button>
          </li>
        ))}
      </ol>
      {pages.length === 0 && inspection.kind !== "error" && <p className="workspace-note">Select frames to arrange them here.</p>}
      {pages.length > 0 && (
        <div className="file-selection-order" aria-label="Selected TIFF page ordering">
          <Button variant="ghost" size="icon-sm" aria-label="Move selected TIFF page up" disabled={selectedIndex === 0} onClick={() => movePage(-1)}>↑</Button>
          <Button variant="ghost" size="icon-sm" aria-label="Move selected TIFF page down" disabled={selectedIndex >= pages.length - 1} onClick={() => movePage(1)}>↓</Button>
        </div>
      )}
      </CardContent>
    </Card>
  );
};
