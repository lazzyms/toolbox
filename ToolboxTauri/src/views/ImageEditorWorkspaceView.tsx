import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { WorkspaceSourceAction } from "../components/ToolScaffold";
import type { ShellEvent } from "../hooks/useShellBridge";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import { useToolAvailability } from "../ToolAvailabilityContext";
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
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { NativeSelect } from "../components/ui/native-select";
import { Slider } from "../components/ui/slider";
import { Switch } from "../components/ui/switch";

const imageEditorActions = toolsForWorkspaceId("image-editor");
const imageEditorIds = new Set<string>(imageEditorActions.map((tool) => tool.id));
const cropAspectPresets = [
  { value: "1:1", label: "Square · 1:1", width: 1, height: 1 },
  { value: "4:3", label: "Classic · 4:3", width: 4, height: 3 },
  { value: "3:2", label: "Photo · 3:2", width: 3, height: 2 },
  { value: "16:9", label: "Wide · 16:9", width: 16, height: 9 },
  { value: "9:16", label: "Portrait · 9:16", width: 9, height: 16 },
] as const;
const aspectRatioMatches = (firstWidth: number, firstHeight: number, secondWidth: number, secondHeight: number) =>
  firstWidth > 0 && firstHeight > 0 && Math.abs(firstWidth / firstHeight - secondWidth / secondHeight) < 0.001;

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
  cropWidth: number;
  cropHeight: number;
  cropX: number;
  cropY: number;
  anchor: string;
  brightness: number;
  contrast: number;
  saturation: number;
  exposure: number;
  watermarkText: string;
  watermarkOpacity: number;
  watermarkX: number;
  watermarkY: number;
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
    width: values.cropWidth,
    height: values.cropHeight,
    mode: values.cropMode,
    aspectWidth: values.cropWidth,
    aspectHeight: values.cropHeight,
    anchor: values.anchor,
  };
  if (id === "image-watermark") return {
    kind: "watermark",
    text: values.watermarkText,
    opacity: values.watermarkOpacity,
    x: values.watermarkX,
    y: values.watermarkY,
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

export const ImageEditorWorkspaceView = ({
  utility,
  fileActivation,
  onActivationAccepted,
  onFilesChange,
  onWorkspaceSourceAction,
}: {
  utility: ToolDefinition;
  fileActivation?: Extract<ShellEvent, { kind: "files" }>;
  onActivationAccepted?: (activationId: string) => void;
  onFilesChange?: (paths: readonly string[]) => void;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
}) => {
  const availability = useToolAvailability();
  const availableActions = availability.filter(imageEditorActions);
  const preferredTool = imageEditorIds.has(utility.id) && availability.allows(utility.id)
    ? utility.id
    : availableActions[0]?.id ?? utility.id;
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
  const [cropMode, setCropMode] = useState("aspectRatio");
  const [cropWidth, setCropWidth] = useState(4);
  const [cropHeight, setCropHeight] = useState(3);
  const [cropX, setCropX] = useState(0);
  const [cropY, setCropY] = useState(0);
  const [anchor, setAnchor] = useState("center");
  const [brightness, setBrightness] = useState(0);
  const [contrast, setContrast] = useState(0);
  const [saturation, setSaturation] = useState(0);
  const [exposure, setExposure] = useState(0);
  const [watermarkText, setWatermarkText] = useState("Toolbox");
  const [watermarkOpacity, setWatermarkOpacity] = useState(20);
  const [watermarkX, setWatermarkX] = useState(0);
  const [watermarkY, setWatermarkY] = useState(0);
  const [history, setHistory] = useState<ImageEditHistory>(() => createImageEditHistory());
  const [committedDraftKey, setCommittedDraftKey] = useState<string | null>(null);
  const [activeToolId, setActiveToolId] = useState<AtomicToolId>(
    preferredTool,
  );
  useEffect(() => {
    setActiveToolId(preferredTool);
  }, [preferredTool]);
  const activeUtility = availability.allows(activeToolId)
    ? UtilityRegistry.find((item) => item.id === activeToolId) ?? utility
    : UtilityRegistry.find((item) => item.id === preferredTool) ?? utility;
  const draft = buildImageEditDraft(activeUtility.id, {
    format, quality, lossless, width, height, resizeMode, percentage, resampling, keepRatio,
    degrees, flip, cropMode, cropWidth, cropHeight, cropX, cropY, anchor, brightness, contrast, saturation, exposure,
    watermarkText, watermarkOpacity, watermarkX, watermarkY,
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
      fileActivation={fileActivation}
      onActivationAccepted={onActivationAccepted}
      onFilesChange={onFilesChange}
      onWorkspaceSourceAction={onWorkspaceSourceAction}
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
            request: { paths, x: cropX, y: cropY, width: cropWidth, height: cropHeight, mode: cropMode, aspectWidth: cropWidth, aspectHeight: cropHeight, anchor, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "image-watermark") {
          return invoke<ToolResult>("watermark_images", {
            request: { paths, opacity: watermarkOpacity, text: watermarkText, x: watermarkX, y: watermarkY, outputLocation: "alongsideInput" },
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
      {(props) => <ImageEditorControls {...props} actions={availableActions} utility={activeUtility} activeToolId={activeToolId} onSelectTool={(id) => { if (availability.allows(id)) setActiveToolId(id); }} {...{
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
        cropWidth,
        setCropWidth,
        cropHeight,
        setCropHeight,
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
        watermarkX,
        setWatermarkX,
        watermarkY,
        setWatermarkY,
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
  actions: readonly ToolDefinition[];
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
  cropWidth: number;
  setCropWidth: (value: number) => void;
  cropHeight: number;
  setCropHeight: (value: number) => void;
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
  watermarkX: number;
  setWatermarkX: (value: number) => void;
  watermarkY: number;
  setWatermarkY: (value: number) => void;
  plan: ImageEditPlan;
  draft: ImageEditOperation | null;
  canUndo: boolean;
  canRedo: boolean;
  onAddEdit: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onReset: () => void;
};

type ImagePreviewValidation =
  | { key: string; status: "pending" }
  | { key: string; status: "valid" }
  | { key: string; status: "error"; message: string };

const ImageEditorControls = ({
  actions,
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
  cropWidth,
  setCropWidth,
  cropHeight,
  setCropHeight,
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
  watermarkX,
  setWatermarkX,
  watermarkY,
  setWatermarkY,
  plan,
  draft,
  canUndo,
  canRedo,
  onAddEdit,
  onUndo,
  onRedo,
  onReset,
}: ImageEditorControlsProps) => {
  const availability = useToolAvailability();
  const [sourcePreview, setSourcePreview] = useState<ImagePreview | null>(null);
  const [resultPreview, setResultPreview] = useState<ImagePreview | null>(null);
  const [sourcePreviewError, setSourcePreviewError] = useState("");
  const [previewValidation, setPreviewValidation] = useState<ImagePreviewValidation | null>(null);
  const inputPath = files[0];
  const previewPlanKey = JSON.stringify({ plan, draft });
  const previousInputPath = useRef<string | undefined>(undefined);
  const previewStageRef = useRef<HTMLDivElement>(null);
  const dragOffset = useRef<{ x: number; y: number } | null>(null);
  const activePointerId = useRef<number | null>(null);

  const clearImageDrag = () => {
    const pointerId = activePointerId.current;
    activePointerId.current = null;
    dragOffset.current = null;
    const stage = previewStageRef.current;
    if (pointerId !== null && stage?.hasPointerCapture(pointerId)) stage.releasePointerCapture(pointerId);
  };

  useEffect(() => {
    const clear = () => clearImageDrag();
    window.addEventListener("pointerup", clear);
    window.addEventListener("pointercancel", clear);
    window.addEventListener("blur", clear);
    return () => {
      window.removeEventListener("pointerup", clear);
      window.removeEventListener("pointercancel", clear);
      window.removeEventListener("blur", clear);
      clearImageDrag();
    };
  }, []);

  useEffect(() => {
    if (previousInputPath.current !== undefined && previousInputPath.current !== inputPath) onReset();
    previousInputPath.current = inputPath;
  }, [inputPath, onReset]);

  useEffect(() => {
    if (!inputPath) {
      setSourcePreview(null);
      setResultPreview(null);
      setSourcePreviewError("");
      setPreviewValidation(null);
      return;
    }
    let current = true;
    setSourcePreview(null);
    setSourcePreviewError("");
    void invoke<ImagePreview>("inspect_image_preview", { request: { path: inputPath } })
      .then((value) => {
        if (current && value && typeof value.dataUrl === "string") setSourcePreview(value);
      })
      .catch((error) => {
        if (current) setSourcePreviewError(String(error));
      });
    return () => {
      current = false;
    };
  }, [inputPath]);

  useEffect(() => {
    const nextPlan = planWithDraft(plan, draft);
    if (!inputPath || !nextPlan.edits.length) {
      setResultPreview(null);
      setPreviewValidation(null);
      return;
    }
    let current = true;
    setResultPreview(null);
    setPreviewValidation({ key: previewPlanKey, status: "pending" });
    void invoke<ImagePreview>("inspect_image_edit_preview", {
      request: { path: inputPath, plan: nextPlan },
    })
      .then((value) => {
        if (!current) return;
        if (!value || typeof value.dataUrl !== "string") {
          setPreviewValidation({ key: previewPlanKey, status: "error", message: "The edit preview was invalid." });
          return;
        }
        setResultPreview(value);
        setPreviewValidation({ key: previewPlanKey, status: "valid" });
      })
      .catch((error) => {
        if (current) setPreviewValidation({ key: previewPlanKey, status: "error", message: String(error) });
      });
    return () => {
      current = false;
    };
  }, [inputPath, previewPlanKey]);

  const previewError = sourcePreviewError || (previewValidation?.status === "error" ? previewValidation.message : "");
  const hasEdits = plan.edits.length > 0 || draft !== null;
  const previewIsValid = previewValidation?.key === previewPlanKey && previewValidation.status === "valid";
  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === "undo" && canUndo) onUndo();
      else if (command === "redo" && canRedo) onRedo();
      else if (command === "export" && files.length > 0 && hasEdits && previewIsValid && !loading) void runCombined();
    };
    window.addEventListener("toolbox:editor-command", onShellCommand);
    return () => window.removeEventListener("toolbox:editor-command", onShellCommand);
  }, [canRedo, canUndo, files.length, hasEdits, loading, onRedo, onUndo, previewIsValid, runCombined]);
  const selectedCropPreset = cropMode !== "aspectRatio"
    ? "free"
    : sourcePreview && aspectRatioMatches(cropWidth, cropHeight, sourcePreview.width, sourcePreview.height)
      ? "original"
      : cropAspectPresets.find((preset) => aspectRatioMatches(cropWidth, cropHeight, preset.width, preset.height))?.value ?? "custom";
  const cropAspectOptions = [
    { value: "free", label: "Free crop" },
    { value: "original", label: "Original", disabled: !sourcePreview },
    ...cropAspectPresets.map(({ value, label }) => ({ value, label })),
    { value: "custom", label: "Custom", disabled: true },
  ];
  const selectCropPreset = (presetValue: string) => {
    if (presetValue === "free") {
      setCropMode("rectangle");
      setCropX(0);
      setCropY(0);
      setCropWidth(sourcePreview?.width ?? cropWidth);
      setCropHeight(sourcePreview?.height ?? cropHeight);
      return;
    }
    if (presetValue === "original" && sourcePreview) {
      setCropMode("aspectRatio");
      setCropX(0);
      setCropY(0);
      setCropWidth(sourcePreview.width);
      setCropHeight(sourcePreview.height);
      setAnchor("center");
      return;
    }
    const preset = cropAspectPresets.find(({ value }) => value === presetValue);
    if (preset) {
      setCropMode("aspectRatio");
      setCropX(0);
      setCropY(0);
      setCropWidth(preset.width);
      setCropHeight(preset.height);
      setAnchor("center");
    }
  };

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
    if (!interactionPreview || (utility.id !== "crop" && utility.id !== "image-watermark")) return;
    const point = pointInPreview(event);
    if (!point) return;
    dragOffset.current = utility.id === "crop"
      ? { x: point.x - (cropRect?.x ?? cropX), y: point.y - (cropRect?.y ?? cropY) }
      : { x: 0, y: 0 };
    activePointerId.current = event.pointerId;
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  const moveImageDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
    if (activePointerId.current !== event.pointerId || !dragOffset.current || !interactionPreview) return;
    const point = pointInPreview(event);
    if (!point) return;
    if (utility.id === "crop") {
      const currentCropWidth = cropRect?.width ?? cropWidth;
      const currentCropHeight = cropRect?.height ?? cropHeight;
      setCropX(Math.round(Math.min(Math.max(point.x - dragOffset.current.x, 0), Math.max(interactionPreview.width - currentCropWidth, 0))));
      setCropY(Math.round(Math.min(Math.max(point.y - dragOffset.current.y, 0), Math.max(interactionPreview.height - currentCropHeight, 0))));
      if (cropMode === "aspectRatio") setAnchor("custom");
    } else {
      setWatermarkX(Math.round(point.x));
      setWatermarkY(Math.round(point.y));
    }
  };
  const endImageDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (activePointerId.current === event.pointerId) clearImageDrag();
  };
  const interactionPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
  const cropRect = utility.id === "crop" && sourcePreview
    ? resolveImageCropRect(sourcePreview.width, sourcePreview.height, {
      kind: "crop", x: cropX, y: cropY, width: cropWidth, height: cropHeight, mode: cropMode,
      aspectWidth: cropWidth, aspectHeight: cropHeight, anchor,
    })
    : null;
  const cropOverlay = cropRect && sourcePreview ? { left: `${cropRect.x / sourcePreview.width * 100}%`, top: `${cropRect.y / sourcePreview.height * 100}%`, width: `${cropRect.width / sourcePreview.width * 100}%`, height: `${cropRect.height / sourcePreview.height * 100}%` } : null;
  const watermarkOverlay = utility.id === "image-watermark" && interactionPreview ? { left: `${Math.min(watermarkX / interactionPreview.width * 100, 94)}%`, top: `${Math.min(watermarkY / interactionPreview.height * 100, 94)}%` } : null;
  const primaryPreview = utility.id === "crop" ? sourcePreview : resultPreview ?? sourcePreview;
  const primaryPreviewAlt = utility.id === "crop" ? `Original image preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}` : `Preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`;
  const selectTool = (nextToolId: AtomicToolId) => {
    if (nextToolId === activeToolId || !availability.allows(nextToolId)) return;
    if (draft) onAddEdit();
    onSelectTool(nextToolId);
  };

  return (
    <div className="image-editor-layout">
      <WorkspaceCommandRail
        actions={actions}
        activeId={activeToolId}
        onSelect={selectTool}
        label="Image editor tools"
      />
      <Card className="image-editor-preview" role="region" aria-label="Image preview">
        <CardContent className="image-editor-preview-content grid min-h-[340px] place-items-center overflow-hidden p-0">
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
            {cropOverlay && <div className="image-editor-crop-overlay" aria-label="Crop selection" style={cropOverlay}><div className="image-editor-crop-grid" aria-hidden="true"><span /><span /><span /><span /></div></div>}
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
        {utility.id === "crop" && resultPreview && <Card className="image-editor-result-preview py-0" role="region" aria-label="Crop result preview">
          <CardContent className="image-editor-result-preview-content grid gap-2 p-2">
          <p className="workspace-panel-label">Resulting crop</p>
          <img src={resultPreview.dataUrl} alt={`Resulting crop preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`} />
          <span>{resultPreview.width} × {resultPreview.height}px</span>
          </CardContent>
        </Card>}
        </CardContent>
      </Card>
      <Card className="image-editor-controls py-0" role="region" aria-label="Image adjustments">
        <CardContent className="image-editor-controls-content grid gap-4 p-4">
        <div className="workspace-panel-intro">
          <h2 className="workspace-active-command">{utility.title}</h2>
          <p className="workspace-panel-copy">Change the outcome without leaving the image editor.</p>
        </div>
        {utility.id === "heic-convert" && (
          <div className="workspace-field">
            <Label htmlFor="image-target-format">Target format</Label>
            <NativeSelect id="image-target-format" aria-label="Target format" value={format} onChange={(event) => setFormat(event.target.value as ConvertImagesRequest["format"])}>
              <option value="png">PNG</option><option value="jpg">JPEG</option><option value="webp">WebP</option><option value="heic">HEIC</option>
            </NativeSelect>
          </div>
        )}
        {utility.id === "compress" && (
          <>
            <div className="workspace-field">
              <div className="workspace-field-heading"><Label id="image-quality-label">Quality</Label><output>{quality}%</output></div>
              <Slider aria-labelledby="image-quality-label" min={1} max={100} value={[quality]} onValueChange={([value]) => value !== undefined && setQuality(value)} />
            </div>
            <div className="workspace-check"><Label htmlFor="image-lossless">Preserve original pixels</Label><Switch id="image-lossless" checked={lossless} onCheckedChange={setLossless} /></div>
          </>
        )}
        {utility.id === "resize" && (
          <>
            <div className="workspace-field"><Label htmlFor="image-resize-mode">Resize mode</Label><NativeSelect id="image-resize-mode" aria-label="Resize mode" value={resizeMode} onChange={(event) => setResizeMode(event.target.value)}>
              <option value="exact">Exact size</option><option value="percentage">Percentage</option><option value="longestSide">Longest side</option>
            </NativeSelect></div>
            {resizeMode === "percentage" ? (
              <div className="workspace-field"><Label htmlFor="image-resize-percentage">Scale</Label><Input id="image-resize-percentage" aria-label="Resize percentage" type="number" min="1" max="1000" value={percentage} onChange={(event) => setPercentage(Number(event.target.value))} /></div>
            ) : (
              <div className="workspace-field-grid">
                <div className="workspace-field"><Label htmlFor="image-resize-width">Width</Label><Input id="image-resize-width" aria-label="Width" type="number" min="1" value={width} onChange={(event) => setWidth(Number(event.target.value))} /></div>
                <div className="workspace-field"><Label htmlFor="image-resize-height">{resizeMode === "longestSide" ? "Longest side" : "Height"}</Label><Input id="image-resize-height" aria-label={resizeMode === "longestSide" ? "Longest side" : "Height"} type="number" min="1" value={resizeMode === "longestSide" ? width : height} onChange={(event) => { if (resizeMode === "longestSide") setWidth(Number(event.target.value)); else setHeight(Number(event.target.value)); }} /></div>
              </div>
            )}
            <div className="workspace-check"><Label htmlFor="image-keep-ratio">Preserve aspect ratio</Label><Switch id="image-keep-ratio" checked={keepRatio} onCheckedChange={setKeepRatio} /></div>
            <div className="workspace-field"><Label htmlFor="image-resampling">Resampling</Label><NativeSelect id="image-resampling" aria-label="Resampling" value={resampling} onChange={(event) => setResampling(event.target.value)}>
              <option value="lanczos">Lanczos — best quality</option><option value="bicubic">Bicubic — balanced</option><option value="nearest">Nearest — sharp edges</option>
            </NativeSelect></div>
          </>
        )}
        {utility.id === "rotate" && (
          <div className="workspace-field-grid">
            <div className="workspace-field"><Label htmlFor="image-rotation">Rotation</Label><NativeSelect id="image-rotation" aria-label="Rotation" value={degrees} onChange={(event) => setDegrees(Number(event.target.value))}>{[0, 90, 180, 270].map((value) => <option key={value} value={value}>{value}°</option>)}</NativeSelect></div>
            <div className="workspace-field"><Label htmlFor="image-flip">Flip</Label><NativeSelect id="image-flip" aria-label="Mirror" value={flip} onChange={(event) => setFlip(event.target.value)}><option value="none">None</option><option value="horizontal">Horizontal</option><option value="vertical">Vertical</option></NativeSelect></div>
          </div>
        )}
        {utility.id === "crop" && (
          <>
            <div className="workspace-field"><Label htmlFor="image-crop-aspect">Aspect ratio</Label><NativeSelect id="image-crop-aspect" aria-label="Aspect ratio" value={selectedCropPreset} onChange={(event) => selectCropPreset(event.target.value)}>{cropAspectOptions.map((option) => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}</NativeSelect></div>
            {cropMode === "aspectRatio" ? (
              <>
                <div className="workspace-field-grid">
                  <div className="workspace-field"><Label htmlFor="image-crop-aspect-width">Aspect width</Label><Input id="image-crop-aspect-width" aria-label="Aspect width" type="number" min="1" value={cropWidth} onChange={(event) => setCropWidth(Number(event.target.value))} /></div>
                  <div className="workspace-field"><Label htmlFor="image-crop-aspect-height">Aspect height</Label><Input id="image-crop-aspect-height" aria-label="Aspect height" type="number" min="1" value={cropHeight} onChange={(event) => setCropHeight(Number(event.target.value))} /></div>
                </div>
                <div className="workspace-field"><Label htmlFor="image-crop-anchor">Crop anchor</Label><NativeSelect id="image-crop-anchor" aria-label="Crop anchor" value={anchor} onChange={(event) => setAnchor(event.target.value)}>{["center", "top", "bottom", "left", "right", ...(anchor === "custom" ? ["custom"] : [])].map((value) => <option key={value} value={value}>{value === "custom" ? "Custom position" : value[0].toUpperCase() + value.slice(1)}</option>)}</NativeSelect></div>
              </>
            ) : (
              <div className="workspace-field-grid">
                <div className="workspace-field"><Label htmlFor="image-crop-width">Crop width</Label><Input id="image-crop-width" aria-label="Crop width" type="number" min="1" max={sourcePreview?.width} value={cropWidth} onChange={(event) => setCropWidth(Number(event.target.value))} /></div>
                <div className="workspace-field"><Label htmlFor="image-crop-height">Crop height</Label><Input id="image-crop-height" aria-label="Crop height" type="number" min="1" max={sourcePreview?.height} value={cropHeight} onChange={(event) => setCropHeight(Number(event.target.value))} /></div>
                <div className="workspace-field"><Label htmlFor="image-crop-left">Crop left</Label><Input id="image-crop-left" aria-label="Crop left" type="number" min="0" value={cropX} onChange={(event) => setCropX(Number(event.target.value))} /></div>
                <div className="workspace-field"><Label htmlFor="image-crop-top">Crop top</Label><Input id="image-crop-top" aria-label="Crop top" type="number" min="0" value={cropY} onChange={(event) => setCropY(Number(event.target.value))} /></div>
              </div>
            )}
          </>
        )}
        {utility.id === "image-watermark" && (
          <>
            <div className="workspace-field"><Label htmlFor="image-watermark-text">Watermark text</Label><Input id="image-watermark-text" aria-label="Watermark text" value={watermarkText} onChange={(event) => setWatermarkText(event.target.value)} /></div>
            <div className="workspace-field"><div className="workspace-field-heading"><Label id="image-watermark-opacity-label">Opacity</Label><output>{watermarkOpacity}%</output></div><Slider aria-labelledby="image-watermark-opacity-label" min={1} max={100} value={[watermarkOpacity]} onValueChange={([value]) => value !== undefined && setWatermarkOpacity(value)} /></div>
            <div className="workspace-field-grid">
              <div className="workspace-field"><Label htmlFor="image-watermark-left">Watermark left</Label><Input id="image-watermark-left" aria-label="Watermark left" type="number" min="0" value={watermarkX} onChange={(event) => setWatermarkX(Number(event.target.value))} /></div>
              <div className="workspace-field"><Label htmlFor="image-watermark-top">Watermark top</Label><Input id="image-watermark-top" aria-label="Watermark top" type="number" min="0" value={watermarkY} onChange={(event) => setWatermarkY(Number(event.target.value))} /></div>
            </div>
          </>
        )}
        {utility.id === "image-tone" && <>
          {([["Brightness", brightness, setBrightness], ["Contrast", contrast, setContrast], ["Saturation", saturation, setSaturation], ["Exposure", exposure, setExposure]] as const).map(([label, value, onChange]) => <div className="workspace-field" key={label}><div className="workspace-field-heading"><Label id={`image-tone-${label.toLowerCase()}-label`}>{label}</Label><output>{value}</output></div><Slider aria-labelledby={`image-tone-${label.toLowerCase()}-label`} min={-100} max={100} value={[value]} onValueChange={([next]) => next !== undefined && onChange(next)} /></div>)}
        </>}
        <Card role="region" className="image-editor-history py-0" aria-label="Image edit history">
          <CardContent className="image-editor-history-content py-4">
          <div className="workspace-panel-intro">
            <p className="workspace-panel-label">Edit stack</p>
            <p className="workspace-panel-copy">Preview changes together and export them once.</p>
          </div>
          <p className="workspace-panel-copy">Edits are applied in order. Undo removes one edit at a time.</p>
          <p aria-live="polite">{plan.edits.length} committed edit{plan.edits.length === 1 ? "" : "s"}{draft ? " plus current draft" : ""}</p>
          {plan.edits.length > 0 && (
            <ol className="image-edit-timeline">
              {plan.edits.map((edit, index) => <li key={`${edit.kind}-${index}`}><span>{index + 1}</span>{imageEditLabel(edit)}</li>)}
            </ol>
          )}
          <div className="workspace-button-row">
            <Button variant="ghost" size="sm" aria-label="Undo" onClick={onUndo} disabled={!canUndo}>Undo</Button>
            <Button variant="ghost" size="sm" aria-label="Redo" onClick={onRedo} disabled={!canRedo}>Redo</Button>
            <Button variant="destructive" size="sm" aria-label="Reset edits" onClick={onReset} disabled={!canUndo && !canRedo && plan.edits.length === 0 && !draft}>Reset edits</Button>
          </div>
          <Button variant="default" size="sm" aria-label="Add edit to plan" onClick={onAddEdit} disabled={!draft}>Add edit to plan</Button>
          </CardContent>
        </Card>
        <p aria-live="polite" className="workspace-note">{plan.edits.length + (draft ? 1 : 0)} edits will be applied to each selected image.</p>
        <Button variant="default" aria-label="Export edited images" disabled={loading || files.length === 0 || !hasEdits || !previewIsValid} onClick={runCombined} className="workspace-primary-action">Export edited images</Button>
        </CardContent>
      </Card>
    </div>
  );
};
