import type { ConvertImagesRequest, OutputLocation, WatermarkOptions } from "../../contracts";

export type ImageEditOperation =
  | { kind: "convert"; format: ConvertImagesRequest["format"] }
  | {
      kind: "resize";
      width: number;
      height: number;
      mode: string;
      percentage: number;
      longestSide: number;
      resampling: string;
      keepAspectRatio: boolean;
    }
  | { kind: "rotate"; degrees: number; flip: string }
  | {
      kind: "crop";
      x: number;
      y: number;
      width: number;
      height: number;
      mode: string;
      aspectWidth: number;
      aspectHeight: number;
      anchor: string;
    }
  | { kind: "tone"; brightness: number; contrast: number; saturation: number; exposure: number }
  | ({ kind: "watermark" } & WatermarkOptions);

export interface ImageEditPlan {
  edits: ImageEditOperation[];
}

export const imageEditRequestPlan = (plan: ImageEditPlan, outputLocation: OutputLocation) => ({ ...plan, outputLocation });

export interface EditHistory<T> {
  past: T[];
  present: T;
  future: T[];
}

export type ImageEditHistory = EditHistory<ImageEditPlan>;

export type ImageCropStep = Omit<Extract<ImageEditOperation, { kind: "crop" }>, "kind">;
export type ImageResizeStep = Omit<Extract<ImageEditOperation, { kind: "resize" }>, "kind">;
export type ImageRotateStep = Omit<Extract<ImageEditOperation, { kind: "rotate" }>, "kind">;
export type ImageToneStep = Omit<Extract<ImageEditOperation, { kind: "tone" }>, "kind">;

/**
 * The watermark step as the panel edits it. It keeps both the text and the picture so switching between them loses neither,
 * and sends only the one in use. Size and opacity are whole percents; the anchor runs from 0 to 1 across the free space.
 */
export interface ImageWatermarkStep {
  mark: "text" | "picture";
  text: string;
  logoPath: string;
  size: number;
  opacity: number;
  color: string;
  rotation: number;
  placement: "position" | "tile";
  anchorX: number;
  anchorY: number;
}

/** The shared edit stack holds at most one of each step. Native runs them in a fixed order, so the stack has no order of its own. */
export interface ImageEditStack {
  crop?: ImageCropStep;
  resize?: ImageResizeStep;
  rotate?: ImageRotateStep;
  tone?: ImageToneStep;
  watermark?: ImageWatermarkStep;
  metadata?: ImageMetadataStep;
}

/** Removing metadata has no options: it always keeps the orientation and colour profile. */
export type ImageMetadataStep = Record<string, never>;

/** The stack as native reads it: the watermark goes as the shared watermark options. */
export type ImageEditStackRequest = Omit<ImageEditStack, "watermark"> & { watermark?: WatermarkOptions };

/** The fixed run order. */
export const imageEditStackSteps = ["crop", "resize", "rotate", "tone", "watermark", "metadata"] as const;
export type ImageEditStackStep = (typeof imageEditStackSteps)[number];

export const imageEditStepLabels: Record<ImageEditStackStep, string> = { crop: "Crop", resize: "Resize", rotate: "Rotate and Flip", tone: "Colour and Tone", watermark: "Watermark", metadata: "Remove Image Metadata" };
/** The shorter names the step chips show. Everything else, including the chips' labels for screen readers, uses the full name. */
export const imageEditStepChipLabels: Record<ImageEditStackStep, string> = { ...imageEditStepLabels, metadata: "Remove Metadata" };
const imageEditStepSuffixes: Record<ImageEditStackStep, string> = { crop: "-cropped", resize: "-resized", rotate: "-rotated", tone: "-tone", watermark: "-watermarked", metadata: "-stripped" };

export const presentImageEditSteps = (stack: ImageEditStack) => imageEditStackSteps.filter((step) => stack[step] !== undefined);

export const defaultImageCropStep = (): ImageCropStep => ({
  mode: "aspectRatio", aspectWidth: 4, aspectHeight: 5, anchor: "center", x: 0, y: 0, width: 0, height: 0,
});

export const defaultImageResizeStep = (): ImageResizeStep => ({
  mode: "longestSide", longestSide: 2048, width: 1600, height: 1200, percentage: 50, resampling: "lanczos", keepAspectRatio: true,
});

export const imageRotateDegrees = [0, 90, 180, 270] as const;
export const imageFlips = ["none", "horizontal", "vertical"] as const;

export const defaultImageRotateStep = (): ImageRotateStep => ({ degrees: 90, flip: "none" });

/** The four tone values in the order the step shows them. Each runs from -100 to 100, and 0 leaves it as it is. */
export const imageToneValues = ["brightness", "contrast", "saturation", "exposure"] as const;

export const defaultImageToneStep = (): ImageToneStep => ({ brightness: 0, contrast: 0, saturation: 0, exposure: 0 });

export const isNeutralImageToneStep = (tone: ImageToneStep) => imageToneValues.every((value) => tone[value] === 0);

export const defaultImageWatermarkStep = (): ImageWatermarkStep => ({
  mark: "text", text: "", logoPath: "", size: 30, opacity: 70, color: "#FFFFFF", rotation: 0, placement: "position", anchorX: 1, anchorY: 1,
});

/** The step as the shared watermark options. Colour is sent for text only, since native rejects it for a picture. */
export const imageWatermarkOptions = (watermark: ImageWatermarkStep): WatermarkOptions => ({
  ...(watermark.mark === "text" ? { text: watermark.text, color: watermark.color.toUpperCase() } : { logoPath: watermark.logoPath }),
  size: watermark.size / 100,
  opacity: watermark.opacity,
  rotation: watermark.rotation,
  position: watermark.placement === "tile" ? "tile" : { anchorX: watermark.anchorX, anchorY: watermark.anchorY },
});

/** A single step names its own output; more than one is named -edited. */
export const imageEditStackSuffix = (stack: ImageEditStack) => {
  const steps = presentImageEditSteps(stack);
  return steps.length === 1 ? imageEditStepSuffixes[steps[0]] : "-edited";
};

/** The request's steps, keyed in run order whatever order they were added in. */
export const imageEditStackRequestSteps = (stack: ImageEditStack): ImageEditStackRequest =>
  Object.fromEntries(presentImageEditSteps(stack).map((step) => [step, step === "watermark" ? imageWatermarkOptions(stack.watermark!) : stack[step]]));

/** The same steps as a preview plan, in run order. Removing metadata changes no pixels, so it has no preview. */
export const imageEditStackPreviewPlan = (stack: ImageEditStack): ImageEditPlan => ({
  edits: presentImageEditSteps(stack).filter((step) => step !== "metadata").map((step): ImageEditOperation => step === "crop"
    ? { kind: "crop", ...stack.crop! }
    : step === "resize"
      ? { kind: "resize", ...stack.resize! }
      : step === "rotate"
        ? { kind: "rotate", ...stack.rotate! }
        : step === "tone"
          ? { kind: "tone", ...stack.tone! }
          : { kind: "watermark", ...imageWatermarkOptions(stack.watermark!) }),
});

const MAX_DIMENSION = 16384;
const inRange = (value: number, min: number, max: number) => Number.isInteger(value) && value >= min && value <= max;

/** What to fix before the resize step can run, or null. Native checks the same limits. */
export const imageResizeStepProblem = (resize: ImageResizeStep): { field: string; message: string } | null => {
  if (resize.mode === "percentage") {
    return inRange(resize.percentage, 1, 1000) ? null : { field: "percentage", message: "Enter a percentage from 1 to 1000." };
  }
  if (resize.mode === "longestSide") {
    return inRange(resize.longestSide, 1, MAX_DIMENSION) ? null : { field: "longest side", message: `Enter a longest side from 1 to ${MAX_DIMENSION} pixels.` };
  }
  return inRange(resize.width, 1, MAX_DIMENSION) && inRange(resize.height, 1, MAX_DIMENSION)
    ? null
    : { field: "size", message: `Enter a width and height from 1 to ${MAX_DIMENSION} pixels.` };
};

/** What to fix before the tone step can run, or null. Native checks the same range. */
export const imageToneStepProblem = (tone: ImageToneStep): { field: string; message: string } | null => {
  const field = imageToneValues.find((value) => !inRange(tone[value], -100, 100));
  return field ? { field, message: `Enter a ${field} from −100 to 100.` } : null;
};

/** What to fix before the watermark step can run, or null. Native checks the same limits, and whether the font can draw the text. */
export const imageWatermarkStepProblem = (watermark: ImageWatermarkStep): { field: string; message: string } | null => {
  if (watermark.mark === "text" && watermark.text.trim() === "") return { field: "text", message: "Enter the watermark text." };
  if (watermark.mark === "picture" && watermark.logoPath === "") return { field: "picture", message: "Choose a picture for the watermark." };
  if (!inRange(watermark.size, 1, 100)) return { field: "size", message: "Enter a size from 1 to 100% of the width." };
  if (!inRange(watermark.opacity, 1, 100)) return { field: "opacity", message: "Enter an opacity from 1 to 100%." };
  if (watermark.mark === "text" && !/^#[0-9a-f]{6}$/i.test(watermark.color)) return { field: "colour", message: "Enter a colour such as #FFFFFF." };
  if (!inRange(watermark.rotation, -180, 180)) return { field: "angle", message: "Enter an angle from −180 to 180 degrees." };
  return null;
};

export const createEditHistory = <T>(present: T): EditHistory<T> => ({ past: [], present, future: [] });

export const pushEdit = <T>(history: EditHistory<T>, next: T): EditHistory<T> => ({
  past: [...history.past, history.present],
  present: next,
  future: [],
});

export type ImageCropRect = { x: number; y: number; width: number; height: number };

export const emptyImageEditPlan = (): ImageEditPlan => ({
  edits: [],
});

export const createImageEditHistory = (): ImageEditHistory => ({
  past: [],
  present: emptyImageEditPlan(),
  future: [],
});

export const commitImageEdit = (history: ImageEditHistory, edit: ImageEditOperation): ImageEditHistory => ({
  past: [...history.past, history.present],
  present: { ...history.present, edits: upsertImageEdit(history.present.edits, edit) },
  future: [],
});

export const undoImageEdit = <T>(history: EditHistory<T>): EditHistory<T> => {
  const previous = history.past[history.past.length - 1];
  return previous
    ? { past: history.past.slice(0, -1), present: previous, future: [history.present, ...history.future] }
    : history;
};

export const redoImageEdit = <T>(history: EditHistory<T>): EditHistory<T> => {
  const next = history.future[0];
  return next
    ? { past: [...history.past, history.present], present: next, future: history.future.slice(1) }
    : history;
};

export const resetImageEdits = (): ImageEditHistory => createImageEditHistory();

export const resolveImageCropRect = (
  sourceWidth: number,
  sourceHeight: number,
  crop: ImageCropStep,
): ImageCropRect => {
  if (crop.mode !== "aspectRatio") return { x: crop.x, y: crop.y, width: crop.width, height: crop.height };
  const ratio = crop.aspectWidth / crop.aspectHeight;
  const [width, height] = sourceWidth / sourceHeight > ratio
    ? [Math.round(sourceHeight * ratio), sourceHeight]
    : [sourceWidth, Math.round(sourceWidth / ratio)];
  const maxX = sourceWidth - width;
  const maxY = sourceHeight - height;
  return {
    x: crop.anchor === "custom" ? Math.min(crop.x, maxX) : crop.anchor === "left" || crop.anchor === "start" ? 0 : crop.anchor === "right" || crop.anchor === "end" ? maxX : Math.floor(maxX / 2),
    y: crop.anchor === "custom" ? Math.min(crop.y, maxY) : crop.anchor === "top" || crop.anchor === "start" ? 0 : crop.anchor === "bottom" || crop.anchor === "end" ? maxY : Math.floor(maxY / 2),
    width,
    height,
  };
};

const upsertImageEdit = (edits: ImageEditOperation[], edit: ImageEditOperation): ImageEditOperation[] => {
  if (edit.kind !== "crop") return [...edits, edit];
  const cropIndex = edits.findIndex((existing) => existing.kind === "crop");
  if (cropIndex < 0) return [...edits, edit];
  const nextEdits = edits.filter((existing, index) => existing.kind !== "crop" || index === cropIndex);
  nextEdits[cropIndex] = edit;
  return nextEdits;
};

// Mirrors the native export: the last convert edit sets the format, otherwise the input's format is kept.
export const editedImageExtension = (plan: ImageEditPlan, inputPath: string) => {
  const convert = plan.edits.filter((edit) => edit.kind === "convert").pop();
  if (convert) return convert.format;
  const extension = inputPath.split(".").pop()?.toLowerCase();
  if (extension === "jpg" || extension === "jpeg") return "jpg";
  if (extension === "webp") return "webp";
  if (extension === "heic" || extension === "heif") return "heic";
  return "png";
};

/** The stack's Format choice: keep each input's own format, or save every image as one format. */
export type ImageEditStackFormat = "original" | ConvertImagesRequest["format"];

/** Mirrors the native stack export: a chosen format wins, otherwise the input's format is kept. */
export const imageEditStackExtension = (format: ImageEditStackFormat, inputPath: string) =>
  format === "original" ? editedImageExtension({ edits: [] }, inputPath) : format;

export const planWithDraft = (plan: ImageEditPlan, draft: ImageEditOperation | null): ImageEditPlan => ({
  ...plan,
  edits: draft ? upsertImageEdit(plan.edits, draft) : plan.edits,
});
