import assert from "node:assert/strict";
import test from "node:test";
import {
  commitImageEdit,
  createEditHistory,
  createImageEditHistory,
  defaultImageCropStep,
  defaultImageResizeStep,
  defaultImageRotateStep,
  defaultImageToneStep,
  imageToneStepProblem,
  isNeutralImageToneStep,
  imageEditStackPreviewPlan,
  imageEditStackExtension,
  imageEditStackRequestSteps,
  imageEditStackSuffix,
  imageResizeStepProblem,
  presentImageEditSteps,
  pushEdit,
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

test("the edit stack history pushes, undoes and redoes whole stacks", () => {
  const cropStep = defaultImageCropStep();
  const added = pushEdit(createEditHistory<{ crop?: typeof cropStep }>({}), { crop: cropStep });
  const moved = pushEdit(added, { crop: { ...cropStep, anchor: "start" } });
  assert.deepEqual(moved.past, [{}, { crop: cropStep }]);
  const undone = undoImageEdit(moved);
  assert.deepEqual(undone.present, { crop: cropStep });
  assert.deepEqual(redoImageEdit(undone).present.crop?.anchor, "start");
  assert.deepEqual(pushEdit(undone, {}).future, [], "a new edit clears redo");
  assert.equal(undoImageEdit(createEditHistory({})).past.length, 0);
});

test("the stack lists its steps and names a crop-only run -cropped", () => {
  assert.deepEqual(presentImageEditSteps({}), []);
  assert.deepEqual(presentImageEditSteps({ crop: defaultImageCropStep() }), ["crop"]);
  assert.equal(imageEditStackSuffix({ crop: defaultImageCropStep() }), "-cropped");
  assert.deepEqual(defaultImageCropStep(), { mode: "aspectRatio", aspectWidth: 4, aspectHeight: 5, anchor: "center", x: 0, y: 0, width: 0, height: 0 });
});

test("a stack keeps the fixed run order and names more than one step -edited", () => {
  const crop = defaultImageCropStep();
  const resize = defaultImageResizeStep();
  assert.equal(imageEditStackSuffix({ resize }), "-resized");
  assert.equal(imageEditStackSuffix({ resize, crop }), "-edited");
  assert.deepEqual(presentImageEditSteps({ resize, crop }), ["crop", "resize"]);
  assert.deepEqual(Object.keys(imageEditStackRequestSteps({ resize, crop })), ["crop", "resize"], "resize added first is still sent after crop");
  assert.deepEqual(Object.keys(imageEditStackRequestSteps({ resize, crop: undefined })), ["resize"], "a removed step is not sent");
  assert.deepEqual(imageEditStackPreviewPlan({ resize, crop }).edits.map(({ kind }) => kind), ["crop", "resize"]);
});

test("Rotate and Flip is one step that runs last and is named -rotated on its own", () => {
  const rotate = defaultImageRotateStep();
  assert.deepEqual(rotate, { degrees: 90, flip: "none" });
  assert.equal(imageEditStackSuffix({ rotate }), "-rotated");
  assert.equal(imageEditStackSuffix({ rotate, crop: defaultImageCropStep() }), "-edited");
  const stack = { rotate: { degrees: 270, flip: "horizontal" }, resize: defaultImageResizeStep(), crop: defaultImageCropStep() };
  assert.deepEqual(presentImageEditSteps(stack), ["crop", "resize", "rotate"]);
  assert.deepEqual(Object.keys(imageEditStackRequestSteps(stack)), ["crop", "resize", "rotate"], "rotate added first is still sent last");
  assert.deepEqual(imageEditStackRequestSteps({ rotate: { degrees: 180, flip: "vertical" } }), { rotate: { degrees: 180, flip: "vertical" } });
  assert.deepEqual(imageEditStackPreviewPlan(stack).edits.at(-1), { kind: "rotate", degrees: 270, flip: "horizontal" });
});

test("Colour and Tone is one step that runs after Rotate and Flip and is named -tone on its own", () => {
  const tone = defaultImageToneStep();
  assert.deepEqual(tone, { brightness: 0, contrast: 0, saturation: 0, exposure: 0 });
  assert.equal(isNeutralImageToneStep(tone), true);
  assert.equal(isNeutralImageToneStep({ ...tone, exposure: -1 }), false);
  assert.equal(imageEditStackSuffix({ tone }), "-tone");
  assert.equal(imageEditStackSuffix({ tone, rotate: defaultImageRotateStep() }), "-edited");
  const adjusted = { brightness: 10, contrast: 5, saturation: -20, exposure: 0 };
  const stack = { tone: adjusted, rotate: defaultImageRotateStep(), crop: defaultImageCropStep() };
  assert.deepEqual(presentImageEditSteps(stack), ["crop", "rotate", "tone"]);
  assert.deepEqual(Object.keys(imageEditStackRequestSteps(stack)), ["crop", "rotate", "tone"], "tone added first is still sent after rotate");
  assert.deepEqual(imageEditStackRequestSteps({ tone: adjusted }), { tone: adjusted });
  assert.deepEqual(imageEditStackPreviewPlan(stack).edits.at(-1), { kind: "tone", ...adjusted });
});

test("tone values are checked against the native range of -100 to 100", () => {
  assert.equal(imageToneStepProblem({ brightness: -100, contrast: 100, saturation: 0, exposure: 0 }), null);
  assert.deepEqual(imageToneStepProblem({ brightness: 0, contrast: 101, saturation: 0, exposure: 0 }), { field: "contrast", message: "Enter a contrast from −100 to 100." });
  assert.deepEqual(imageToneStepProblem({ brightness: 0, contrast: 0, saturation: -101, exposure: 0 }), { field: "saturation", message: "Enter a saturation from −100 to 100." });
  assert.equal(imageToneStepProblem({ brightness: 0, contrast: 0, saturation: 0, exposure: Number.NaN })?.field, "exposure");
});

test("resize steps are checked against the native limits", () => {
  const resize = defaultImageResizeStep();
  assert.deepEqual(resize, { mode: "longestSide", longestSide: 2048, width: 1600, height: 1200, percentage: 50, resampling: "lanczos", keepAspectRatio: true });
  assert.equal(imageResizeStepProblem(resize), null);
  assert.equal(imageResizeStepProblem({ ...resize, longestSide: 16385 })?.field, "longest side");
  assert.equal(imageResizeStepProblem({ ...resize, mode: "percentage", percentage: 1000 }), null);
  assert.equal(imageResizeStepProblem({ ...resize, mode: "percentage", percentage: 1001 })?.message, "Enter a percentage from 1 to 1000.");
  assert.equal(imageResizeStepProblem({ ...resize, mode: "percentage", percentage: 0 })?.field, "percentage");
  assert.equal(imageResizeStepProblem({ ...resize, mode: "exact", width: 0 })?.field, "size");
  assert.equal(imageResizeStepProblem({ ...resize, mode: "exact", height: 16385 })?.field, "size");
  assert.equal(imageResizeStepProblem({ ...resize, mode: "exact", width: 1.5 })?.field, "size");
});

test("aspect crops keep the start or end of the long side", () => {
  const step = { ...defaultImageCropStep(), aspectWidth: 1, aspectHeight: 1 };
  assert.deepEqual(resolveImageCropRect(300, 100, { ...step, anchor: "start" }), { x: 0, y: 0, width: 100, height: 100 });
  assert.deepEqual(resolveImageCropRect(300, 100, { ...step, anchor: "end" }), { x: 200, y: 0, width: 100, height: 100 });
  assert.deepEqual(resolveImageCropRect(100, 300, { ...step, anchor: "end" }), { x: 0, y: 200, width: 100, height: 100 });
  assert.deepEqual(resolveImageCropRect(100, 300, { ...step, anchor: "center" }), { x: 0, y: 100, width: 100, height: 100 });
});

test("the stack's output extension follows the chosen format, or each input's own", () => {
  assert.equal(imageEditStackExtension("original", "/photos/a.JPEG"), "jpg");
  assert.equal(imageEditStackExtension("original", "/photos/b.webp"), "webp");
  assert.equal(imageEditStackExtension("original", "/photos/c.heif"), "heic");
  assert.equal(imageEditStackExtension("original", "/photos/d.tiff"), "png");
  assert.equal(imageEditStackExtension("jpg", "/photos/badge.png"), "jpg");
  assert.equal(imageEditStackExtension("heic", "/photos/b.webp"), "heic");
});
