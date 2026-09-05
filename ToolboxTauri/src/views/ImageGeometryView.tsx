import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageGeometryView = ({
  utility,
  mode,
}: {
  utility: ToolDefinition;
  mode: "resize" | "rotate" | "crop";
}) => {
  const [width, setWidth] = useState(mode === "crop" ? 256 : 1024);
  const [height, setHeight] = useState(mode === "crop" ? 256 : 768);
  const [degrees, setDegrees] = useState(90);
  const [cropMode, setCropMode] = useState("rectangle");
  const [anchor, setAnchor] = useState("center");
  const [flip, setFlip] = useState("none");
  const [resizeMode, setResizeMode] = useState("exact");
  const [percentage, setPercentage] = useState(100);
  const [units, setUnits] = useState("Pixels");
  const [resampling, setResampling] = useState("lanczos");
  const [keepRatio, setKeepRatio] = useState(true);
  return (
    <ToolScaffold
      utility={utility}
      onRun={(paths) => {
        if (mode === "resize")
          return invoke<ToolResult>("resize_images", {
            request: {
              paths,
              width,
              height,
              mode: units === "Percent" ? "percentage" : resizeMode,
              percentage,
              longestSide: width,
              resampling,
              keepAspectRatio: keepRatio,
              outputLocation: "alongsideInput",
            },
          });
        if (mode === "rotate")
          return invoke<ToolResult>("rotate_images", {
            request: { paths, degrees, flip, outputLocation: "alongsideInput" },
          });
        return invoke<ToolResult>("crop_images", {
          request: {
            paths,
            x: 0,
            y: 0,
            width,
            height,
            mode: cropMode,
            aspectWidth: width,
            aspectHeight: height,
            anchor,
            outputLocation: "alongsideInput",
          },
        });
      }}
    >
      {({ files, run, loading }) => (
        <div className="space-y-4">
          {mode === "resize" && (
            <div className="flex flex-wrap gap-2">
              <select
                aria-label="Resize mode"
                value={resizeMode}
                onChange={(event) => setResizeMode(event.target.value)}
              >
                <option value="exact">Exact size</option>
                <option value="percentage">Percentage</option>
                <option value="longestSide">Longest side</option>
              </select>
              <select
                aria-label="Resize units"
                value={units}
                onChange={(event) => setUnits(event.target.value)}
              >
                <option>Pixels</option>
                <option>Percent</option>
              </select>
            </div>
          )}
          {mode === "resize" &&
            (units === "Pixels" && resizeMode !== "percentage" ? (
              <div className="flex flex-wrap gap-2">
                <input
                  aria-label="Width"
                  type="number"
                  min="1"
                  value={width}
                  onChange={(event) => setWidth(Number(event.target.value))}
                />
                <input
                  aria-label="Height"
                  type="number"
                  min="1"
                  value={height}
                  onChange={(event) => setHeight(Number(event.target.value))}
                />
              </div>
            ) : (
              <input
                aria-label="Resize percentage"
                type="number"
                min="1"
                max="1000"
                value={percentage}
                onChange={(event) => setPercentage(Number(event.target.value))}
              />
            ))}
          {mode === "resize" && (
            <div className="flex flex-wrap gap-3">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={keepRatio}
                  onChange={(event) => setKeepRatio(event.target.checked)}
                />
                Preserve aspect ratio
              </label>
              <label className="text-sm">
                Resampling
                <select
                  aria-label="Resampling"
                  value={resampling}
                  onChange={(event) => setResampling(event.target.value)}
                >
                  <option value="lanczos">Lanczos — best quality</option>
                  <option value="bicubic">Bicubic — balanced</option>
                  <option value="nearest">Nearest — sharp edges</option>
                </select>
              </label>
            </div>
          )}
          {mode === "crop" && (
            <div className="flex flex-wrap gap-2">
              <select
                aria-label="Crop mode"
                value={cropMode}
                onChange={(event) => setCropMode(event.target.value)}
              >
                <option value="rectangle">Rectangle</option>
                <option value="aspectRatio">Aspect ratio</option>
              </select>
              {cropMode === "aspectRatio" && (
                <select
                  aria-label="Crop anchor"
                  value={anchor}
                  onChange={(event) => setAnchor(event.target.value)}
                >
                  <option value="center">Center</option>
                  <option value="top">Top</option>
                  <option value="bottom">Bottom</option>
                  <option value="left">Left</option>
                  <option value="right">Right</option>
                </select>
              )}
            </div>
          )}
          {mode === "rotate" && (
            <div className="flex flex-wrap gap-2">
              <select
                aria-label="Rotation"
                value={degrees}
                onChange={(event) => setDegrees(Number(event.target.value))}
              >
                <option value="0">0°</option>
                <option value="90">90°</option>
                <option value="180">180°</option>
                <option value="270">270°</option>
              </select>
              <select
                aria-label="Mirror"
                value={flip}
                onChange={(event) => setFlip(event.target.value)}
              >
                <option value="none">No mirror</option>
                <option value="horizontal">Mirror horizontally</option>
                <option value="vertical">Mirror vertically</option>
              </select>
            </div>
          )}
          <button
            type="button"
            disabled={loading || files.length === 0}
            onClick={run}
            className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50"
          >
            {utility.shortTitle}
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};
