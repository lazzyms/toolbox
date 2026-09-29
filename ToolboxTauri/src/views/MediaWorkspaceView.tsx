import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type { AtomicToolId, ImagePreview, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

type TiffFrame = { key: string; path: string; page: number; preview: ImagePreview };
export type TiffInspection =
  | { kind: "idle" }
  | { kind: "loading"; key: string }
  | { kind: "ready"; key: string; frames: TiffFrame[] }
  | { kind: "error"; key: string; message: string };

export const tiffInspectionKey = (files: string[]) => files.join("\u0000");
export const isCurrentTiffInspection = (inspection: TiffInspection, files: string[]): inspection is Extract<TiffInspection, { kind: "ready" }> =>
  inspection.kind === "ready" && inspection.key === tiffInspectionKey(files);

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
              pages: isCurrentTiffInspection(tiffInspection, paths)
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
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
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
              <div className="workspace-field">
                <Label htmlFor="icon-preset">Icon preset</Label>
                <NativeSelect id="icon-preset" aria-label="Icon preset" value={iconPreset} onChange={(event) => selectPreset(event.target.value)}>
                  <option value="macos">macOS</option>
                  <option value="favicon">Favicon</option>
                  <option value="ios">iOS</option>
                  <option value="android">Android</option>
                  <option value="custom">Custom</option>
                </NativeSelect>
              </div>
              <fieldset className="workspace-fieldset">
                <legend>Icon sizes</legend>
                <div className="icon-size-grid" aria-label="Icon size presets">
                  {iconSizesByPreset[iconPreset].map((size) => (
                    <Label key={size} className="workspace-check" htmlFor={`icon-size-${size}`}>
                      <Checkbox
                        id={`icon-size-${size}`}
                        aria-label={`Icon size ${size}px`}
                        checked={iconSizes.includes(size)}
                        onCheckedChange={(checked) => setIconSizes((current) => checked ? [...new Set([...current, size])].sort((left, right) => left - right) : current.filter((value) => value !== size))}
                      />
                      <span>{size}px</span>
                    </Label>
                  ))}
                </div>
              </fieldset>
            </>
          )}

          {activeUtility.id === "gif-create" && (
            <div className="workspace-field-grid">
              <div className="workspace-field">
                <Label htmlFor="frame-delay">Frame delay</Label>
                <Input
                  id="frame-delay"
                  type="number"
                  aria-label="Frame delay in milliseconds"
                  min="1"
                  max="60000"
                  value={delay}
                  onChange={(event) => setDelay(Number(event.target.value))}
                />
              </div>
              <div className="media-toggle-field">
                <Label htmlFor="gif-loop">Loop animation</Label>
                <Switch id="gif-loop" checked={loop} onCheckedChange={setLoop} />
              </div>
            </div>
          )}

          {activeUtility.id === "image-metadata" && (
            <>
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                aria-label="Metadata mode"
                value={metadataMode}
                onValueChange={(value) => {
                  if (value) setMetadataMode(value as typeof metadataMode);
                }}
              >
                <ToggleGroupItem value="inspect">Inspect metadata</ToggleGroupItem>
                <ToggleGroupItem value="strip">Remove metadata in a new copy</ToggleGroupItem>
              </ToggleGroup>
              <Button
                variant="ghost"
                size="sm"
                disabled={loading || files.length === 0}
                onClick={async () => setReport(await invoke<unknown[]>("inspect_image_metadata", { request: { paths: files, outputLocation: "alongsideInput" } }))}
              >
                Inspect metadata
              </Button>
              {report.length > 0 && <pre className="workspace-report">{JSON.stringify(report, null, 2)}</pre>}
            </>
          )}

          <Button variant="default" disabled={loading || files.length === 0 || (activeUtility.id === "tiff-pages" && !isCurrentTiffInspection(tiffInspection, files)) || (activeUtility.id === "image-metadata" && metadataMode === "inspect")} onClick={run} className="workspace-primary-action">
            {activeUtility.id === "image-metadata" ? "Remove metadata" : activeUtility.shortTitle}
          </Button>
          </CardContent>
        </Card>
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
  const frameKey = tiffInspectionKey(files);
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

  const tiffFrames = isCurrentTiffInspection(tiffInspection, files) ? tiffInspection.frames : [];

  return (
    <Card className="media-frame-order py-0" role="region" aria-label="Frame order">
      <CardContent className="media-frame-order-content py-4">
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
              <Button variant={index === selectedIndex ? "secondary" : "ghost"} size="sm" aria-pressed={index === selectedIndex} onClick={() => selectIndex(index)} aria-label={"Frame " + (index + 1)}>
                <span>{index + 1}</span>
                {activeUtility.capability.supportsPreview && preview && <img className="media-frame-preview" src={preview.dataUrl} alt={"Preview of frame " + (index + 1)} />}
                {file.split(/[\\/]/).pop()}{page === null ? "" : " · page " + (page + 1)}
              </Button>
            </li>
          );
        })}
      </ol>
      {((isTiff && tiffFrames.length === 0 && tiffInspection.kind !== "error") || (!isTiff && files.length === 0)) && <p className="workspace-note">Select frames to arrange them here.</p>}
      {isTiff && tiffFrames.length > 0 && (
        <div className="file-selection-order" aria-label="Selected TIFF page ordering">
          <Button variant="ghost" size="icon-sm" aria-label="Move selected TIFF page up" disabled={selectedIndex === 0} onClick={() => moveTiffFrame(-1)}>↑</Button>
          <Button variant="ghost" size="icon-sm" aria-label="Move selected TIFF page down" disabled={selectedIndex >= tiffFrames.length - 1} onClick={() => moveTiffFrame(1)}>↓</Button>
        </div>
      )}
      </CardContent>
    </Card>
  );
};
