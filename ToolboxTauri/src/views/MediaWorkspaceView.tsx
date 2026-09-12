import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type { AtomicToolId, ImagePreview, ToolDefinition, ToolResult } from "../contracts";

type TiffFrame = { key: string; path: string; page: number; preview: ImagePreview };
type TiffInspection =
  | { kind: "idle" }
  | { kind: "loading"; key: string }
  | { kind: "ready"; key: string; frames: TiffFrame[] }
  | { kind: "error"; key: string; message: string };

const mediaActions = toolsForWorkspaceId("media-tools");
const mediaIds = new Set<string>(mediaActions.map((tool) => tool.id));

const iconSizesByPreset: Record<string, number[]> = {
  macos: [16, 32, 128, 256, 512, 1024],
  favicon: [16, 32, 48, 64, 128, 256],
  ios: [20, 29, 40, 60, 76, 83, 1024],
  android: [48, 72, 96, 144, 192, 512],
  custom: [16, 32, 64, 128, 256, 512],
};

export const MediaWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [delay, setDelay] = useState(100);
  const [loop, setLoop] = useState(true);
  const [iconPreset, setIconPreset] = useState("macos");
  const [iconSizes, setIconSizes] = useState(iconSizesByPreset.macos);
  const [metadataMode, setMetadataMode] = useState<"inspect" | "strip">("strip");
  const [report, setReport] = useState<unknown[]>([]);
  const [selectedTiffFrameIndex, setSelectedTiffFrameIndex] = useState(0);
  const [tiffInspection, setTiffInspection] = useState<TiffInspection>({ kind: "idle" });
  const [activeToolId, setActiveToolId] = useState<AtomicToolId>(
    mediaIds.has(utility.id) ? utility.id : "icon-set",
  );
  useEffect(() => {
    setActiveToolId(mediaIds.has(utility.id) ? utility.id : "icon-set");
  }, [utility.id]);
  const activeUtility = UtilityRegistry.find((item) => item.id === activeToolId) ?? utility;

  useEffect(() => {
    setReport([]);
  }, [activeUtility.id]);

  const selectPreset = (preset: string) => {
    setIconPreset(preset);
    setIconSizes(iconSizesByPreset[preset] ?? iconSizesByPreset.custom);
  };

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="media-tools"
      utility={activeUtility}
      showFileOrdering={activeUtility.id !== "tiff-pages"}
      onRun={(paths) => {
        if (activeUtility.id === "icon-set") {
          return invoke<ToolResult>("generate_icon_set", {
            request: { paths, preset: iconPreset, sizes: iconSizes, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "gif-create") {
          return invoke<ToolResult>("create_gif", {
            request: { paths, frameDelayMs: delay, loopForever: loop, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "gif-extract") {
          return invoke<ToolResult>("extract_gif_frames", {
            request: { paths, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "tiff-pages") {
          return invoke<ToolResult>("process_tiff_pages", {
            request: {
              paths,
              pages: tiffInspection.kind === "ready"
                ? tiffInspection.frames.map(({ path, page }) => ({ path, page }))
                : [],
              outputLocation: "alongsideInput",
            },
          });
        }
        return invoke<ToolResult>("image_metadata", {
          request: { paths, outputLocation: "alongsideInput" },
        });
      }}
    >
      {({ files, run, loading, selectedFileIndex, selectFile }) => (
        <div className="workspace-control-panel">
          <WorkspaceCommandRail
            actions={mediaActions}
            activeId={activeToolId}
            onSelect={setActiveToolId}
            label="Media workspace tools"
          />
          <div>
            <h2 className="workspace-active-command">{activeUtility.title}</h2>
            <p className="workspace-panel-label">Specialized media outcome</p>
            <p className="workspace-panel-copy">Keep the same selected files while choosing a sequence, icon, or explicit metadata result.</p>
          </div>

          {(activeUtility.id === "gif-create" || activeUtility.id === "tiff-pages") && (
            <MediaFrameOrder files={files} activeUtility={activeUtility} selectedIndex={activeUtility.id === "tiff-pages" ? selectedTiffFrameIndex : selectedFileIndex} selectIndex={activeUtility.id === "tiff-pages" ? setSelectedTiffFrameIndex : selectFile} tiffInspection={tiffInspection} setTiffInspection={setTiffInspection} moveTiffFrame={(delta) => {
              setTiffInspection((current) => {
                if (current.kind !== "ready") return current;
                const nextIndex = selectedTiffFrameIndex + delta;
                if (nextIndex < 0 || nextIndex >= current.frames.length) return current;
                const frames = [...current.frames];
                [frames[selectedTiffFrameIndex], frames[nextIndex]] = [frames[nextIndex], frames[selectedTiffFrameIndex]];
                setSelectedTiffFrameIndex(nextIndex);
                return { ...current, frames };
              });
            }} />
          )}

          {activeUtility.id === "icon-set" && (
            <>
              <label className="workspace-field">
                <span>Icon preset</span>
                <select aria-label="Icon preset" value={iconPreset} onChange={(event) => selectPreset(event.target.value)}>
                  <option value="macos">macOS</option>
                  <option value="favicon">Favicon</option>
                  <option value="ios">iOS</option>
                  <option value="android">Android</option>
                  <option value="custom">Custom</option>
                </select>
              </label>
              <fieldset className="workspace-fieldset">
                <legend>Icon sizes</legend>
                <div className="icon-size-grid" aria-label="Icon size presets">
                  {iconSizesByPreset[iconPreset].map((size) => (
                    <label key={size} className="workspace-check icon-size-option">
                      <input
                        type="checkbox"
                        aria-label={`Icon size ${size}px`}
                        checked={iconSizes.includes(size)}
                        onChange={(event) => setIconSizes((current) => event.target.checked ? [...new Set([...current, size])].sort((left, right) => left - right) : current.filter((value) => value !== size))}
                      />
                      {size}px
                    </label>
                  ))}
                </div>
              </fieldset>
            </>
          )}

          {activeUtility.id === "gif-create" && (
            <div className="workspace-field-grid">
              <label className="workspace-field"><span>Frame delay</span><input aria-label="Frame delay in milliseconds" type="number" min="1" max="60000" value={delay} onChange={(event) => setDelay(Number(event.target.value))} /></label>
              <label className="workspace-check"><input aria-label="Loop animation" type="checkbox" checked={loop} onChange={(event) => setLoop(event.target.checked)} /> Loop animation</label>
            </div>
          )}

          {activeUtility.id === "image-metadata" && (
            <>
              <fieldset className="workspace-fieldset">
                <legend>Metadata mode</legend>
                <label className="workspace-check"><input type="radio" name="metadata-mode" checked={metadataMode === "inspect"} onChange={() => setMetadataMode("inspect")} /> Inspect metadata</label>
                <label className="workspace-check"><input type="radio" name="metadata-mode" checked={metadataMode === "strip"} onChange={() => setMetadataMode("strip")} /> Remove metadata in a new copy</label>
              </fieldset>
              <button type="button" className="workspace-secondary-action" disabled={loading || files.length === 0} onClick={async () => setReport(await invoke<unknown[]>("inspect_image_metadata", { request: { paths: files, outputLocation: "alongsideInput" } }))}>Inspect metadata</button>
              {report.length > 0 && <pre className="workspace-report">{JSON.stringify(report, null, 2)}</pre>}
            </>
          )}

          <button type="button" disabled={loading || files.length === 0 || (activeUtility.id === "tiff-pages" && tiffInspection.kind !== "ready") || (activeUtility.id === "image-metadata" && metadataMode === "inspect")} onClick={run} className="workspace-primary-action">
            {activeUtility.id === "image-metadata" ? "Remove metadata" : activeUtility.shortTitle}
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};

const MediaFrameOrder = ({
  files,
  activeUtility,
  selectedIndex,
  selectIndex,
  tiffInspection,
  setTiffInspection,
  moveTiffFrame,
}: {
  files: string[];
  activeUtility: ToolDefinition;
  selectedIndex: number;
  selectIndex: (index: number) => void;
  tiffInspection: TiffInspection;
  setTiffInspection: (update: TiffInspection | ((current: TiffInspection) => TiffInspection)) => void;
  moveTiffFrame: (delta: -1 | 1) => void;
}) => {
  const [previews, setPreviews] = useState<Record<string, ImagePreview | null>>({});
  const frameKey = files.join("\u0000");
  const isTiff = activeUtility.id === "tiff-pages";

  useEffect(() => {
    if (isTiff || !frameKey || !activeUtility.capability.supportsPreview || activeUtility.id !== "gif-create") {
      setPreviews({});
      return;
    }
    let current = true;
    setPreviews({});
    void Promise.all(
      files.map(async (path) => {
        try {
          return [path, await invoke<ImagePreview>("inspect_image_preview", { request: { path } })] as const;
        } catch {
          return [path, null] as const;
        }
      }),
    ).then((entries) => {
      if (current) setPreviews(Object.fromEntries(entries));
    });
    return () => {
      current = false;
    };
  }, [activeUtility.capability.supportsPreview, activeUtility.id, frameKey, isTiff]);

  useEffect(() => {
    if (!isTiff) {
      setTiffInspection({ kind: "idle" });
      return;
    }
    if (!frameKey) {
      setTiffInspection({ kind: "idle" });
      selectIndex(0);
      return;
    }
    let current = true;
    setTiffInspection({ kind: "loading", key: frameKey });
    selectIndex(0);
    void Promise.all(files.map(async (path) => {
      const pages = await invoke<ImagePreview[]>("inspect_tiff_pages", { request: { path } });
      return pages.map((preview, page) => ({ key: path + "\u0000" + page, path, page, preview }));
    })).then((groups) => {
      if (current) setTiffInspection({ kind: "ready", key: frameKey, frames: groups.flat() });
    }).catch((error) => {
      if (current) setTiffInspection({ kind: "error", key: frameKey, message: String(error) });
    });
    return () => {
      current = false;
    };
  }, [files, frameKey, isTiff, selectIndex]);

  const tiffFrames = tiffInspection.kind === "ready" && tiffInspection.key === frameKey ? tiffInspection.frames : [];

  return (
    <section className="media-frame-order" role="region" aria-label="Frame order">
      <p className="workspace-panel-label">Frame order</p>
      <p className="workspace-panel-copy">The output follows this order. Select a frame, then move it with the arrows.</p>
      {isTiff && tiffInspection.kind === "loading" && <p className="workspace-note" role="status">Reading TIFF pages on this device.</p>}
      {isTiff && tiffInspection.kind === "error" && <p className="workspace-note" role="alert">TIFF pages could not be read. {tiffInspection.message}</p>}
      <ol className="media-frame-list">
        {(isTiff ? tiffFrames : files).map((item, index) => {
          const file = isTiff ? (item as TiffFrame).path : item as string;
          const preview = isTiff ? (item as TiffFrame).preview : previews[file];
          const page = isTiff ? (item as TiffFrame).page : null;
          return (
            <li key={isTiff ? (item as TiffFrame).key : file} data-selected={index === selectedIndex ? "true" : undefined}>
              <button type="button" onClick={() => selectIndex(index)} aria-label={"Frame " + (index + 1)}>
                <span>{index + 1}</span>
                {activeUtility.capability.supportsPreview && preview && <img className="media-frame-preview" src={preview.dataUrl} alt={"Preview of frame " + (index + 1)} />}
                {file.split(/[\\/]/).pop()}{page === null ? "" : " · page " + (page + 1)}
              </button>
            </li>
          );
        })}
      </ol>
      {((isTiff && tiffFrames.length === 0 && tiffInspection.kind !== "error") || (!isTiff && files.length === 0)) && <p className="workspace-note">Select frames to arrange them here.</p>}
      {isTiff && tiffFrames.length > 0 && (
        <div className="file-selection-order" aria-label="Selected TIFF page ordering">
          <button type="button" aria-label="Move selected TIFF page up" disabled={selectedIndex === 0} onClick={() => moveTiffFrame(-1)}>↑</button>
          <button type="button" aria-label="Move selected TIFF page down" disabled={selectedIndex >= tiffFrames.length - 1} onClick={() => moveTiffFrame(1)}>↓</button>
        </div>
      )}
    </section>
  );
};
