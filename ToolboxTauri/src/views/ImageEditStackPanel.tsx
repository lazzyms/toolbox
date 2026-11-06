import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Redo2, Undo2, X } from "lucide-react";
import type { ImagePreview, OutputLocation } from "../contracts";
import {
  imageEditRequestPlan,
  presentImageEditSteps,
  resolveImageCropRect,
  type EditHistory,
  type ImageCropStep,
  type ImageEditStack,
} from "../features/image-editor/session";
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { NativeSelect } from "../components/ui/native-select";
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

const fileName = (path: string | undefined) => path?.split(/[\\/]/).pop() ?? "selected image";

// Native rejects a crop whose size or ratio is not positive, so Run waits for a well-formed step.
const cropStepIsRunnable = (crop: ImageCropStep) => crop.mode === "aspectRatio"
  ? crop.aspectWidth > 0 && crop.aspectHeight > 0
  : crop.width > 0 && crop.height > 0 && crop.x >= 0 && crop.y >= 0;

/** The shared edit stack: one step per tool, applied to every image in one run. Crop is its only step so far. */
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
  onAddCrop,
  onEditCrop,
  onRemoveCrop,
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
  onAddCrop: () => void;
  /** Edits sharing a coalesce key replace one another in the history instead of each adding an undo step. */
  onEditCrop: (crop: ImageCropStep, coalesceKey?: string) => void;
  onRemoveCrop: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onReset: () => void;
}) => {
  const stack = history.present;
  const crop = stack.crop;
  const steps = presentImageEditSteps(stack);
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
  const canRun = files.length > 0 && crop !== undefined && cropStepIsRunnable(crop) && !loading;

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

  const cropKey = JSON.stringify(crop ?? null);
  useEffect(() => {
    setResultPreview(null);
    setResultError("");
    if (!inputPath || !crop || !cropStepIsRunnable(crop)) return;
    let current = true;
    void invoke<ImagePreview>("inspect_image_edit_preview", {
      request: { path: inputPath, plan: imageEditRequestPlan({ edits: [{ kind: "crop", ...crop }] }, outputLocation) },
    })
      .then((value) => { if (current && value && typeof value.dataUrl === "string") setResultPreview(value); })
      .catch((error) => { if (current) setResultError(String(error)); });
    return () => { current = false; };
  }, [inputPath, cropKey]);

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
  const footHint = files.length === 0 ? "Add images to run" : !crop ? "Add a step to run" : "Originals stay as they are";
  const cropAria = crop?.mode === "rectangle"
    ? `Crop area, ${crop.width} by ${crop.height} pixels, from ${crop.x}, ${crop.y}. Drag to move it.`
    : crop ? `Crop area, ${ratioValue}, keeping ${keepOptions.find((option) => option.value === crop.anchor)?.label.toLowerCase() ?? "the centre"}. Drag to choose the part to keep.` : "";

  return (
    <div className="image-edit-stack">
      <div className="image-edit-stack-toolbar" role="toolbar" aria-label="Edit stack">
        <div className="image-edit-stack-steps" role="list" aria-label="Steps, in the order they run">
          {crop ? (
            <span role="listitem" className="image-edit-stack-step">
              <Button type="button" size="sm" variant="secondary" aria-pressed="true" aria-label={`Step ${steps.indexOf("crop") + 1} of ${steps.length}, Crop, showing its options`}>
                <span aria-hidden="true">{steps.indexOf("crop") + 1}</span>Crop
              </Button>
              <Button type="button" size="icon-sm" variant="ghost" aria-label="Remove Crop step" onClick={onRemoveCrop}><X aria-hidden="true" /></Button>
            </span>
          ) : (
            <span role="listitem" className="image-edit-stack-step">
              <Button type="button" size="sm" variant="outline" className="border-dashed" aria-label="Add Crop step" onClick={onAddCrop}>+ Crop</Button>
            </span>
          )}
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
            {crop && resultPreview && <Card className="image-editor-result-preview py-0" role="region" aria-label="Crop result preview">
              <CardContent className="image-editor-result-preview-content grid gap-2 p-2">
                <p className="workspace-panel-label">Resulting crop</p>
                <img src={resultPreview.dataUrl} alt={`Resulting crop preview of ${fileName(inputPath)}`} />
                <span>{resultPreview.width} × {resultPreview.height}px</span>
              </CardContent>
            </Card>}
          </CardContent>
        </Card>
        </div>
        <Card className="image-editor-controls py-0" role="region" aria-label="Options">
          <CardContent className="image-editor-controls-content grid gap-4 p-4">
            {tabs}
            {crop ? (
              <>
                <div className="image-edit-stack-focus">
                  <h2 className="workspace-active-command">Crop</h2>
                  <Button type="button" size="sm" variant="ghost" onClick={onRemoveCrop}>Remove step</Button>
                  <p className="workspace-panel-copy">Step {steps.indexOf("crop") + 1} of {steps.length}. {steps.length === 1 ? "The only step." : ""}</p>
                </div>
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
            ) : (
              <Card className="py-0" role="note">
                <CardContent className="grid gap-1 p-4">
                  <strong>No steps in the stack</strong>
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
