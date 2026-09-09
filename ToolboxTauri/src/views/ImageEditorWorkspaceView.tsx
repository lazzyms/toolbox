import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { UtilityRegistry } from "../registry";
import type {
  CompressImagesRequest,
  ConvertImagesRequest,
  ImagePreview,
  ToolDefinition,
  ToolResult,
} from "../contracts";

const imageEditorIds = new Set([
  "heic-convert",
  "compress",
  "resize",
  "rotate",
  "crop",
  "image-watermark",
  "image-tone",
]);

export const ImageEditorWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [format, setFormat] = useState<ConvertImagesRequest["format"]>("png");
  const [quality, setQuality] = useState(80);
  const [lossless, setLossless] = useState(false);
  const [width, setWidth] = useState(1024);
  const [height, setHeight] = useState(768);
  const [resizeMode, setResizeMode] = useState("exact");
  const [percentage, setPercentage] = useState(100);
  const [resampling, setResampling] = useState("lanczos");
  const [keepRatio, setKeepRatio] = useState(true);
  const [degrees, setDegrees] = useState(90);
  const [flip, setFlip] = useState("none");
  const [cropMode, setCropMode] = useState("rectangle");
  const [cropX, setCropX] = useState(0);
  const [cropY, setCropY] = useState(0);
  const [anchor, setAnchor] = useState("center");
  const [brightness, setBrightness] = useState(0);
  const [contrast, setContrast] = useState(0);
  const [saturation, setSaturation] = useState(0);
  const [exposure, setExposure] = useState(0);
  const [watermarkText, setWatermarkText] = useState("Toolbox");
  const [watermarkOpacity, setWatermarkOpacity] = useState(20);
  const activeUtility = imageEditorIds.has(utility.id)
    ? utility
    : UtilityRegistry.find((item) => item.id === "heic-convert") ?? utility;

  return (
    <ToolScaffold
      utility={activeUtility}
      onRun={(paths) => {
        if (activeUtility.id === "heic-convert") {
          return invoke<ToolResult>("convert_images", {
            request: { paths, format, outputLocation: "alongsideInput" } satisfies ConvertImagesRequest,
          });
        }
        if (activeUtility.id === "compress") {
          return invoke<ToolResult>("compress_images", {
            request: { paths, quality, lossless, outputLocation: "alongsideInput" } satisfies CompressImagesRequest,
          });
        }
        if (activeUtility.id === "resize") {
          return invoke<ToolResult>("resize_images", {
            request: {
              paths,
              width,
              height,
              mode: resizeMode,
              percentage,
              longestSide: width,
              resampling,
              keepAspectRatio: keepRatio,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "rotate") {
          return invoke<ToolResult>("rotate_images", {
            request: { paths, degrees, flip, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "crop") {
          return invoke<ToolResult>("crop_images", {
            request: {
              paths,
              x: cropX,
              y: cropY,
              width,
              height,
              mode: cropMode,
              aspectWidth: width,
              aspectHeight: height,
              anchor,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "image-watermark") {
          return invoke<ToolResult>("watermark_images", {
            request: {
              paths,
              opacity: watermarkOpacity,
              text: watermarkText,
              x: cropX,
              y: cropY,
              outputLocation: "alongsideInput",
            },
          });
        }
        return invoke<ToolResult>("adjust_image_tone", {
          request: {
            paths,
            brightness,
            contrast,
            saturation,
            exposure,
            outputLocation: "alongsideInput",
          },
        });
      }}
    >
      {(props) => <ImageEditorControls {...props} utility={activeUtility} {...{
        format,
        setFormat,
        quality,
        setQuality,
        lossless,
        setLossless,
        width,
        setWidth,
        height,
        setHeight,
        resizeMode,
        setResizeMode,
        percentage,
        setPercentage,
        resampling,
        setResampling,
        keepRatio,
        setKeepRatio,
        degrees,
        setDegrees,
        flip,
        setFlip,
        cropMode,
        setCropMode,
        cropX,
        setCropX,
        cropY,
        setCropY,
        anchor,
        setAnchor,
        brightness,
        setBrightness,
        contrast,
        setContrast,
        saturation,
        setSaturation,
        exposure,
        setExposure,
        watermarkText,
        setWatermarkText,
        watermarkOpacity,
        setWatermarkOpacity,
      }} />}
    </ToolScaffold>
  );
};

type ImageEditorControlsProps = {
  files: string[];
  run: () => Promise<void>;
  loading: boolean;
  utility: ToolDefinition;
  format: ConvertImagesRequest["format"];
  setFormat: (value: ConvertImagesRequest["format"]) => void;
  quality: number;
  setQuality: (value: number) => void;
  lossless: boolean;
  setLossless: (value: boolean) => void;
  width: number;
  setWidth: (value: number) => void;
  height: number;
  setHeight: (value: number) => void;
  resizeMode: string;
  setResizeMode: (value: string) => void;
  percentage: number;
  setPercentage: (value: number) => void;
  resampling: string;
  setResampling: (value: string) => void;
  keepRatio: boolean;
  setKeepRatio: (value: boolean) => void;
  degrees: number;
  setDegrees: (value: number) => void;
  flip: string;
  setFlip: (value: string) => void;
  cropMode: string;
  setCropMode: (value: string) => void;
  cropX: number;
  setCropX: (value: number) => void;
  cropY: number;
  setCropY: (value: number) => void;
  anchor: string;
  setAnchor: (value: string) => void;
  brightness: number;
  setBrightness: (value: number) => void;
  contrast: number;
  setContrast: (value: number) => void;
  saturation: number;
  setSaturation: (value: number) => void;
  exposure: number;
  setExposure: (value: number) => void;
  watermarkText: string;
  setWatermarkText: (value: string) => void;
  watermarkOpacity: number;
  setWatermarkOpacity: (value: number) => void;
};

const ImageEditorControls = ({
  files,
  run,
  loading,
  utility,
  format,
  setFormat,
  quality,
  setQuality,
  lossless,
  setLossless,
  width,
  setWidth,
  height,
  setHeight,
  resizeMode,
  setResizeMode,
  percentage,
  setPercentage,
  resampling,
  setResampling,
  keepRatio,
  setKeepRatio,
  degrees,
  setDegrees,
  flip,
  setFlip,
  cropMode,
  setCropMode,
  cropX,
  setCropX,
  cropY,
  setCropY,
  anchor,
  setAnchor,
  brightness,
  setBrightness,
  contrast,
  setContrast,
  saturation,
  setSaturation,
  exposure,
  setExposure,
  watermarkText,
  setWatermarkText,
  watermarkOpacity,
  setWatermarkOpacity,
}: ImageEditorControlsProps) => {
  const [preview, setPreview] = useState<ImagePreview | null>(null);
  const [previewError, setPreviewError] = useState("");
  const inputPath = files[0];

  useEffect(() => {
    if (!inputPath) {
      setPreview(null);
      setPreviewError("");
      return;
    }
    let current = true;
    setPreviewError("");
    void invoke<ImagePreview>("inspect_image_preview", { request: { path: inputPath } })
      .then((value) => {
        if (current && value && typeof value.dataUrl === "string") setPreview(value);
      })
      .catch((error) => {
        if (current) setPreviewError(String(error));
      });
    return () => {
      current = false;
    };
  }, [inputPath]);

  return (
    <div className="image-editor-layout">
      <section className="image-editor-preview" aria-label="Image preview">
        {preview ? (
          <img src={preview.dataUrl} alt={`Preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`} />
        ) : (
          <div className="image-editor-empty">
            <span className="image-editor-empty-icon" aria-hidden="true">✦</span>
            <strong>{inputPath ? "Preview unavailable" : "Choose an image to begin"}</strong>
            <span>{inputPath ? "The editor can still process this file." : "Your original stays on this device."}</span>
          </div>
        )}
        {preview && <span className="image-editor-dimensions">{preview.width} × {preview.height}px</span>}
        {previewError && <span className="image-editor-preview-error" role="status">{previewError}</span>}
      </section>
      <section className="image-editor-controls" aria-label="Image adjustments">
        <div className="workspace-panel-intro">
          <p className="workspace-panel-label">{utility.title}</p>
          <p className="workspace-panel-copy">Change the outcome without leaving the image editor.</p>
        </div>
        {utility.id === "heic-convert" && (
          <label className="workspace-field">
            <span>Target format</span>
            <select aria-label="Target format" value={format} onChange={(event) => setFormat(event.target.value as ConvertImagesRequest["format"])}>
              <option value="png">PNG</option>
              <option value="jpg">JPEG</option>
              <option value="webp">WebP</option>
              <option value="heic">HEIC</option>
            </select>
          </label>
        )}
        {utility.id === "compress" && (
          <>
            <label className="workspace-field">
              <span>Quality <output>{quality}%</output></span>
              <input aria-label="Compression quality" type="range" min="1" max="100" value={quality} onChange={(event) => setQuality(Number(event.target.value))} />
            </label>
            <label className="workspace-check"><input aria-label="Lossless compression" type="checkbox" checked={lossless} onChange={(event) => setLossless(event.target.checked)} /> Preserve original pixels</label>
          </>
        )}
        {utility.id === "resize" && (
          <>
            <label className="workspace-field"><span>Resize mode</span><select aria-label="Resize mode" value={resizeMode} onChange={(event) => setResizeMode(event.target.value)}><option value="exact">Exact size</option><option value="percentage">Percentage</option><option value="longestSide">Longest side</option></select></label>
            {resizeMode === "percentage" ? <label className="workspace-field"><span>Scale <output>{percentage}%</output></span><input aria-label="Resize percentage" type="number" min="1" max="1000" value={percentage} onChange={(event) => setPercentage(Number(event.target.value))} /></label> : <div className="workspace-field-grid"><label className="workspace-field"><span>Width</span><input aria-label="Width" type="number" min="1" value={width} onChange={(event) => setWidth(Number(event.target.value))} /></label><label className="workspace-field"><span>{resizeMode === "longestSide" ? "Longest side" : "Height"}</span>{resizeMode === "longestSide" ? <input aria-label="Longest side" type="number" min="1" value={width} onChange={(event) => setWidth(Number(event.target.value))} /> : <input aria-label="Height" type="number" min="1" value={height} onChange={(event) => setHeight(Number(event.target.value))} />}</label></div>}
            <label className="workspace-check"><input type="checkbox" checked={keepRatio} onChange={(event) => setKeepRatio(event.target.checked)} /> Preserve aspect ratio</label>
            <label className="workspace-field"><span>Resampling</span><select aria-label="Resampling" value={resampling} onChange={(event) => setResampling(event.target.value)}><option value="lanczos">Lanczos — best quality</option><option value="bicubic">Bicubic — balanced</option><option value="nearest">Nearest — sharp edges</option></select></label>
          </>
        )}
        {utility.id === "rotate" && <div className="workspace-field-grid"><label className="workspace-field"><span>Rotation</span><select aria-label="Rotation" value={degrees} onChange={(event) => setDegrees(Number(event.target.value))}><option value="0">0°</option><option value="90">90°</option><option value="180">180°</option><option value="270">270°</option></select></label><label className="workspace-field"><span>Flip</span><select aria-label="Mirror" value={flip} onChange={(event) => setFlip(event.target.value)}><option value="none">None</option><option value="horizontal">Horizontal</option><option value="vertical">Vertical</option></select></label></div>}
        {utility.id === "crop" && <><div className="workspace-field-grid"><label className="workspace-field"><span>Crop mode</span><select aria-label="Crop mode" value={cropMode} onChange={(event) => setCropMode(event.target.value)}><option value="rectangle">Rectangle</option><option value="aspectRatio">Aspect ratio</option></select></label>{cropMode === "aspectRatio" ? <label className="workspace-field"><span>Anchor</span><select aria-label="Crop anchor" value={anchor} onChange={(event) => setAnchor(event.target.value)}><option value="center">Center</option><option value="top">Top</option><option value="bottom">Bottom</option><option value="left">Left</option><option value="right">Right</option></select></label> : <span />}</div><div className="workspace-field-grid"><label className="workspace-field"><span>Width</span><input aria-label="Crop width" type="number" min="1" value={width} onChange={(event) => setWidth(Number(event.target.value))} /></label><label className="workspace-field"><span>Height</span><input aria-label="Crop height" type="number" min="1" value={height} onChange={(event) => setHeight(Number(event.target.value))} /></label><label className="workspace-field"><span>Left</span><input aria-label="Crop left" type="number" min="0" value={cropX} onChange={(event) => setCropX(Number(event.target.value))} /></label><label className="workspace-field"><span>Top</span><input aria-label="Crop top" type="number" min="0" value={cropY} onChange={(event) => setCropY(Number(event.target.value))} /></label></div></>}
        {utility.id === "image-watermark" && <><label className="workspace-field"><span>Watermark text</span><input aria-label="Watermark text" value={watermarkText} onChange={(event) => setWatermarkText(event.target.value)} /></label><label className="workspace-field"><span>Opacity <output>{watermarkOpacity}%</output></span><input aria-label="Watermark opacity" type="range" min="1" max="100" value={watermarkOpacity} onChange={(event) => setWatermarkOpacity(Number(event.target.value))} /></label><div className="workspace-field-grid"><label className="workspace-field"><span>Left</span><input aria-label="Watermark left" type="number" min="0" value={cropX} onChange={(event) => setCropX(Number(event.target.value))} /></label><label className="workspace-field"><span>Top</span><input aria-label="Watermark top" type="number" min="0" value={cropY} onChange={(event) => setCropY(Number(event.target.value))} /></label></div></>}
        {utility.id === "image-tone" && <><ToneControl label="Brightness" value={brightness} onChange={setBrightness} /><ToneControl label="Contrast" value={contrast} onChange={setContrast} /><ToneControl label="Saturation" value={saturation} onChange={setSaturation} /><ToneControl label="Exposure" value={exposure} onChange={setExposure} /></>}
        <button type="button" disabled={loading || files.length === 0} onClick={run} className="workspace-primary-action">{utility.id === "compress" ? "Compress Images" : utility.shortTitle}</button>
      </section>
    </div>
  );
};

const ToneControl = ({ label, value, onChange }: { label: string; value: number; onChange: (value: number) => void }) => (
  <label className="workspace-field">
    <span>{label} <output>{value}</output></span>
    <input aria-label={label} type="range" min="-100" max="100" value={value} onChange={(event) => onChange(Number(event.target.value))} />
  </label>
);
