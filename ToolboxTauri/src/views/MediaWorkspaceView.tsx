import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { UtilityRegistry } from "../registry";
import type { ToolDefinition, ToolResult } from "../contracts";

const mediaIds = new Set(["image-icons", "gif-create", "gif-extract", "tiff-pages", "image-metadata"]);

export const MediaWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [delay, setDelay] = useState(100);
  const [loop, setLoop] = useState(true);
  const [iconPreset, setIconPreset] = useState("macos");
  const [report, setReport] = useState<unknown[]>([]);
  const activeUtility = mediaIds.has(utility.id)
    ? utility
    : UtilityRegistry.find((item) => item.id === "icon-set") ?? utility;

  return (
    <ToolScaffold
      utility={activeUtility}
      onRun={(paths) => {
        if (activeUtility.id === "icon-set") {
          return invoke<ToolResult>("generate_icon_set", {
            request: { paths: [paths[0]], preset: iconPreset, sizes: [], outputLocation: "alongsideInput" },
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
      {({ files, run, loading }) => (
        <div className="workspace-control-panel">
          <div>
            <p className="workspace-panel-label">Specialized media outcome</p>
            <p className="workspace-panel-copy">Keep the same selected files while choosing a sequence, icon, or metadata result.</p>
          </div>
          {activeUtility.id === "icon-set" && <label className="workspace-field"><span>Icon preset</span><select aria-label="Icon preset" value={iconPreset} onChange={(event) => setIconPreset(event.target.value)}><option value="macos">macOS</option><option value="favicon">Favicon</option><option value="ios">iOS</option><option value="android">Android</option></select></label>}
          {activeUtility.id === "gif-create" && <div className="workspace-field-grid"><label className="workspace-field"><span>Frame delay</span><input aria-label="Frame delay in milliseconds" type="number" min="1" max="60000" value={delay} onChange={(event) => setDelay(Number(event.target.value))} /></label><label className="workspace-check"><input aria-label="Loop animation" type="checkbox" checked={loop} onChange={(event) => setLoop(event.target.checked)} /> Loop animation</label></div>}
          {activeUtility.id === "image-metadata" && <button type="button" className="workspace-secondary-action" disabled={loading || files.length === 0} onClick={async () => setReport(await invoke<unknown[]>("inspect_image_metadata", { request: { paths: files, outputLocation: "alongsideInput" } }))}>Inspect metadata</button>}
          {report.length > 0 && activeUtility.id === "image-metadata" && <pre className="workspace-report">{JSON.stringify(report, null, 2)}</pre>}
          <button type="button" disabled={loading || files.length === 0} onClick={run} className="workspace-primary-action">{activeUtility.shortTitle}</button>
        </div>
      )}
    </ToolScaffold>
  );
};
