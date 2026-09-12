import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type { AtomicToolId, ImagePreview, ToolDefinition, ToolResult } from "../contracts";

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
            request: { paths, outputLocation: "alongsideInput" },
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
            <MediaFrameOrder files={files} activeUtility={activeUtility} selectedFileIndex={selectedFileIndex} selectFile={selectFile} />
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

          <button type="button" disabled={loading || files.length === 0 || (activeUtility.id === "image-metadata" && metadataMode === "inspect")} onClick={run} className="workspace-primary-action">
            {activeUtility.id === "image-metadata" ? "Remove metadata" : activeUtility.shortTitle}
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};

const MediaFrameOrder = ({ files, activeUtility, selectedFileIndex, selectFile }: { files: string[]; activeUtility: ToolDefinition; selectedFileIndex: number; selectFile: (index: number) => void }) => {
  const [previews, setPreviews] = useState<Record<string, ImagePreview | null>>({});
  const frameKey = files.join("\u0000");

  useEffect(() => {
    if (!frameKey || !activeUtility.capability.supportsPreview || !["gif-create", "tiff-pages"].includes(activeUtility.id)) {
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
  }, [activeUtility.capability.supportsPreview, activeUtility.id, frameKey]);

  return (
    <section className="media-frame-order" role="region" aria-label="Frame order">
      <p className="workspace-panel-label">Frame order</p>
      <p className="workspace-panel-copy">The output follows this order. Select a frame, then move it with the arrows.</p>
      <ol className="media-frame-list">
        {files.map((file, index) => (
          <li key={file} data-selected={index === selectedFileIndex ? "true" : undefined}>
            <button type="button" onClick={() => selectFile(index)} aria-label={`Frame ${index + 1}`}>
              <span>{index + 1}</span>
              {activeUtility.capability.supportsPreview && previews[file] && <img className="media-frame-preview" src={previews[file]?.dataUrl} alt={`Preview of frame ${index + 1}`} />}
              {file.split(/[\\/]/).pop()}
            </button>
          </li>
        ))}
      </ol>
      {files.length === 0 && <p className="workspace-note">Select frames to arrange them here.</p>}
    </section>
  );
};
