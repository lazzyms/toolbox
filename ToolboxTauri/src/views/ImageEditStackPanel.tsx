import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Info, Link2, Redo2, Undo2, X } from "lucide-react";
import type { ImagePreview, OutputLocation } from "../contracts";
import {
  imageEditRequestPlan,
  defaultImageToneStep,
  imageEditStackPreviewPlan,
  imageEditStackSteps,
  imageEditStepLabels,
  imageFlips,
  imageResizeStepProblem,
  imageRotateDegrees,
  imageToneStepProblem,
  imageToneValues,
  isNeutralImageToneStep,
  presentImageEditSteps,
  resolveImageCropRect,
  type EditHistory,
  type ImageCropStep,
  type ImageEditStack,
  type ImageEditStackStep,
  type ImageResizeStep,
  type ImageRotateStep,
  type ImageToneStep,
} from "../features/image-editor/session";
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { NativeSelect } from "../components/ui/native-select";
import { Slider } from "../components/ui/slider";
import { Toggle } from "../components/ui/toggle";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { isWindows, shortcutTarget } from "../shortcuts";

export type StackMessage = { text: string; undo: boolean };

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
  return tone ? `Fix the ${tone.field} to run` : null;
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
  focus,
  onFocus,
  onAdd,
  onEditCrop,
  onEditResize,
  onEditRotate,
  onEditTone,
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
  /** The step whose options are showing; it follows the chosen tool. */
  focus: ImageEditStackStep;
  onFocus: (step: ImageEditStackStep) => void;
  onAdd: (step: ImageEditStackStep) => void;
  /** Edits sharing a coalesce key replace one another in the history instead of each adding an undo step. */
  onEditCrop: (crop: ImageCropStep, coalesceKey?: string) => void;
  onEditResize: (resize: ImageResizeStep, coalesceKey?: string) => void;
  onEditRotate: (rotate: ImageRotateStep) => void;
  onEditTone: (tone: ImageToneStep, coalesceKey?: string) => void;
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
  const dragCount = useRef(0);
  // What the user is typing in a tone field, kept until it leaves the field so "-" or "+" can start a value.
  const [toneText, setToneText] = useState<Partial<Record<(typeof imageToneValues)[number], string>>>({});
  const canRun = files.length > 0 && steps.length > 0 && !problem && !loading;

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
    const pointerId = drag.current?.pointerId;
    drag.current = null;
    const stage = previewStageRef.current;
    if (pointerId !== undefined && stage?.hasPointerCapture(pointerId)) stage.releasePointerCapture(pointerId);
  };
  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId === event.pointerId) clearDrag();
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
  const footHint = files.length === 0 ? "Add images to run" : steps.length === 0 ? "Add a step to run" : problem ?? "Originals stay as they are";
  const resizeProblem = resize ? imageResizeStepProblem(resize) : null;
  const toneProblem = tone ? imageToneStepProblem(tone) : null;
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
                  <span aria-hidden="true">{steps.indexOf(step) + 1}</span>{imageEditStepLabels[step]}
                </Button>
                <Button type="button" size="icon-sm" variant="ghost" aria-label={`Remove ${imageEditStepLabels[step]} step`} onClick={() => onRemove(step)}><X aria-hidden="true" /></Button>
              </span>
            ) : (
              <span key={step} role="listitem" className="image-edit-stack-step">
                {separator}
                <Button type="button" size="sm" variant="outline" className="border-dashed" aria-label={`Add ${imageEditStepLabels[step]} step`} onClick={() => onAdd(step)}>+ {imageEditStepLabels[step]}</Button>
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
                className={`image-editor-preview-stage ${crop ? "image-editor-preview-stage-interactive" : ""}`}
                style={{ aspectRatio: `${sourcePreview.width} / ${sourcePreview.height}` }}
                onPointerDown={crop ? beginDrag : undefined}
                onPointerMove={crop ? moveDrag : undefined}
                onPointerUp={endDrag}
                onPointerCancel={endDrag}
              >
                <img src={sourcePreview.dataUrl} alt={`Original image preview of ${fileName(inputPath)}`} />
                {cropOverlay && <div className="image-editor-crop-overlay" role="group" aria-label="Crop selection" aria-describedby="image-crop-area-description" style={cropOverlay}><div className="image-editor-crop-grid" aria-hidden="true"><span /><span /><span /><span /></div><span id="image-crop-area-description" className="sr-only">{cropAria}</span></div>}
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
            {resultError && <span className="image-editor-preview-error" role="status">{resultError}</span>}
            {steps.length > 0 && resultPreview && <Card className="image-editor-result-preview py-0" role="region" aria-label="Result preview">
              <CardContent className="image-editor-result-preview-content grid gap-2 p-2">
                <p className="workspace-panel-label">After {steps.length === 1 ? "this step" : `all ${steps.length} steps`}</p>
                <img src={resultPreview.dataUrl} alt={`Result preview of ${fileName(inputPath)}`} />
                <span>{resultPreview.width} × {resultPreview.height}px</span>
              </CardContent>
            </Card>}
          </CardContent>
        </Card>
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
                    ? "This step changes nothing. All four values are 0, so the preview matches the original."
                    : "Transparent areas stay transparent."}
                </p>
              </>
            ) : (
              <Card className="py-0" role="note">
                <CardContent className="grid gap-1 p-4">
                  <strong>{steps.length === 0 ? "No steps in the stack" : `${imageEditStepLabels[focus]} is not in the stack`}</strong>
                  <p className="workspace-panel-copy">Pick a tool in the sidebar, or a dashed step above, to add it. Steps always run in the same order, whichever you add first.</p>
                </CardContent>
              </Card>
            )}
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
