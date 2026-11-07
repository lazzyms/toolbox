import type { ConvertImagesRequest, OutputLocation, WatermarkOptions } from "../../contracts";

export type ImageEditOperation =
  | { kind: "convert"; format: ConvertImagesRequest["format"] }
  | { kind: "compress"; quality: number; lossless: boolean }
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

/** The shared edit stack holds at most one of each step. Native runs them in a fixed order, so the stack has no order of its own. */
export interface ImageEditStack {
  crop?: ImageCropStep;
}

export const imageEditStackSteps = ["crop"] as const;
export type ImageEditStackStep = (typeof imageEditStackSteps)[number];

export const presentImageEditSteps = (stack: ImageEditStack) => imageEditStackSteps.filter((step) => stack[step] !== undefined);

export const defaultImageCropStep = (): ImageCropStep => ({
  mode: "aspectRatio", aspectWidth: 4, aspectHeight: 5, anchor: "center", x: 0, y: 0, width: 0, height: 0,
});

// A single step names its own output; #513 adds "-edited" for more than one.
export const imageEditStackSuffix = (stack: ImageEditStack) => (stack.crop ? "-cropped" : "-edited");

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

export const planWithDraft = (plan: ImageEditPlan, draft: ImageEditOperation | null): ImageEditPlan => ({
  ...plan,
  edits: draft ? upsertImageEdit(plan.edits, draft) : plan.edits,
});
