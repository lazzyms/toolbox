import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronLeft, ChevronRight, CircleAlert, ImageIcon, Info, Link2, Redo2, Undo2, X } from "lucide-react";
import type { ImagePreview, OutputLocation } from "../contracts";
import {
  imageEditRequestPlan,
  defaultImageToneStep,
  imageEditStackPreviewPlan,
  imageEditStackSteps,
  imageEditStepChipLabels,
  imageEditStepLabels,
  imageFlips,
  imageResizeStepProblem,
  imageRotateDegrees,
  imageToneStepProblem,
  imageToneValues,
  imageWatermarkStepProblem,
  isNeutralImageToneStep,
  presentImageEditSteps,
  resolveImageCropRect,
  type EditHistory,
  type ImageCropStep,
  type ImageEditStack,
  type ImageEditStackFormat,
  type ImageEditStackStep,
  type ImageResizeStep,
  type ImageRotateStep,
  type ImageToneStep,
  type ImageWatermarkStep,
} from "../features/image-editor/session";
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { NativeSelect } from "../components/ui/native-select";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Slider } from "../components/ui/slider";
import { Toggle } from "../components/ui/toggle";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { isWindows, shortcutTarget } from "../shortcuts";

export type StackMessage = { text: string; undo: boolean };

const stackFormats = [
  { value: "original", label: "Keep original format" },
  { value: "jpg", label: "JPEG" },
  { value: "png", label: "PNG" },
  { value: "webp", label: "WebP" },
  { value: "heic", label: "HEIC" },
] as const satisfies ReadonlyArray<{ value: ImageEditStackFormat; label: string }>;

const cropRatios = [[1, 1], [4, 5], [4, 3], [3, 2], [16, 9]] as const;
const keepOptions = [
  { value: "center", label: "Centre" },
  { value: "start", label: "Top or left" },
  { value: "end", label: "Bottom or right" },
  { value: "custom", label: "Where I drag it" },
] as const;
const pixelFields = [
  { key: "x", label: "X", aria: "Left edge, pixels", min: 0 },
  { key: "y", label: "Y", aria: "Top edge, pixels", min: 0 },
  { key: "width", label: "Width", aria: "Width, pixels", min: 1 },
  { key: "height", label: "Height", aria: "Height, pixels", min: 1 },
] as const;

const resizeModes = [
  { value: "longestSide", label: "Longest side" },
  { value: "exact", label: "Exact size" },
  { value: "percentage", label: "Percentage" },
] as const;
const resamplingOptions = [
  { value: "lanczos", label: "Smooth, best for photos" },
  { value: "bicubic", label: "Sharp, keeps fine detail" },
  { value: "nearest", label: "Nearest, for pixel art" },
] as const;

const flipLabels: Record<(typeof imageFlips)[number], string> = { none: "None", horizontal: "Horizontal", vertical: "Vertical" };

const toneLabels: Record<(typeof imageToneValues)[number], string> = { brightness: "Brightness", contrast: "Contrast", saturation: "Saturation", exposure: "Exposure" };
// Shown as the board writes them: +10, −20 and 0.
const formatToneValue = (value: number) => value > 0 ? `+${value}` : value < 0 ? `−${Math.abs(value)}` : "0";
const parseToneValue = (text: string) => {
  const trimmed = text.trim().replace("−", "-");
  const value = Number(trimmed);
  return trimmed !== "" && trimmed !== "-" && trimmed !== "+" && Number.isFinite(value) ? value : null;
};

const watermarkAngles = [0, 45, 90] as const;
const watermarkPictureExtensions = ["png", "jpg", "jpeg", "webp", "tif", "tiff"];
const formatAngle = (degrees: number) => `${degrees < 0 ? "−" : ""}${Math.abs(degrees)}°`;
const anchorSteps = [0, 0.5, 1] as const;
const anchorName = (x: number, y: number) => [y < 1 / 3 ? "top" : y > 2 / 3 ? "bottom" : "", x < 1 / 3 ? "left" : x > 2 / 3 ? "right" : ""].filter(Boolean).join(" ") || "centre";
const capitalise = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);
const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);
const roundAnchor = (value: number) => Math.round(clamp(value, 0, 1) * 1000) / 1000;
// Native keeps the mark this far inside the image, as a share of the shorter side.
const WATERMARK_MARGIN = 0.03;
// The native message for text the font can't draw starts with this.
const unsupportedTextMessage = "The watermark font can't draw";

/** Where the mark sits on an image, in image pixels, by the same rule native places it. */
const watermarkFrame = (watermark: ImageWatermarkStep, aspect: number, imageWidth: number, imageHeight: number) => {
  const width = watermark.size / 100 * imageWidth;
  const height = width * aspect;
  const turn = watermark.rotation * Math.PI / 180;
  const boxWidth = Math.abs(width * Math.cos(turn)) + Math.abs(height * Math.sin(turn));
  const boxHeight = Math.abs(width * Math.sin(turn)) + Math.abs(height * Math.cos(turn));
  const margin = Math.round(WATERMARK_MARGIN * Math.min(imageWidth, imageHeight));
  return {
    width,
    height,
    boxWidth,
    boxHeight,
    margin,
    centreX: margin + watermark.anchorX * (imageWidth - boxWidth - 2 * margin) + boxWidth / 2,
    centreY: margin + watermark.anchorY * (imageHeight - boxHeight - 2 * margin) + boxHeight / 2,
  };
};

/** The anchor that puts the mark's centre at a point, or the middle when the mark leaves no room. */
const anchorForCentre = (centre: number, box: number, size: number, margin: number) => {
  const room = size - box - 2 * margin;
  return room > 0 ? roundAnchor((centre - box / 2 - margin) / room) : 0.5;
};

/** The text's size at 100px in the app font, so the stage can draw it at the width native gives it. */
const measureWatermarkText = (text: string) => {
  const fallback = { width: Math.max(text.length, 1) * 55, ascent: 75, descent: 20 };
  try {
    const context = document.createElement("canvas").getContext("2d");
    if (!context) return fallback;
    context.font = `600 100px ${getComputedStyle(document.body).fontFamily}`;
    const metrics = context.measureText(text);
    if (!(metrics.width > 0)) return fallback;
    const ascent = metrics.actualBoundingBoxAscent || 75;
    const descent = metrics.actualBoundingBoxDescent || 20;
    return { width: metrics.width, ascent, descent };
  } catch {
    return fallback;
  }
};

const fileName = (path: string | undefined) => path?.split(/[\\/]/).pop() ?? "selected image";

// Native rejects a crop whose size or ratio is not positive, so Run waits for a well-formed step.
const cropStepIsRunnable = (crop: ImageCropStep) => crop.mode === "aspectRatio"
  ? crop.aspectWidth > 0 && crop.aspectHeight > 0
  : crop.width > 0 && crop.height > 0 && crop.x >= 0 && crop.y >= 0;

/** The first thing that stops the stack from running, as the footer says it. */
const stackProblem = (stack: ImageEditStack) => {
  if (stack.crop && !cropStepIsRunnable(stack.crop)) return "Fix the crop to run";
  const resize = stack.resize && imageResizeStepProblem(stack.resize);
  if (resize) return `Fix the ${resize.field} to run`;
  const tone = stack.tone && imageToneStepProblem(stack.tone);
  if (tone) return `Fix the ${tone.field} to run`;
  const watermark = stack.watermark && imageWatermarkStepProblem(stack.watermark);
  if (!watermark) return null;
  return watermark.field === "picture" ? "Choose a picture to run" : `Fix the ${watermark.field} to run`;
};

const stepOrder = (steps: ImageEditStackStep[], step: ImageEditStackStep) => {
  const index = steps.indexOf(step);
  const where = steps.length === 1
    ? "The only step."
    : index === 0
      ? `Runs first, before ${steps.slice(1).map((other) => imageEditStepLabels[other]).join(" and ")}.`
      : `Runs after ${steps.slice(0, index).map((other) => imageEditStepLabels[other]).join(" and ")}.`;
  return `Step ${index + 1} of ${steps.length}. ${where}`;
};

/** The shared edit stack: one step per tool, applied to every image in one run, always in the same order. */
export const ImageEditStackPanel = ({
  tabs,
  files,
  outputLocation,
  selectedFileIndex,
  selectFile,
  destination,
  run,
  loading,
  history,
  message,
  format,
  onFormatChange,
  focus,
  onFocus,
  onAdd,
  onEditCrop,
  onEditResize,
  onEditRotate,
  onEditTone,
  onEditWatermark,
  onRemove,
  onUndo,
  onRedo,
  onReset,
}: {
  tabs: React.ReactNode;
  files: string[];
  outputLocation: OutputLocation;
  selectedFileIndex: number;
  selectFile: (index: number) => void;
  destination: React.ReactNode;
  run: () => Promise<void>;
  loading: boolean;
  history: EditHistory<ImageEditStack>;
  message: StackMessage | null;
  /** The output format for every image in the run; "original" keeps each input's own format. */
  format: ImageEditStackFormat;
  onFormatChange: (format: ImageEditStackFormat) => void;
  /** The step whose options are showing; it follows the chosen tool. */
  focus: ImageEditStackStep;
  onFocus: (step: ImageEditStackStep) => void;
  onAdd: (step: ImageEditStackStep) => void;
  /** Edits sharing a coalesce key replace one another in the history instead of each adding an undo step. */
  onEditCrop: (crop: ImageCropStep, coalesceKey?: string) => void;
  onEditResize: (resize: ImageResizeStep, coalesceKey?: string) => void;
  onEditRotate: (rotate: ImageRotateStep) => void;
  onEditTone: (tone: ImageToneStep, coalesceKey?: string) => void;
  onEditWatermark: (watermark: ImageWatermarkStep, coalesceKey?: string) => void;
  onRemove: (step: ImageEditStackStep) => void;
  onUndo: () => void;
  onRedo: () => void;
  onReset: () => void;
}) => {
  const stack = history.present;
  const crop = stack.crop;
  const resize = stack.resize;
  const rotate = stack.rotate;
  const tone = stack.tone;
  const watermark = stack.watermark;
  const steps = presentImageEditSteps(stack);
  const problem = stackProblem(stack);
  const canUndo = history.past.length > 0;
  const canRedo = history.future.length > 0;
  const inputPath = files[selectedFileIndex] ?? files[0];
  const [sourcePreview, setSourcePreview] = useState<ImagePreview | null>(null);
  const [resultPreview, setResultPreview] = useState<ImagePreview | null>(null);
  const [previewError, setPreviewError] = useState("");
  const [resultError, setResultError] = useState("");
  const previewStageRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointerId: number; offsetX: number; offsetY: number; key: string } | null>(null);
  const markDrag = useRef<{ pointerId: number; part: "move" | "resize" | "rotate"; offsetX: number; offsetY: number; key: string } | null>(null);
  const dragCount = useRef(0);
  const [logoPreview, setLogoPreview] = useState<ImagePreview | null>(null);
  const [pictureError, setPictureError] = useState("");
  const [customAngle, setCustomAngle] = useState(false);
  // What the user is typing in a tone field, kept until it leaves the field so "-" or "+" can start a value.
  const [toneText, setToneText] = useState<Partial<Record<(typeof imageToneValues)[number], string>>>({});
  // Only native knows which characters its font can draw, so the preview's answer stops the run as well.
  const unsupportedText = watermark?.mark === "text" && resultError.startsWith(unsupportedTextMessage) ? resultError : "";
  const canRun = files.length > 0 && steps.length > 0 && !problem && !unsupportedText && !loading;

  useEffect(() => {
    if (!inputPath) {
      setSourcePreview(null);
      setPreviewError("");
      return;
    }
    let current = true;
    setSourcePreview(null);
    setPreviewError("");
    void invoke<ImagePreview>("inspect_image_preview", { request: { path: inputPath } })
      .then((value) => { if (current && value && typeof value.dataUrl === "string") setSourcePreview(value); })
      .catch((error) => { if (current) setPreviewError(String(error)); });
    return () => { current = false; };
  }, [inputPath]);

  const stackKey = JSON.stringify(imageEditStackPreviewPlan(stack));
  useEffect(() => {
    setResultPreview(null);
    setResultError("");
    if (!inputPath || steps.length === 0 || problem) return;
    let current = true;
    void invoke<ImagePreview>("inspect_image_edit_preview", {
      request: { path: inputPath, plan: imageEditRequestPlan(imageEditStackPreviewPlan(stack), outputLocation) },
    })
      .then((value) => { if (current && value && typeof value.dataUrl === "string") setResultPreview(value); })
      .catch((error) => { if (current) setResultError(String(error)); });
    return () => { current = false; };
  }, [inputPath, stackKey]);

  const logoPath = watermark?.mark === "picture" ? watermark.logoPath : "";
  useEffect(() => {
    setLogoPreview(null);
    if (!logoPath) return;
    let current = true;
    void invoke<ImagePreview>("inspect_image_preview", { request: { path: logoPath } })
      .then((value) => { if (current && value && typeof value.dataUrl === "string") setLogoPreview(value); })
      .catch(() => { /* The run reports a picture it can't read; the stage then shows only the frame. */ });
    return () => { current = false; };
  }, [logoPath]);

  useEffect(() => {
    const onShellCommand = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === "undo" && canUndo) onUndo();
      else if (command === "redo" && canRedo) onRedo();
      else if (command === "export" && canRun) void run();
    };
    window.addEventListener("toolbox:editor-command", onShellCommand);
    return () => window.removeEventListener("toolbox:editor-command", onShellCommand);
  }, [canRedo, canRun, canUndo, onRedo, onUndo, run]);

  const cropRect = crop && sourcePreview ? resolveImageCropRect(sourcePreview.width, sourcePreview.height, crop) : null;
  const cropOverlay = cropRect && sourcePreview ? {
    left: `${cropRect.x / sourcePreview.width * 100}%`,
    top: `${cropRect.y / sourcePreview.height * 100}%`,
    width: `${cropRect.width / sourcePreview.width * 100}%`,
    height: `${cropRect.height / sourcePreview.height * 100}%`,
  } : null;

  const pointInPreview = (event: React.PointerEvent<HTMLDivElement>) => {
    const bounds = previewStageRef.current?.getBoundingClientRect();
    if (!bounds || !sourcePreview) return null;
    return {
      x: Math.min(Math.max((event.clientX - bounds.left) / bounds.width * sourcePreview.width, 0), sourcePreview.width),
      y: Math.min(Math.max((event.clientY - bounds.top) / bounds.height * sourcePreview.height, 0), sourcePreview.height),
    };
  };
  const beginDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const point = pointInPreview(event);
    if (!point || !cropRect) return;
    drag.current = { pointerId: event.pointerId, offsetX: point.x - cropRect.x, offsetY: point.y - cropRect.y, key: `drag-${++dragCount.current}` };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const active = drag.current;
    if (!active || active.pointerId !== event.pointerId || !crop || !cropRect || !sourcePreview) return;
    const point = pointInPreview(event);
    if (!point) return;
    const x = Math.round(Math.min(Math.max(point.x - active.offsetX, 0), Math.max(sourcePreview.width - cropRect.width, 0)));
    const y = Math.round(Math.min(Math.max(point.y - active.offsetY, 0), Math.max(sourcePreview.height - cropRect.height, 0)));
    if (x === cropRect.x && y === cropRect.y) return;
    onEditCrop(crop.mode === "aspectRatio" ? { ...crop, anchor: "custom", x, y } : { ...crop, x, y }, active.key);
  };
  const clearDrag = () => {
    const pointerId = drag.current?.pointerId ?? markDrag.current?.pointerId;
    drag.current = null;
    markDrag.current = null;
    const stage = previewStageRef.current;
    if (pointerId !== undefined && stage?.hasPointerCapture(pointerId)) stage.releasePointerCapture(pointerId);
  };
  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId === event.pointerId || markDrag.current?.pointerId === event.pointerId) clearDrag();
  };

  const textMetrics = useMemo(() => watermark?.mark === "text" && watermark.text ? measureWatermarkText(watermark.text) : null, [watermark?.mark, watermark?.text]);
  const markAspect = watermark?.mark === "text"
    ? textMetrics && (textMetrics.ascent + textMetrics.descent) / textMetrics.width
    : watermark?.logoPath ? (logoPreview ? logoPreview.height / Math.max(logoPreview.width, 1) : 1) : null;
  const showMark = Boolean(watermark && watermark.placement === "position" && sourcePreview && markAspect && focus !== "crop");
  const markFrame = showMark && watermark && sourcePreview && markAspect ? watermarkFrame(watermark, markAspect, sourcePreview.width, sourcePreview.height) : null;
  const markEditable = focus === "watermark" && markFrame !== null;
  const markLabel = watermark?.mark === "picture" ? fileName(watermark.logoPath) : watermark?.text ?? "";
  const markAria = watermark ? `Watermark, ${markLabel}, ${anchorName(watermark.anchorX, watermark.anchorY)}. Selected. Arrow keys move it, ${isWindows() ? "Alt" : "Option"}+arrow resizes it.` : "";

  const beginMarkDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const part = (event.target as HTMLElement).closest<HTMLElement>("[data-watermark-part]")?.dataset.watermarkPart;
    const point = pointInPreview(event);
    if (!point || !markFrame || (part !== "move" && part !== "resize" && part !== "rotate")) return;
    markDrag.current = { pointerId: event.pointerId, part, offsetX: point.x - markFrame.centreX, offsetY: point.y - markFrame.centreY, key: `drag-${++dragCount.current}` };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  const moveMarkDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const active = markDrag.current;
    if (!active || active.pointerId !== event.pointerId || !watermark || !markFrame || !sourcePreview) return;
    const point = pointInPreview(event);
    if (!point) return;
    let next = watermark;
    if (active.part === "move") {
      next = {
        ...watermark,
        anchorX: anchorForCentre(point.x - active.offsetX, markFrame.boxWidth, sourcePreview.width, markFrame.margin),
        anchorY: anchorForCentre(point.y - active.offsetY, markFrame.boxHeight, sourcePreview.height, markFrame.margin),
      };
    } else {
      const dx = point.x - markFrame.centreX;
      const dy = point.y - markFrame.centreY;
      if (active.part === "resize") {
        // The corner's distance along the mark's own width, both sides of the centre.
        const turn = watermark.rotation * Math.PI / 180;
        const along = Math.abs(dx * Math.cos(turn) + dy * Math.sin(turn));
        next = { ...watermark, size: Math.round(clamp(2 * along / sourcePreview.width * 100, 1, 100)) };
      } else {
        // The handle sits straight above the centre at 0°, and turning it clockwise turns the mark clockwise.
        let rotation = Math.round(Math.atan2(dy, dx) * 180 / Math.PI + 90);
        if (rotation > 180) rotation -= 360;
        next = { ...watermark, rotation };
      }
    }
    if (next.anchorX === watermark.anchorX && next.anchorY === watermark.anchorY && next.size === watermark.size && next.rotation === watermark.rotation) return;
    onEditWatermark(next, active.key);
  };
  const onMarkKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!watermark) return;
    const dx = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    const dy = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (!dx && !dy) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.altKey) {
      const size = clamp(watermark.size + (dx || -dy), 1, 100);
      if (size !== watermark.size) onEditWatermark({ ...watermark, size }, "watermark-key-size");
      return;
    }
    const step = event.shiftKey ? 0.1 : 0.02;
    const anchorX = roundAnchor(watermark.anchorX + dx * step);
    const anchorY = roundAnchor(watermark.anchorY + dy * step);
    if (anchorX !== watermark.anchorX || anchorY !== watermark.anchorY) onEditWatermark({ ...watermark, anchorX, anchorY }, "watermark-key-move");
  };
  const choosePicture = async () => {
    if (!watermark) return;
    try {
      const picked = await open({ multiple: false, filters: [{ name: "Picture", extensions: watermarkPictureExtensions }] });
      setPictureError("");
      if (typeof picked === "string" && picked) onEditWatermark({ ...watermark, logoPath: picked });
    } catch (error) {
      setPictureError(String(error));
    }
  };

  // A pointer released or cancelled outside the stage, or a lost window focus, still ends the drag.
  useEffect(() => {
    window.addEventListener("pointerup", clearDrag);
    window.addEventListener("pointercancel", clearDrag);
    window.addEventListener("blur", clearDrag);
    return () => {
      window.removeEventListener("pointerup", clearDrag);
      window.removeEventListener("pointercancel", clearDrag);
      window.removeEventListener("blur", clearDrag);
      clearDrag();
    };
  }, []);

  const selectCropMode = (mode: string) => {
    if (!crop || !mode || mode === crop.mode) return;
    if (mode === "rectangle" && crop.width <= 0 && cropRect) {
      // Start the pixel rectangle from the area the ratio keeps in the previewed image.
      onEditCrop({ ...crop, mode, ...cropRect });
      return;
    }
    onEditCrop({ ...crop, mode });
  };
  const ratioValue = crop ? `${crop.aspectWidth}:${crop.aspectHeight}` : "";
  const undoKey = isWindows() ? "Ctrl+Z" : "⌘Z";
  const redoKey = isWindows() ? "Ctrl+Y" : "⇧⌘Z";
  const footHint = files.length === 0 ? "Add images to run" : steps.length === 0 ? "Add a step to run" : problem ?? (unsupportedText ? "Fix the text to run" : "Originals stay as they are");
  const resizeProblem = resize ? imageResizeStepProblem(resize) : null;
  const toneProblem = tone ? imageToneStepProblem(tone) : null;
  const watermarkProblem = watermark ? imageWatermarkStepProblem(watermark) : null;
  const watermarkFieldProblem = watermarkProblem && watermarkProblem.field !== "text" && watermarkProblem.field !== "picture" ? watermarkProblem : null;
  const anchorValue = watermark ? anchorSteps.flatMap((y) => anchorSteps.map((x) => `${x},${y}`)).find((value) => value === `${watermark.anchorX},${watermark.anchorY}`) ?? "" : "";
  const angleValue = watermark ? (customAngle ? "custom" : String(watermark.rotation)) : "";
  const focused = stack[focus] !== undefined;
  const cropAria = crop?.mode === "rectangle"
    ? `Crop area, ${crop.width} by ${crop.height} pixels, from ${crop.x}, ${crop.y}. Drag to move it.`
    : crop ? `Crop area, ${ratioValue}, keeping ${keepOptions.find((option) => option.value === crop.anchor)?.label.toLowerCase() ?? "the centre"}. Drag to choose the part to keep.` : "";

  return (
    <div className="image-edit-stack">
      <div className="image-edit-stack-toolbar" role="toolbar" aria-label="Edit stack">
        <div className="image-edit-stack-steps" role="list" aria-label="Steps, in the order they run">
          {imageEditStackSteps.map((step, index) => {
            const separator = index > 0 && <span className="image-edit-stack-separator" aria-hidden="true">›</span>;
            return stack[step] ? (
              <span key={step} role="listitem" className="image-edit-stack-step">
                {separator}
                <Button
                  type="button"
                  size="sm"
                  variant={step === focus ? "secondary" : "ghost"}
                  aria-pressed={step === focus}
                  aria-label={`Step ${steps.indexOf(step) + 1} of ${steps.length}, ${imageEditStepLabels[step]}${step === focus ? ", showing its options" : ""}`}
                  onClick={() => onFocus(step)}
                >
                  <span aria-hidden="true">{steps.indexOf(step) + 1}</span>{imageEditStepChipLabels[step]}
                </Button>
                <Button type="button" size="icon-sm" variant="ghost" aria-label={`Remove ${imageEditStepLabels[step]} step`} onClick={() => onRemove(step)}><X aria-hidden="true" /></Button>
              </span>
            ) : (
              <span key={step} role="listitem" className="image-edit-stack-step">
                {separator}
                <Button type="button" size="sm" variant="outline" className="border-dashed" aria-label={`Add ${imageEditStepLabels[step]} step`} onClick={() => onAdd(step)}>+ {imageEditStepChipLabels[step]}</Button>
              </span>
            );
          })}
        </div>
        <Button type="button" size="icon-sm" variant="ghost" aria-label={`Undo, ${undoKey}`} title={`Undo ${undoKey}`} disabled={!canUndo} onClick={onUndo}><Undo2 aria-hidden="true" /></Button>
        <Button type="button" size="icon-sm" variant="ghost" aria-label={`Redo, ${redoKey}`} title={`Redo ${redoKey}`} disabled={!canRedo} onClick={onRedo}><Redo2 aria-hidden="true" /></Button>
        <Button type="button" size="sm" variant="ghost" disabled={steps.length === 0} onClick={onReset}>Reset stack</Button>
      </div>
      {message && (
        <div className="image-edit-stack-message" role="status" aria-live="polite">
          <span>{message.text}</span>
          {message.undo && <Button type="button" size="sm" variant="outline" onClick={onUndo} disabled={!canUndo}>Undo</Button>}
        </div>
      )}
      <div className="image-editor-layout">
        <div className="image-edit-stack-preview">
          {inputPath && (
            <div className="image-edit-stack-preview-header">
              <span>Previewing {fileName(inputPath)}</span>
              <span>{selectedFileIndex + 1} of {files.length}</span>
              <Button type="button" size="icon-sm" variant="ghost" aria-label="Previous image" disabled={selectedFileIndex <= 0} onClick={() => selectFile(selectedFileIndex - 1)}><ChevronLeft aria-hidden="true" /></Button>
              <Button type="button" size="icon-sm" variant="ghost" aria-label="Next image" disabled={selectedFileIndex >= files.length - 1} onClick={() => selectFile(selectedFileIndex + 1)}><ChevronRight aria-hidden="true" /></Button>
            </div>
          )}
        <Card className="image-editor-preview" role="region" aria-label="Image preview">
          <CardContent className="image-editor-preview-content grid min-h-[340px] place-items-center overflow-hidden p-0">
            {sourcePreview ? (
              <div
                ref={previewStageRef}
                className={`image-editor-preview-stage ${crop || markEditable ? "image-editor-preview-stage-interactive" : ""}`}
                style={{ aspectRatio: `${sourcePreview.width} / ${sourcePreview.height}` }}
                onPointerDown={markEditable ? beginMarkDrag : crop ? beginDrag : undefined}
                onPointerMove={markEditable ? moveMarkDrag : crop ? moveDrag : undefined}
                onPointerUp={endDrag}
                onPointerCancel={endDrag}
              >
                <img src={sourcePreview.dataUrl} alt={`Original image preview of ${fileName(inputPath)}`} />
                {cropOverlay && <div className="image-editor-crop-overlay" role="group" aria-label="Crop selection" aria-describedby="image-crop-area-description" style={cropOverlay}><div className="image-editor-crop-grid" aria-hidden="true"><span /><span /><span /><span /></div><span id="image-crop-area-description" className="sr-only">{cropAria}</span></div>}
                {watermark && markFrame && (
                  <div
                    className="image-editor-watermark-mark"
                    data-watermark-part="move"
                    data-selected={markEditable}
                    role={markEditable ? "group" : undefined}
                    aria-label={markEditable ? markAria : undefined}
                    aria-hidden={markEditable ? undefined : true}
                    tabIndex={markEditable ? 0 : undefined}
                    onKeyDown={markEditable ? onMarkKeyDown : undefined}
                    style={{
                      left: `${(markFrame.centreX - markFrame.width / 2) / sourcePreview.width * 100}%`,
                      top: `${(markFrame.centreY - markFrame.height / 2) / sourcePreview.height * 100}%`,
                      width: `${markFrame.width / sourcePreview.width * 100}%`,
                      height: `${markFrame.height / sourcePreview.height * 100}%`,
                      transform: `rotate(${watermark.rotation}deg)`,
                    }}
                  >
                    {watermark.mark === "text" && textMetrics ? (
                      <svg viewBox={`0 0 ${textMetrics.width} ${textMetrics.ascent + textMetrics.descent}`} preserveAspectRatio="none" aria-hidden="true">
                        <text x="0" y={textMetrics.ascent} fontSize="100" fontWeight="600" fill={/^#[0-9a-f]{6}$/i.test(watermark.color) ? watermark.color : "#FFFFFF"} fillOpacity={clamp(watermark.opacity, 0, 100) / 100}>{watermark.text}</text>
                      </svg>
                    ) : logoPreview ? (
                      <img src={logoPreview.dataUrl} alt="" style={{ opacity: clamp(watermark.opacity, 0, 100) / 100 }} />
                    ) : null}
                    {markEditable && (
                      <>
                        <span className="image-editor-watermark-stem" aria-hidden="true" />
                        <span className="image-editor-watermark-handle image-editor-watermark-rotate" data-watermark-part="rotate" aria-hidden="true" />
                        {[[0, 0], [100, 0], [100, 100], [0, 100]].map(([left, top]) => (
                          <span key={`${left}-${top}`} className="image-editor-watermark-handle" data-watermark-part="resize" aria-hidden="true" style={{ left: `${left}%`, top: `${top}%` }} />
                        ))}
                      </>
                    )}
                  </div>
                )}
              </div>
            ) : (
              <div className="image-editor-empty">
                <span className="image-editor-empty-icon" aria-hidden="true">✦</span>
                <strong>{inputPath ? "Preview unavailable" : "Choose an image to begin"}</strong>
                <span>{inputPath ? "The editor can still process this file." : "Your original stays on this device."}</span>
              </div>
            )}
            {sourcePreview && <span className="image-editor-dimensions">{sourcePreview.width} × {sourcePreview.height}px</span>}
            {previewError && <span className="image-editor-preview-error" role="status">{previewError}</span>}
            {resultError && !unsupportedText && <span className="image-editor-preview-error" role="status">{resultError}</span>}
            {steps.length > 0 && resultPreview && <Card className="image-editor-result-preview py-0" role="region" aria-label="Result preview">
              <CardContent className="image-editor-result-preview-content grid gap-2 p-2">
                <p className="workspace-panel-label">After {steps.length === 1 ? "this step" : `all ${steps.length} steps`}</p>
                <img src={resultPreview.dataUrl} alt={`Result preview of ${fileName(inputPath)}`} />
                <span>{resultPreview.width} × {resultPreview.height}px</span>
              </CardContent>
            </Card>}
          </CardContent>
        </Card>
        {focus === "watermark" && watermark && sourcePreview && (
          <p className="workspace-panel-copy" role="status">
            {watermark.placement === "tile"
              ? "Tiles cover the whole image, so there is nothing to drag."
              : `Drag the ${watermark.mark === "text" ? "text" : "picture"} to place it, a corner to resize it, or the top handle to turn it. Arrow keys move it.`}
          </p>
        )}
        </div>
        <Card className="image-editor-controls py-0" role="region" aria-label="Options">
          <CardContent className="image-editor-controls-content grid gap-4 p-4">
            {tabs}
            {focused && (
              <div className="image-edit-stack-focus">
                <h2 className="workspace-active-command">{imageEditStepLabels[focus]}</h2>
                <Button type="button" size="sm" variant="ghost" onClick={() => onRemove(focus)}>Remove step</Button>
                <p className="workspace-panel-copy">{stepOrder(steps, focus)}</p>
              </div>
            )}
            {focus === "crop" && crop ? (
              <>
                <fieldset className="workspace-field">
                  <legend className="workspace-panel-label">Crop by</legend>
                  <ToggleGroup type="single" variant="outline" size="sm" aria-label="Crop by" value={crop.mode} onValueChange={selectCropMode}>
                    <ToggleGroupItem value="aspectRatio">Aspect ratio</ToggleGroupItem>
                    <ToggleGroupItem value="rectangle">Pixel rectangle</ToggleGroupItem>
                  </ToggleGroup>
                </fieldset>
                {crop.mode === "aspectRatio" ? (
                  <>
                    <fieldset className="workspace-field">
                      <legend className="workspace-panel-label">Ratio</legend>
                      <ToggleGroup
                        type="single"
                        variant="outline"
                        size="sm"
                        aria-label="Ratio"
                        value={ratioValue}
                        onValueChange={(value) => {
                          const ratio = cropRatios.find(([width, height]) => `${width}:${height}` === value);
                          if (ratio) onEditCrop({ ...crop, aspectWidth: ratio[0], aspectHeight: ratio[1] });
                        }}
                      >
                        {cropRatios.map(([width, height]) => <ToggleGroupItem key={`${width}:${height}`} value={`${width}:${height}`}>{width}:{height}</ToggleGroupItem>)}
                      </ToggleGroup>
                    </fieldset>
                    <div className="workspace-field">
                      <Label htmlFor="image-crop-keep">Keep</Label>
                      <NativeSelect id="image-crop-keep" value={crop.anchor} onChange={(event) => onEditCrop({ ...crop, anchor: event.target.value })}>
                        {keepOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                      </NativeSelect>
                      <p className="workspace-panel-copy">Each image keeps the largest {ratioValue} area it has, so every size in the batch works. Drag the area in the preview to choose the part to keep.</p>
                    </div>
                  </>
                ) : (
                  <fieldset className="workspace-field">
                    <legend className="workspace-panel-label">Rectangle, pixels</legend>
                    <div className="workspace-field-grid">
                      {pixelFields.map((field) => (
                        <div className="workspace-field" key={field.key}>
                          <Label htmlFor={`image-crop-${field.key}`}>{field.label}</Label>
                          <Input
                            id={`image-crop-${field.key}`}
                            aria-label={field.aria}
                            type="number"
                            inputMode="numeric"
                            min={field.min}
                            value={crop[field.key]}
                            onChange={(event) => onEditCrop({ ...crop, [field.key]: Number(event.target.value) }, `field-${field.key}`)}
                          />
                        </div>
                      ))}
                    </div>
                    <p className="workspace-panel-copy">The same pixels are cut from every image. Drag the area in the preview to move it. An image smaller than the rectangle fails on its own.</p>
                  </fieldset>
                )}
              </>
            ) : focus === "resize" && resize ? (
              <>
                <fieldset className="workspace-field">
                  <legend className="workspace-panel-label">Resize by</legend>
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    aria-label="Resize by"
                    className="w-full"
                    value={resize.mode}
                    onValueChange={(mode) => { if (mode && mode !== resize.mode) onEditResize({ ...resize, mode }); }}
                  >
                    {resizeModes.map((mode) => <ToggleGroupItem key={mode.value} value={mode.value} className="h-auto min-h-8 min-w-0 flex-1 px-2 whitespace-normal leading-tight">{mode.label}</ToggleGroupItem>)}
                  </ToggleGroup>
                </fieldset>
                {resize.mode === "longestSide" ? (
                  <div className="workspace-field">
                    <Label htmlFor="image-resize-longest-side">Longest side, pixels</Label>
                    <Input
                      id="image-resize-longest-side"
                      type="number"
                      inputMode="numeric"
                      min={1}
                      max={16384}
                      value={resize.longestSide}
                      aria-invalid={resizeProblem ? true : undefined}
                      aria-describedby="image-resize-longest-side-hint"
                      onChange={(event) => onEditResize({ ...resize, longestSide: Number(event.target.value) }, "resize-longest-side")}
                    />
                    {resizeProblem
                      ? <p id="image-resize-longest-side-hint" className="text-sm text-destructive">{resizeProblem.message}</p>
                      : <p id="image-resize-longest-side-hint" className="workspace-panel-copy">Each image is scaled so its longer edge is {resize.longestSide} px, keeping its shape. Portrait and landscape images both work. Up to 16384.</p>}
                  </div>
                ) : resize.mode === "exact" ? (
                  <fieldset className="workspace-field">
                    <legend className="workspace-panel-label">Size, pixels</legend>
                    <div className="image-resize-size">
                      <div className="workspace-field">
                        <Label htmlFor="image-resize-width">Width</Label>
                        <Input id="image-resize-width" type="number" inputMode="numeric" min={1} max={16384} value={resize.width} aria-invalid={resizeProblem ? true : undefined} aria-describedby="image-resize-size-hint" onChange={(event) => onEditResize({ ...resize, width: Number(event.target.value) }, "resize-width")} />
                      </div>
                      <Toggle variant="outline" size="sm" aria-label="Keep aspect ratio" title="Keep aspect ratio" pressed={resize.keepAspectRatio} onPressedChange={(keepAspectRatio) => onEditResize({ ...resize, keepAspectRatio })}>
                        <Link2 aria-hidden="true" />
                      </Toggle>
                      <div className="workspace-field">
                        <Label htmlFor="image-resize-height">Height</Label>
                        <Input id="image-resize-height" type="number" inputMode="numeric" min={1} max={16384} value={resize.height} aria-invalid={resizeProblem ? true : undefined} aria-describedby="image-resize-size-hint" onChange={(event) => onEditResize({ ...resize, height: Number(event.target.value) }, "resize-height")} />
                      </div>
                    </div>
                    {resizeProblem
                      ? <p id="image-resize-size-hint" className="text-sm text-destructive">{resizeProblem.message}</p>
                      : <p id="image-resize-size-hint" className="workspace-panel-copy">{resize.keepAspectRatio
                        ? `Keep aspect ratio is on: each image fits inside ${resize.width} × ${resize.height} and keeps its shape. Turn it off to make every image exactly ${resize.width} × ${resize.height}.`
                        : `Keep aspect ratio is off: every image becomes exactly ${resize.width} × ${resize.height}, even if that stretches it. Turn it on to keep each image's shape.`}</p>}
                  </fieldset>
                ) : (
                  <div className="workspace-field">
                    <Label htmlFor="image-resize-percentage">Percentage</Label>
                    <Input
                      id="image-resize-percentage"
                      type="number"
                      inputMode="numeric"
                      min={1}
                      max={1000}
                      value={resize.percentage}
                      aria-invalid={resizeProblem ? true : undefined}
                      aria-describedby="image-resize-percentage-hint"
                      onChange={(event) => onEditResize({ ...resize, percentage: Number(event.target.value) }, "resize-percentage")}
                    />
                    {resizeProblem
                      ? <p id="image-resize-percentage-hint" className="text-sm text-destructive">{resizeProblem.message}</p>
                      : <p id="image-resize-percentage-hint" className="workspace-panel-copy">Each image is scaled to {resize.percentage}% of its own size. From 1 to 1000.</p>}
                  </div>
                )}
                <div className="workspace-field">
                  <Label htmlFor="image-resize-resampling">Resampling</Label>
                  <NativeSelect id="image-resize-resampling" value={resize.resampling} onChange={(event) => onEditResize({ ...resize, resampling: event.target.value })}>
                    {resamplingOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                  </NativeSelect>
                </div>
              </>
            ) : focus === "rotate" && rotate ? (
              <>
                <fieldset className="workspace-field">
                  <legend className="workspace-panel-label">Rotate</legend>
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    aria-label="Rotate"
                    aria-describedby="image-rotate-hint"
                    className="w-full"
                    value={String(rotate.degrees)}
                    onValueChange={(value) => { if (value && Number(value) !== rotate.degrees) onEditRotate({ ...rotate, degrees: Number(value) }); }}
                  >
                    {imageRotateDegrees.map((degrees) => <ToggleGroupItem key={degrees} value={String(degrees)} aria-label={`${degrees} degrees`} className="min-w-0 flex-1">{degrees}°</ToggleGroupItem>)}
                  </ToggleGroup>
                  <p id="image-rotate-hint" className="workspace-panel-copy">Clockwise, as you see the image in the preview.</p>
                </fieldset>
                <fieldset className="workspace-field">
                  <legend className="workspace-panel-label">Flip</legend>
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    aria-label="Flip"
                    className="w-full"
                    value={rotate.flip}
                    onValueChange={(flip) => { if (flip && flip !== rotate.flip) onEditRotate({ ...rotate, flip }); }}
                  >
                    {imageFlips.map((flip) => <ToggleGroupItem key={flip} value={flip} className="min-w-0 flex-1">{flipLabels[flip]}</ToggleGroupItem>)}
                  </ToggleGroup>
                </fieldset>
                <p className="workspace-panel-copy flex gap-2"><Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />Rotated images are always re-encoded at quality 90, even with no other changes.</p>
                <p className="workspace-panel-copy flex gap-2"><Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />Each photo&apos;s camera orientation is applied to the pixels once, so the result shows upright everywhere.</p>
              </>
            ) : focus === "tone" && tone ? (
              <>
                {imageToneValues.map((key) => (
                  <div className="workspace-field" key={key}>
                    <Label id={`image-tone-${key}-label`} htmlFor={`image-tone-${key}`}>{toneLabels[key]}</Label>
                    <div className="grid grid-cols-[minmax(0,1fr)_4rem] items-center gap-2.5">
                      <Slider
                        aria-labelledby={`image-tone-${key}-label`}
                        min={-100}
                        max={100}
                        step={1}
                        value={[Math.min(Math.max(tone[key], -100), 100)]}
                        onValueChange={([value]) => { if (value !== undefined && value !== tone[key]) onEditTone({ ...tone, [key]: value }, `tone-${key}`); }}
                      />
                      <Input
                        id={`image-tone-${key}`}
                        aria-label={`${toneLabels[key]}, from −100 to 100`}
                        inputMode="numeric"
                        value={toneText[key] ?? formatToneValue(tone[key])}
                        aria-invalid={toneProblem?.field === key ? true : undefined}
                        aria-describedby="image-tone-hint"
                        onChange={(event) => {
                          const text = event.target.value;
                          setToneText((current) => ({ ...current, [key]: text }));
                          const value = parseToneValue(text);
                          if (value !== null && value !== tone[key]) onEditTone({ ...tone, [key]: value }, `tone-field-${key}`);
                        }}
                        onBlur={() => setToneText((current) => ({ ...current, [key]: undefined }))}
                      />
                    </div>
                  </div>
                ))}
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={isNeutralImageToneStep(tone)}
                    onClick={() => {
                      setToneText({});
                      onEditTone(defaultImageToneStep());
                    }}
                  >
                    Reset tone
                  </Button>
                  {toneProblem
                    ? <p id="image-tone-hint" className="text-sm text-destructive">{toneProblem.message}</p>
                    : <p id="image-tone-hint" className="workspace-panel-copy">Each runs from −100 to 100. 0 leaves it as it is.</p>}
                </div>
                <p className="workspace-panel-copy flex gap-2" role="note">
                  <Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
                  {isNeutralImageToneStep(tone)
                    ? "This step changes nothing. With no other step, every image is Skipped as No changes, unless you choose another format below."
                    : "Transparent areas stay transparent."}
                </p>
              </>
            ) : focus === "watermark" && watermark ? (
              <>
                <div className="flex flex-wrap items-center gap-2.5">
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    aria-label="Watermark type"
                    value={watermark.mark}
                    onValueChange={(mark) => { if ((mark === "text" || mark === "picture") && mark !== watermark.mark) onEditWatermark({ ...watermark, mark }); }}
                  >
                    <ToggleGroupItem value="text">Text</ToggleGroupItem>
                    <ToggleGroupItem value="picture">Picture</ToggleGroupItem>
                  </ToggleGroup>
                  <span className="workspace-panel-copy">Same options as the PDF watermark</span>
                </div>
                {watermark.mark === "text" ? (
                  <div className="workspace-field">
                    <Label htmlFor="image-watermark-text">Text</Label>
                    <Input
                      id="image-watermark-text"
                      value={watermark.text}
                      aria-invalid={unsupportedText ? true : undefined}
                      aria-describedby={unsupportedText || watermarkProblem?.field === "text" ? "image-watermark-text-hint" : undefined}
                      onChange={(event) => onEditWatermark({ ...watermark, text: event.target.value }, "watermark-text")}
                    />
                    {unsupportedText
                      ? <p id="image-watermark-text-hint" className="flex gap-2 text-sm text-destructive"><CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />{unsupportedText} Remove it, or use a picture instead.</p>
                      : watermarkProblem?.field === "text" && <p id="image-watermark-text-hint" className="workspace-panel-copy">{watermarkProblem.message}</p>}
                  </div>
                ) : (
                  <div className="workspace-field" role="group" aria-labelledby="image-watermark-picture-label">
                    <Label id="image-watermark-picture-label">Picture</Label>
                    <div className="flex items-center gap-2">
                      <span className="image-watermark-picture-icon" aria-hidden="true"><ImageIcon className="size-4" /></span>
                      <span className="min-w-0 flex-1 truncate">{watermark.logoPath ? fileName(watermark.logoPath) : "No picture chosen"}</span>
                      <Button type="button" size="sm" variant="outline" aria-label="Choose a picture for the watermark" onClick={() => void choosePicture()}>Choose…</Button>
                    </div>
                    {pictureError && <p className="text-sm text-destructive">{pictureError}</p>}
                  </div>
                )}
                <div className="workspace-field-grid">
                  <div className="workspace-field">
                    <Label htmlFor="image-watermark-size">Size, % of width</Label>
                    <Input id="image-watermark-size" type="number" inputMode="numeric" min={1} max={100} value={watermark.size} aria-invalid={watermarkFieldProblem?.field === "size" ? true : undefined} onChange={(event) => onEditWatermark({ ...watermark, size: Number(event.target.value) }, "watermark-size")} />
                  </div>
                  <div className="workspace-field">
                    <Label htmlFor="image-watermark-opacity">Opacity</Label>
                    <Input id="image-watermark-opacity" type="number" inputMode="numeric" min={1} max={100} value={watermark.opacity} aria-invalid={watermarkFieldProblem?.field === "opacity" ? true : undefined} onChange={(event) => onEditWatermark({ ...watermark, opacity: Number(event.target.value) }, "watermark-opacity")} />
                  </div>
                  {watermark.mark === "text" ? (
                    <div className="workspace-field">
                      <Label htmlFor="image-watermark-colour">Colour</Label>
                      <div className="flex gap-1.5">
                        <input
                          type="color"
                          className="image-watermark-swatch"
                          aria-label="Pick the watermark colour"
                          value={/^#[0-9a-f]{6}$/i.test(watermark.color) ? watermark.color.toLowerCase() : "#ffffff"}
                          onChange={(event) => onEditWatermark({ ...watermark, color: event.target.value.toUpperCase() }, "watermark-colour-picker")}
                        />
                        <Input id="image-watermark-colour" value={watermark.color} aria-invalid={watermarkFieldProblem?.field === "colour" ? true : undefined} onChange={(event) => onEditWatermark({ ...watermark, color: event.target.value }, "watermark-colour")} />
                      </div>
                    </div>
                  ) : (
                    <div className="workspace-field">
                      <span className="workspace-panel-label">Colour</span>
                      <p className="workspace-panel-copy">Colour applies to text only. Pictures keep their own colours.</p>
                    </div>
                  )}
                  <div className="workspace-field">
                    <Label htmlFor="image-watermark-angle">Angle</Label>
                    <NativeSelect
                      id="image-watermark-angle"
                      value={angleValue}
                      onChange={(event) => {
                        if (event.target.value === "custom") {
                          setCustomAngle(true);
                          return;
                        }
                        setCustomAngle(false);
                        onEditWatermark({ ...watermark, rotation: Number(event.target.value) });
                      }}
                    >
                      {!watermarkAngles.some((angle) => angle === watermark.rotation) && <option value={String(watermark.rotation)}>{formatAngle(watermark.rotation)}</option>}
                      {watermarkAngles.map((angle) => <option key={angle} value={String(angle)}>{formatAngle(angle)}</option>)}
                      <option value="custom">Custom…</option>
                    </NativeSelect>
                    {customAngle && (
                      <Input
                        aria-label="Angle, degrees from −180 to 180"
                        type="number"
                        inputMode="numeric"
                        min={-180}
                        max={180}
                        value={watermark.rotation}
                        aria-invalid={watermarkFieldProblem?.field === "angle" ? true : undefined}
                        onChange={(event) => onEditWatermark({ ...watermark, rotation: Number(event.target.value) }, "watermark-angle")}
                      />
                    )}
                  </div>
                </div>
                {watermarkFieldProblem && <p className="text-sm text-destructive">{watermarkFieldProblem.message}</p>}
                <fieldset className="workspace-field">
                  <legend className="workspace-panel-label">Placement</legend>
                  <div className="flex flex-wrap items-start gap-3">
                    <ToggleGroup
                      type="single"
                      variant="outline"
                      size="sm"
                      aria-label="Placement"
                      value={watermark.placement}
                      onValueChange={(placement) => { if ((placement === "position" || placement === "tile") && placement !== watermark.placement) onEditWatermark({ ...watermark, placement }); }}
                    >
                      <ToggleGroupItem value="position">Position</ToggleGroupItem>
                      <ToggleGroupItem value="tile">Tile</ToggleGroupItem>
                    </ToggleGroup>
                    {watermark.placement === "position" && (
                      <RadioGroup
                        aria-label="Position on the image"
                        className="image-watermark-anchors"
                        value={anchorValue}
                        onValueChange={(value) => {
                          const [anchorX, anchorY] = value.split(",").map(Number);
                          onEditWatermark({ ...watermark, anchorX, anchorY });
                        }}
                      >
                        {anchorSteps.flatMap((y) => anchorSteps.map((x) => (
                          <RadioGroupItem key={`${x},${y}`} value={`${x},${y}`} aria-label={capitalise(anchorName(x, y))} />
                        )))}
                      </RadioGroup>
                    )}
                  </div>
                  <p className="workspace-panel-copy">{watermark.placement === "tile" ? "Repeats across the whole image." : "Or drag it in the preview. It lands in the same place on every size."}</p>
                </fieldset>
              </>
            ) : focus === "metadata" && stack.metadata ? (
              <>
                <div className="workspace-field">
                  <p id="image-metadata-removes" className="workspace-panel-label">Removes</p>
                  <ul aria-labelledby="image-metadata-removes" className="workspace-panel-copy grid gap-0.5 pl-5 text-foreground list-disc">
                    <li>Camera details (EXIF), including GPS location</li>
                    <li>XMP</li>
                    <li>IPTC</li>
                    <li>Comments</li>
                  </ul>
                </div>
                <div className="workspace-field">
                  <p id="image-metadata-keeps" className="workspace-panel-label">Keeps</p>
                  <ul aria-labelledby="image-metadata-keeps" className="workspace-panel-copy grid gap-0.5 pl-5 text-foreground list-disc">
                    <li>Orientation, so photos stay upright</li>
                    <li>Colour profile, so colours look the same</li>
                  </ul>
                </div>
                <p className="workspace-panel-copy flex gap-2" role="note"><Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />JPEG and PNG are cleaned without re-encoding. HEIC, WebP and TIFF are re-encoded at quality 90.</p>
                <p className="workspace-panel-copy flex gap-2" role="note"><Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />Each copy is checked after saving. If anything removed is still there, that file fails and no copy is kept.</p>
              </>
            ) : (
              <Card className="py-0" role="note">
                <CardContent className="grid gap-1 p-4">
                  <strong>{steps.length === 0 ? "No steps in the stack" : `${imageEditStepLabels[focus]} is not in the stack`}</strong>
                  <p className="workspace-panel-copy">Pick a tool in the sidebar, or a dashed step above, to add it. Steps always run in the same order, whichever you add first.</p>
                </CardContent>
              </Card>
            )}
            <div className="grid gap-2">
              <Label htmlFor="image-edit-stack-format">Format</Label>
              <NativeSelect id="image-edit-stack-format" value={format} onChange={(event) => onFormatChange(event.target.value as ImageEditStackFormat)}>
                {stackFormats.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
              </NativeSelect>
              {format === "jpg" ? (
                <>
                  <p className="workspace-panel-copy flex gap-2" role="note">
                    <Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
                    Transparent areas are flattened onto white, because JPEG has none.
                  </p>
                  <p className="workspace-panel-copy flex gap-2" role="note">
                    <Info aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
                    JPEG is saved at quality 90.
                  </p>
                </>
              ) : <p className="workspace-panel-copy">JPEG and HEIC save at quality 90.</p>}
            </div>
            {destination}
            <div className="image-edit-stack-footer">
              <span className="workspace-panel-copy">{footHint}</span>
              <Button variant="default" disabled={!canRun} onClick={run} className="workspace-primary-action" {...shortcutTarget("run")}>
                {files.length ? `Run on ${files.length} ${files.length === 1 ? "image" : "images"}` : "Run"}<ShortcutHint id="run" />
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
};
