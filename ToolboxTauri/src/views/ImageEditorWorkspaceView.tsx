import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type {
  AtomicToolId,
  CompressImagesRequest,
  ConvertImagesRequest,
  ImagePreview,
  ToolDefinition,
  ToolResult,
} from "../contracts";
import {
  commitImageEdit,
  createImageEditHistory,
  planWithDraft,
  redoImageEdit,
  resolveImageCropRect,
  resetImageEdits,
  undoImageEdit,
  type ImageEditOperation,
  type ImageEditPlan,
  type ImageEditHistory,
} from "../features/image-editor/session";

const imageEditorActions = toolsForWorkspaceId("image-editor");
const imageEditorIds = new Set<string>(imageEditorActions.map((tool) => tool.id));

type ImageEditorDraftValues = {
  format: ConvertImagesRequest["format"];
  quality: number;
  lossless: boolean;
  width: number;
  height: number;
  resizeMode: string;
  percentage: number;
  resampling: string;
  keepRatio: boolean;
  degrees: number;
  flip: string;
  cropMode: string;
  cropX: number;
  cropY: number;
  anchor: string;
  brightness: number;
  contrast: number;
  saturation: number;
  exposure: number;
  watermarkText: string;
  watermarkOpacity: number;
};

const buildImageEditDraft = (id: string, values: ImageEditorDraftValues): ImageEditOperation => {
  if (id === "heic-convert") return { kind: "convert", format: values.format };
  if (id === "compress") return { kind: "compress", quality: values.quality, lossless: values.lossless };
  if (id === "resize") return {
    kind: "resize",
    width: values.width,
    height: values.height,
    mode: values.resizeMode,
    percentage: values.percentage,
    longestSide: values.width,
    resampling: values.resampling,
    keepAspectRatio: values.keepRatio,
  };
  if (id === "rotate") return { kind: "rotate", degrees: values.degrees, flip: values.flip };
  if (id === "crop") return {
    kind: "crop",
    x: values.cropX,
    y: values.cropY,
    width: values.width,
    height: values.height,
    mode: values.cropMode,
    aspectWidth: values.width,
    aspectHeight: values.height,
    anchor: values.anchor,
  };
  if (id === "image-watermark") return {
    kind: "watermark",
    text: values.watermarkText,
    opacity: values.watermarkOpacity,
    x: values.cropX,
    y: values.cropY,
  };
  return {
    kind: "tone",
    brightness: values.brightness,
    contrast: values.contrast,
    saturation: values.saturation,
    exposure: values.exposure,
  };
};

const imageEditLabel = (edit: ImageEditOperation) => {
  if (edit.kind === "resize") return `Resize ${edit.width} × ${edit.height}`;
  if (edit.kind === "rotate") return `Rotate ${edit.degrees}°${edit.flip !== "none" ? ` · ${edit.flip}` : ""}`;
  if (edit.kind === "tone") return `Tone · ${edit.brightness}/${edit.contrast}/${edit.saturation}/${edit.exposure}`;
  if (edit.kind === "watermark") return `Watermark · ${edit.text || "Text"}`;
  if (edit.kind === "compress") return `Compress · ${edit.lossless ? "lossless" : `${edit.quality}%`}`;
  if (edit.kind === "convert") return `Convert · ${edit.format.toUpperCase()}`;
  return `Crop ${edit.width} × ${edit.height}`;
};

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
  const [degrees, setDegrees] = useState(0);
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
  const [history, setHistory] = useState<ImageEditHistory>(() => createImageEditHistory());
  const [committedDraftKey, setCommittedDraftKey] = useState<string | null>(null);
  const [activeToolId, setActiveToolId] = useState<AtomicToolId>(
    imageEditorIds.has(utility.id) ? utility.id : "heic-convert",
  );
  useEffect(() => {
    setActiveToolId(imageEditorIds.has(utility.id) ? utility.id : "heic-convert");
  }, [utility.id]);
  const activeUtility = UtilityRegistry.find((item) => item.id === activeToolId) ?? utility;
  const draft = buildImageEditDraft(activeUtility.id, {
    format, quality, lossless, width, height, resizeMode, percentage, resampling, keepRatio,
    degrees, flip, cropMode, cropX, cropY, anchor, brightness, contrast, saturation, exposure,
    watermarkText, watermarkOpacity,
  });
  const draftKey = JSON.stringify(draft);
  const liveDraft = draftKey === committedDraftKey ? null : draft;

  useEffect(() => {
    setCommittedDraftKey(draftKey);
  }, [activeUtility.id]);

  const editPlan = planWithDraft(history.present, liveDraft);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="image-editor"
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
            request: { paths, width, height, mode: resizeMode, percentage, longestSide: width, resampling, keepAspectRatio: keepRatio, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "rotate") {
          return invoke<ToolResult>("rotate_images", { request: { paths, degrees, flip, outputLocation: "alongsideInput" } });
        }
        if (activeUtility.id === "crop") {
          return invoke<ToolResult>("crop_images", {
            request: { paths, x: cropX, y: cropY, width, height, mode: cropMode, aspectWidth: width, aspectHeight: height, anchor, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "image-watermark") {
          return invoke<ToolResult>("watermark_images", {
            request: { paths, opacity: watermarkOpacity, text: watermarkText, x: cropX, y: cropY, outputLocation: "alongsideInput" },
          });
        }
        return invoke<ToolResult>("adjust_image_tone", {
          request: { paths, brightness, contrast, saturation, exposure, outputLocation: "alongsideInput" },
        });
      }}
      onRunCombined={(paths) => invoke<ToolResult>("export_image_edit_plan", {
        request: { paths, plan: editPlan },
      })}
    >
      {(props) => <ImageEditorControls {...props} utility={activeUtility} activeToolId={activeToolId} onSelectTool={setActiveToolId} {...{
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
        plan: history.present,
        draft: liveDraft,
        canUndo: history.past.length > 0,
        canRedo: history.future.length > 0,
        onAddEdit: () => {
          if (!liveDraft) return;
          setHistory((current) => commitImageEdit(current, liveDraft));
          setCommittedDraftKey(draftKey);
        },
        onUndo: () => {
          setHistory((current) => undoImageEdit(current));
          setCommittedDraftKey(draftKey);
        },
        onRedo: () => {
          setHistory((current) => redoImageEdit(current));
          setCommittedDraftKey(draftKey);
        },
        onReset: () => {
          setHistory(resetImageEdits());
          setCommittedDraftKey(draftKey);
        },
      }} />}
    </ToolScaffold>
  );
};

type ImageEditorControlsProps = {
  files: string[];
  runCombined: () => Promise<void>;
  loading: boolean;
  utility: ToolDefinition;
  activeToolId: AtomicToolId;
  onSelectTool: (id: AtomicToolId) => void;
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
  plan: ImageEditPlan;
  draft: ImageEditOperation | null;
  canUndo: boolean;
  canRedo: boolean;
  onAddEdit: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onReset: () => void;
};

const ImageEditorControls = ({
  files,
  runCombined,
  loading,
  utility,
  activeToolId,
  onSelectTool,
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
  plan,
  draft,
  canUndo,
  canRedo,
  onAddEdit,
  onUndo,
  onRedo,
  onReset,
}: ImageEditorControlsProps) => {
  const [sourcePreview, setSourcePreview] = useState<ImagePreview | null>(null);
  const [resultPreview, setResultPreview] = useState<ImagePreview | null>(null);
  const [previewError, setPreviewError] = useState("");
  const inputPath = files[0];
  const previewPlanKey = JSON.stringify({ plan, draft });
  const previousInputPath = useRef<string | undefined>(undefined);
  const previewStageRef = useRef<HTMLDivElement>(null);
  const dragOffset = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    if (previousInputPath.current !== undefined && previousInputPath.current !== inputPath) onReset();
    previousInputPath.current = inputPath;
  }, [inputPath, onReset]);

  useEffect(() => {
    if (!inputPath) {
      setSourcePreview(null);
      setResultPreview(null);
      setPreviewError("");
      return;
    }
    let current = true;
    setSourcePreview(null);
    setPreviewError("");
    void invoke<ImagePreview>("inspect_image_preview", { request: { path: inputPath } })
      .then((value) => {
        if (current && value && typeof value.dataUrl === "string") setSourcePreview(value);
      })
      .catch((error) => {
        if (current) setPreviewError(String(error));
      });
    return () => {
      current = false;
    };
  }, [inputPath]);

  useEffect(() => {
    if (!inputPath || !planWithDraft(plan, draft).edits.length) {
      setResultPreview(null);
      return;
    }
    let current = true;
    setPreviewError("");
    void invoke<ImagePreview>("inspect_image_edit_preview", {
      request: { path: inputPath, plan: planWithDraft(plan, draft) },
    })
      .then((value) => {
        if (current && value && typeof value.dataUrl === "string") setResultPreview(value);
      })
      .catch((error) => {
        if (current) setPreviewError(String(error));
      });
    return () => {
      current = false;
    };
  }, [inputPath, previewPlanKey]);

  const pointInPreview = (event: React.PointerEvent<HTMLDivElement>) => {
    const bounds = previewStageRef.current?.getBoundingClientRect();
    const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
    if (!bounds || !interactionPreview) return null;
    return {
      x: Math.min(Math.max((event.clientX - bounds.left) / bounds.width * interactionPreview.width, 0), interactionPreview.width),
      y: Math.min(Math.max((event.clientY - bounds.top) / bounds.height * interactionPreview.height, 0), interactionPreview.height),
    };
  };
  const beginImageDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
    if (!interactionPreview || (utility.id === "crop" && cropMode !== "rectangle") || (utility.id !== "crop" && utility.id !== "image-watermark")) return;
    const point = pointInPreview(event);
    if (!point) return;
    dragOffset.current = utility.id === "crop"
      ? { x: point.x - cropX, y: point.y - cropY }
      : { x: 0, y: 0 };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  const moveImageDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
    if (!dragOffset.current || !interactionPreview) return;
    const point = pointInPreview(event);
    if (!point) return;
    if (utility.id === "crop") {
      setCropX(Math.round(Math.min(Math.max(point.x - dragOffset.current.x, 0), Math.max(interactionPreview.width - width, 0))));
      setCropY(Math.round(Math.min(Math.max(point.y - dragOffset.current.y, 0), Math.max(interactionPreview.height - height, 0))));
    } else {
      setCropX(Math.round(point.x));
      setCropY(Math.round(point.y));
    }
  };
  const endImageDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    dragOffset.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };
  const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
  const cropRect = utility.id === "crop" && sourcePreview
    ? resolveImageCropRect(sourcePreview.width, sourcePreview.height, {
      kind: "crop", x: cropX, y: cropY, width, height, mode: cropMode,
      aspectWidth: width, aspectHeight: height, anchor,
    })
    : null;
  const cropOverlay = cropRect && sourcePreview ? { left: `${cropRect.x / sourcePreview.width * 100}%`, top: `${cropRect.y / sourcePreview.height * 100}%`, width: `${cropRect.width / sourcePreview.width * 100}%`, height: `${cropRect.height / sourcePreview.height * 100}%` } : null;
  const watermarkOverlay = utility.id === "image-watermark" && interactionPreview ? { left: `${Math.min(cropX / interactionPreview.width * 100, 94)}%`, top: `${Math.min(cropY / interactionPreview.height * 100, 94)}%` } : null;
  const primaryPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
  const primaryPreviewAlt = utility.id === "crop" ? `Original image preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}` : `Preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`;
  const selectTool = (nextToolId: AtomicToolId) => {
    if (nextToolId === activeToolId) return;
    if (draft) onAddEdit();
    onSelectTool(nextToolId);
  };

  return (
    <div className="image-editor-layout">
      <WorkspaceCommandRail
        actions={imageEditorActions}
        activeId={activeToolId}
        onSelect={selectTool}
        label="Image editor tools"
      />
      <section className="image-editor-preview" aria-label="Image preview">
        {primaryPreview ? (
          <div
            ref={previewStageRef}
            className={`image-editor-preview-stage ${utility.id === "crop" || utility.id === "image-watermark" ? "image-editor-preview-stage-interactive" : ""}`}
            style={{ aspectRatio: `${primaryPreview.width} / ${primaryPreview.height}` }}
            onPointerDown={beginImageDrag}
            onPointerMove={moveImageDrag}
            onPointerUp={endImageDrag}
            onPointerCancel={endImageDrag}
          >
            <img src={primaryPreview.dataUrl} alt={primaryPreviewAlt} />
            {cropOverlay && <div className="image-editor-crop-overlay" aria-label="Crop selection" style={cropOverlay} />}
            {watermarkOverlay && <div className="image-editor-watermark-overlay" aria-label="Watermark preview" style={watermarkOverlay}>{watermarkText || "Watermark"}</div>}
          </div>
        ) : (
          <div className="image-editor-empty">
            <span className="image-editor-empty-icon" aria-hidden="true">✦</span>
            <strong>{inputPath ? "Preview unavailable" : "Choose an image to begin"}</strong>
            <span>{inputPath ? "The editor can still process this file." : "Your original stays on this device."}</span>
          </div>
        )}
        {primaryPreview && <span className="image-editor-dimensions">{primaryPreview.width} × {primaryPreview.height}px</span>}
        {previewError && <span className="image-editor-preview-error" role="status">{previewError}</span>}
        {utility.id === "crop" && resultPreview && <section className="image-editor-result-preview" aria-label="Crop result preview">
          <p className="workspace-panel-label">Resulting crop</p>
          <img src={resultPreview.dataUrl} alt={`Resulting crop preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`} />
          <span>{resultPreview.width} × {resultPreview.height}px</span>
        </section>}
      </section>
      <section className="image-editor-controls" aria-label="Image adjustments">
        <div className="workspace-panel-intro">
          <h2 className="workspace-active-command">{utility.title}</h2>
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
        <section className="image-editor-history" aria-label="Image edit history">
          <div className="workspace-panel-intro">
            <p className="workspace-panel-label">Edit stack</p>
            <p className="workspace-panel-copy">Preview changes together and export them once.</p>
          </div>
          <p aria-live="polite">{plan.edits.length} committed edit{plan.edits.length === 1 ? "" : "s"}{draft ? " plus current draft" : ""}</p>
          {plan.edits.length > 0 && (
            <ol className="image-edit-timeline">
              {plan.edits.map((edit, index) => <li key={`${edit.kind}-${index}`}><span>{index + 1}</span>{imageEditLabel(edit)}</li>)}
            </ol>
          )}
          <div className="workspace-button-row">
            <button type="button" aria-label="Undo" onClick={onUndo} disabled={!canUndo}>Undo</button>
            <button type="button" aria-label="Redo" onClick={onRedo} disabled={!canRedo}>Redo</button>
            <button type="button" aria-label="Reset edits" onClick={onReset} disabled={!canUndo && !canRedo && plan.edits.length === 0 && !draft}>Reset edits</button>
          </div>
          <button type="button" aria-label="Add edit to plan" onClick={onAddEdit} disabled={!draft}>Add edit to plan</button>
        </section>
        <p aria-live="polite" className="workspace-note">{plan.edits.length + (draft ? 1 : 0)} edits will be applied to each selected image.</p>
        <button type="button" aria-label="Export edited images" disabled={loading || files.length === 0 || (!plan.edits.length && !draft)} onClick={runCombined} className="workspace-primary-action">Export edited images</button>
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
