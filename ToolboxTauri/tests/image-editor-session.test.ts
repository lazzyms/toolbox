import assert from "node:assert/strict";
import test from "node:test";
import {
  commitImageEdit,
  createImageEditHistory,
  planWithDraft,
  resolveImageCropRect,
  type ImageEditOperation,
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
