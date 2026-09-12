import type { ConvertImagesRequest } from "../../contracts";

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
  | { kind: "watermark"; text: string; opacity: number; x: number; y: number };

export interface ImageEditPlan {
  edits: ImageEditOperation[];
  outputLocation: "alongsideInput";
  suffix: string;
}

export interface ImageEditHistory {
  past: ImageEditPlan[];
  present: ImageEditPlan;
  future: ImageEditPlan[];
}

export const emptyImageEditPlan = (): ImageEditPlan => ({
  edits: [],
  outputLocation: "alongsideInput",
  suffix: "-edited",
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

export const undoImageEdit = (history: ImageEditHistory): ImageEditHistory => {
  const previous = history.past[history.past.length - 1];
  return previous
    ? { past: history.past.slice(0, -1), present: previous, future: [history.present, ...history.future] }
    : history;
};

export const redoImageEdit = (history: ImageEditHistory): ImageEditHistory => {
  const next = history.future[0];
  return next
    ? { past: [...history.past, history.present], present: next, future: history.future.slice(1) }
    : history;
};

export const resetImageEdits = (): ImageEditHistory => createImageEditHistory();

const upsertImageEdit = (edits: ImageEditOperation[], edit: ImageEditOperation): ImageEditOperation[] => {
  if (edit.kind !== "crop") return [...edits, edit];
  const cropIndex = edits.findIndex((existing) => existing.kind === "crop");
  if (cropIndex < 0) return [...edits, edit];
  const nextEdits = edits.filter((existing, index) => existing.kind !== "crop" || index === cropIndex);
  nextEdits[cropIndex] = edit;
  return nextEdits;
};

export const planWithDraft = (plan: ImageEditPlan, draft: ImageEditOperation | null): ImageEditPlan => ({
  ...plan,
  edits: draft ? upsertImageEdit(plan.edits, draft) : plan.edits,
});
