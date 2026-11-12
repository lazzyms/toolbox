import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { exampleOutputName } from "../components/RunDestination";
import type { WorkspaceSourceAction } from "../components/ToolScaffold";
import type { ShellEvent } from "../hooks/useShellBridge";
import type {
  AtomicToolId,
  ImagePreview,
  OutputLocation,
  ToolDefinition,
  ToolResult,
} from "../contracts";
import {
  commitImageEdit,
  createEditHistory,
  createImageEditHistory,
  defaultImageCropStep,
  defaultImageResizeStep,
  defaultImageRotateStep,
  defaultImageToneStep,
  defaultImageWatermarkStep,
  editedImageExtension,
  imageEditRequestPlan,
  imageEditStackExtension,
  imageEditStackRequestSteps,
  imageEditStackSuffix,
  imageEditStepLabels,
  planWithDraft,
  presentImageEditSteps,
  pushEdit,
  redoImageEdit,
  resetImageEdits,
  undoImageEdit,
  type EditHistory,
  type ImageCropStep,
  type ImageEditOperation,
  type ImageEditPlan,
  type ImageEditHistory,
  type ImageEditStack,
  type ImageEditStackFormat,
  type ImageEditStackStep,
  type ImageResizeStep,
  type ImageRotateStep,
  type ImageToneStep,
  type ImageWatermarkStep,
} from "../features/image-editor/session";
import { ImageEditStackPanel, type StackMessage } from "./ImageEditStackPanel";
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";
import { UtilityRegistry, workspaceById } from "../registry";
import { useToolAvailability } from "../ToolAvailabilityContext";

// Tools that are steps in the shared edit stack, not drafts of the edit plan.
const stackStepForTool: Partial<Record<string, ImageEditStackStep>> = { crop: "crop", resize: "resize", rotate: "rotate", "image-tone": "tone", "image-watermark": "watermark", "image-metadata": "metadata" };
const toolForStackStep: Record<ImageEditStackStep, AtomicToolId> = { crop: "crop", resize: "resize", rotate: "rotate", tone: "image-tone", watermark: "image-watermark", metadata: "image-metadata" };
const stackStepDefaults: { [Step in ImageEditStackStep]: () => NonNullable<ImageEditStack[Step]> } = { crop: defaultImageCropStep, resize: defaultImageResizeStep, rotate: defaultImageRotateStep, tone: defaultImageToneStep, watermark: defaultImageWatermarkStep, metadata: () => ({}) };


const imageEditLabel = (edit: ImageEditOperation) => {
  if (edit.kind === "resize") return `Resize ${edit.width} × ${edit.height}`;
  if (edit.kind === "rotate") return `Rotate ${edit.degrees}°${edit.flip !== "none" ? ` · ${edit.flip}` : ""}`;
  if (edit.kind === "tone") return `Tone · ${edit.brightness}/${edit.contrast}/${edit.saturation}/${edit.exposure}`;
  if (edit.kind === "watermark") return `Watermark · ${edit.text || "Text"}`;
  if (edit.kind === "convert") return `Convert · ${edit.format.toUpperCase()}`;
  return `Crop ${edit.width} × ${edit.height}`;
};

const editImageTabLabel: Partial<Record<AtomicToolId, string>> = { "image-tone": "Adjust" };

/** Resize, Crop, Rotate, Adjust, Watermark and Remove Metadata share one edit plan and switch as tabs. */
const EditImageTabs = ({ activeId, onSelect }: { activeId: AtomicToolId; onSelect: (id: AtomicToolId) => void }) => {
  const availability = useToolAvailability();
  const tools = availability.filter(UtilityRegistry.filter((tool) => workspaceById("image-editor")?.editor?.toolIds.includes(tool.id)));
  if (!tools.some((tool) => tool.id === activeId)) return null;
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const index = tools.findIndex((tool) => tool.id === activeId);
    onSelect(tools[(index + step + tools.length) % tools.length].id);
    // Stack steps and plan edits render the tabs in different panels, so find the list again after the switch.
    requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('[role="tablist"][aria-label="Edit Image"] [aria-selected="true"]')?.focus());
  };
  return (
    <div className="edit-image-tabs" role="tablist" aria-label="Edit Image" onKeyDown={onKeyDown}>
      {tools.map((tool) => (
        <Button
          key={tool.id}
          type="button"
          role="tab"
          size="sm"
          variant={tool.id === activeId ? "secondary" : "ghost"}
          aria-selected={tool.id === activeId}
          tabIndex={tool.id === activeId ? 0 : -1}
          onClick={() => { if (tool.id !== activeId) onSelect(tool.id); }}
        >
          {editImageTabLabel[tool.id] ?? tool.shortTitle}
        </Button>
      ))}
    </div>
  );
};

export const ImageEditorWorkspaceView = ({
  utility: activeUtility,
  initialPaths,
  onSelectTool,
  fileActivation,
  onActivationAccepted,
  onFilesChange,
  onWorkspaceSourceAction,
}: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  /** Switches to another Edit Image tab; the view stays open with its files and edit plan. */
  onSelectTool?: (id: AtomicToolId) => void;
  fileActivation?: Extract<ShellEvent, { kind: "files" }>;
  onActivationAccepted?: (activationId: string) => void;
  onFilesChange?: (paths: readonly string[]) => void;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
}) => {
  const [stackFormat, setStackFormat] = useState<ImageEditStackFormat>("original");
  const [history, setHistory] = useState<ImageEditHistory>(() => createImageEditHistory());
  const [committedDraftKey, setCommittedDraftKey] = useState<string | null>(null);
  // Every Edit Image tool is now a stack step, so no tool drafts an edit-plan edit.
  const draft: ImageEditOperation | null = null;
  const draftKey = JSON.stringify(draft);
  const liveDraft = draftKey === committedDraftKey ? null : draft;
  const previousLiveDraft = useRef<{ toolId: string; draft: ImageEditOperation | null } | null>(null);

  // A tool switch keeps the outgoing tool's unsaved change as a committed edit.
  // The tool id check stops a remount of the same tool from committing its draft.
  useEffect(() => {
    const carried = previousLiveDraft.current;
    if (carried?.draft && carried.toolId !== activeUtility.id) {
      const edit = carried.draft;
      setHistory((current) => commitImageEdit(current, edit));
    }
    setCommittedDraftKey(draftKey);
  }, [activeUtility.id]);

  useEffect(() => {
    previousLiveDraft.current = { toolId: activeUtility.id, draft: liveDraft };
  });

  const editPlan = planWithDraft(history.present, liveDraft);

  const [stack, setStack] = useState<EditHistory<ImageEditStack>>(() => createEditHistory({}));
  const [stackMessage, setStackMessage] = useState<StackMessage | null>(null);
  const lastCoalesceKey = useRef<string | null>(null);
  const changeStack = useCallback((change: (current: ImageEditStack) => ImageEditStack | null, message: StackMessage | null, coalesceKey?: string) => {
    const coalesce = coalesceKey !== undefined && coalesceKey === lastCoalesceKey.current;
    lastCoalesceKey.current = coalesceKey ?? null;
    setStack((current) => {
      const next = change(current.present);
      if (!next) return current;
      return coalesce ? { ...current, present: next } : pushEdit(current, next);
    });
    setStackMessage(message);
  }, []);
  const stackStep = stackStepForTool[activeUtility.id];
  const addStep = useCallback((step: ImageEditStackStep) => {
    if (stack.present[step]) return;
    const position = presentImageEditSteps({ ...stack.present, [step]: stackStepDefaults[step]() }).indexOf(step) + 1;
    changeStack(
      (current) => current[step] ? null : { ...current, [step]: stackStepDefaults[step]() },
      { text: `${imageEditStepLabels[step]} added as step ${position}. Steps always run Crop, Resize, Rotate and Flip, Colour and Tone, Watermark, then Remove Metadata.`, undo: false },
    );
  }, [changeStack, stack.present]);
  const moveStackHistory = useCallback((move: (current: EditHistory<ImageEditStack>) => EditHistory<ImageEditStack>) => {
    lastCoalesceKey.current = null;
    setStack(move);
    setStackMessage(null);
  }, []);
  const onStackUndo = useCallback(() => moveStackHistory(undoImageEdit), [moveStackHistory]);
  const onStackRedo = useCallback(() => moveStackHistory(redoImageEdit), [moveStackHistory]);

  // Choosing a stack tool adds its step, or shows it when the stack already has one.
  useEffect(() => {
    if (stackStep) addStep(stackStep);
  }, [activeUtility.id]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="image-editor"
      utility={activeUtility}
      initialPaths={initialPaths}
      fileActivation={fileActivation}
      onActivationAccepted={onActivationAccepted}
      onFilesChange={onFilesChange}
      onWorkspaceSourceAction={onWorkspaceSourceAction}
      exampleName={(path) => stackStep
        ? exampleOutputName(path, activeUtility.command, imageEditStackSuffix(stack.present), imageEditStackExtension(stackFormat, path))
        : exampleOutputName(path, activeUtility.command, "-edited", editedImageExtension(editPlan, path))}
      onRun={(paths, outputLocation, { runId }) => {
        return invoke<ToolResult>("export_image_edit_stack", { runId, request: {
          paths,
          steps: imageEditStackRequestSteps(stack.present),
          format: stackFormat === "original" ? null : stackFormat,
          outputLocation,
        } });
      }}
      onRunCombined={(paths, outputLocation, { runId }) => invoke<ToolResult>("export_image_edit_plan", { runId,
        request: { paths, plan: imageEditRequestPlan(editPlan, outputLocation) },
      })}
    >
      {(props) => stackStep ? (
        <ImageEditStackPanel
          {...props}
          tabs={onSelectTool && <EditImageTabs activeId={activeUtility.id} onSelect={onSelectTool} />}
          history={stack}
          message={stackMessage}
          format={stackFormat}
          onFormatChange={setStackFormat}
          focus={stackStep}
          onFocus={(step) => { if (step !== stackStep) onSelectTool?.(toolForStackStep[step]); }}
          onAdd={(step) => {
            addStep(step);
            if (step !== stackStep) onSelectTool?.(toolForStackStep[step]);
          }}
          onEditCrop={(crop: ImageCropStep, coalesceKey?: string) => changeStack((current) => ({ ...current, crop }), null, coalesceKey)}
          onEditResize={(resize: ImageResizeStep, coalesceKey?: string) => changeStack((current) => ({ ...current, resize }), null, coalesceKey)}
          onEditRotate={(rotate: ImageRotateStep) => changeStack((current) => ({ ...current, rotate }), null)}
          onEditTone={(tone: ImageToneStep, coalesceKey?: string) => changeStack((current) => ({ ...current, tone }), null, coalesceKey)}
          onEditWatermark={(watermark: ImageWatermarkStep, coalesceKey?: string) => changeStack((current) => ({ ...current, watermark }), null, coalesceKey)}
          onRemove={(step) => changeStack(
            (current) => current[step] ? { ...current, [step]: undefined } : null,
            { text: `${imageEditStepLabels[step]} removed. The other steps keep their settings.`, undo: true },
          )}
          onUndo={onStackUndo}
          onRedo={onStackRedo}
          onReset={() => {
            const removed = presentImageEditSteps(stack.present).length;
            changeStack(
              (current) => presentImageEditSteps(current).length ? {} : null,
              { text: `Stack reset. ${removed} ${removed === 1 ? "step" : "steps"} removed. Your images are still in the list.`, undo: true },
            );
          }}
        />
      ) : <ImageEditorControls {...props} utility={activeUtility} onSelectTool={onSelectTool} {...{
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
  destination: React.ReactNode;
  files: string[];
  outputLocation: OutputLocation;
  runCombined: () => Promise<void>;
  loading: boolean;
  utility: ToolDefinition;
  onSelectTool?: (id: AtomicToolId) => void;
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
  destination,
  files,
  outputLocation,
  runCombined,
  loading,
  utility,
  onSelectTool,
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
  const [sourcePreviewError, setSourcePreviewError] = useState("");
  const [previewValidation, setPreviewValidation] = useState<ImagePreviewValidation | null>(null);
  const inputPath = files[0];
  const previewPlanKey = JSON.stringify({ plan, draft });
  const previousInputPath = useRef<string | undefined>(undefined);

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
      request: { path: inputPath, plan: imageEditRequestPlan(nextPlan, outputLocation) },
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
  const primaryPreview = resultPreview ?? sourcePreview;
  const primaryPreviewAlt = `Preview of ${inputPath?.split(/[\\/]/).pop() ?? "selected image"}`;

  return (
    <div className="image-editor-layout">
      <Card className="image-editor-preview" role="region" aria-label="Image preview">
        <CardContent className="image-editor-preview-content grid min-h-[340px] place-items-center overflow-hidden p-0">
        {primaryPreview ? (
          <div className="image-editor-preview-stage" style={{ aspectRatio: `${primaryPreview.width} / ${primaryPreview.height}` }}>
            <img src={primaryPreview.dataUrl} alt={primaryPreviewAlt} />
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
        </CardContent>
      </Card>
      <Card className="image-editor-controls py-0" role="region" aria-label="Image adjustments">
        <CardContent className="image-editor-controls-content grid gap-4 p-4">
        {onSelectTool && <EditImageTabs activeId={utility.id} onSelect={onSelectTool} />}
        <div className="workspace-panel-intro">
          <h2 className="workspace-active-command">{utility.title}</h2>
          <p className="workspace-panel-copy">Change the outcome without leaving the image editor.</p>
        </div>
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
        {destination}
        <Button variant="default" aria-label="Export edited images" disabled={loading || files.length === 0 || !hasEdits || !previewIsValid} onClick={runCombined} className="workspace-primary-action" {...shortcutTarget("run")}>Export edited images<ShortcutHint id="run" /></Button>
        </CardContent>
      </Card>
    </div>
  );
};
