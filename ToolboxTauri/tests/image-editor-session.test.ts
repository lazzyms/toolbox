import assert from "node:assert/strict";
import test from "node:test";
import {
  commitImageEdit,
  createImageEditHistory,
  planWithDraft,
  redoImageEdit,
  resolveImageCropRect,
  type ImageEditOperation,
  resetImageEdits,
  undoImageEdit,
} from "../src/features/image-editor/session";

const crop = (x: number, y: number, width: number, height: number): ImageEditOperation => ({
  kind: "crop",
  x,
  y,
  width,
  height,
  mode: "rectangle",
  aspectWidth: width,
  aspectHeight: height,
  anchor: "center",
});

test("coalesces source-space crop selections without moving later edits", () => {
  const firstSelection = crop(0, 0, 2, 3);
  const secondSelection = crop(7, 5, 3, 2);
  const rotate: ImageEditOperation = { kind: "rotate", degrees: 90, flip: "none" };
  const history = commitImageEdit(
    commitImageEdit(createImageEditHistory(), firstSelection),
    rotate,
  );

  assert.deepEqual(planWithDraft(history.present, secondSelection).edits, [secondSelection, rotate]);
  assert.deepEqual(commitImageEdit(history, secondSelection).present.edits, [secondSelection, rotate]);
});

test("resolves aspect crop geometry from source dimensions and anchor", () => {
  assert.deepEqual(resolveImageCropRect(640, 480, {
    ...crop(0, 0, 1, 1),
    mode: "aspectRatio",
    aspectWidth: 1,
    aspectHeight: 1,
    anchor: "right",
  }), { x: 160, y: 0, width: 480, height: 480 });
});

test("appends edits with predictable undo, redo, reset, and branch behavior", () => {
  const rotate: ImageEditOperation = { kind: "rotate", degrees: 90, flip: "none" };
  const tone: ImageEditOperation = { kind: "tone", brightness: 10, contrast: 20, saturation: 0, exposure: 0 };
  const replacement: ImageEditOperation = { kind: "rotate", degrees: 180, flip: "none" };
  const initial = createImageEditHistory();
  const one = commitImageEdit(initial, rotate);
  const two = commitImageEdit(one, tone);

  assert.deepEqual(one.present.edits, [rotate]);
  assert.deepEqual(two.present.edits, [rotate, tone]);
  assert.deepEqual(two.past.map((plan) => plan.edits), [[], [rotate]]);

  const undone = undoImageEdit(two);
  assert.deepEqual(undone.present.edits, [rotate]);
  assert.deepEqual(undone.future.map((plan) => plan.edits), [[rotate, tone]]);

  const redone = redoImageEdit(undone);
  assert.deepEqual(redone.present.edits, [rotate, tone]);
  assert.deepEqual(redone.future, []);

  const branched = commitImageEdit(undone, replacement);
  assert.deepEqual(branched.present.edits, [rotate, replacement]);
  assert.deepEqual(branched.future, []);
  assert.deepEqual(resetImageEdits(), createImageEditHistory());
  assert.deepEqual(initial.present.edits, []);
  assert.deepEqual(one.present.edits, [rotate]);
  assert.deepEqual(two.present.edits, [rotate, tone]);
});
