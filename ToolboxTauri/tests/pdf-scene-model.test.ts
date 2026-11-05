import assert from "node:assert/strict";
import test from "node:test";
import {
  composeOrientation, interleaveDuplexScan, mirrorPages, movePagesBefore, movePagesBy, movePagesTo, movePagesToGap, removePages, resetPageRotation, reversePages, rotatePages, sceneFromDocument, swapPage,
} from "../src/features/pdf-editor/scene";
import { parsePageRange } from "../src/shared/workspaceValidation";

test("scene import carries source rotation and page-box baseline", () => {
  const scene = sceneFromDocument({
    path: "/local/annotated.pdf",
    pages: [{ index: 0, width: 300, height: 200, rotation: 90, pageBox: [20, 30, 220, 330] }],
  });

  assert.deepEqual(scene.pages[0], {
    id: "source-0",
    sourceIndex: 0,
    width: 300,
    height: 200,
    rotation: 0,
    mirrored: false,
    crop: null,
    sourceRotation: 90,
    sourceBox: { x: 20, y: 30, width: 200, height: 300 },
    objects: [],
  });
});

const pageIds = (scene: { pages: { id: string }[] }) => scene.pages.map((page) => page.id);
const sixPages = () => sceneFromDocument({
  path: "/local/six.pdf",
  pages: Array.from({ length: 6 }, (_, index) => ({ index, width: 300, height: 400 })),
});

test("moving a multi-selection keeps its relative order before the target", () => {
  const scene = sixPages();
  const moved = movePagesBefore(scene, ["source-4", "source-1"], "source-3");
  assert.deepEqual(pageIds(moved), ["source-0", "source-2", "source-1", "source-4", "source-3", "source-5"]);
  assert.equal(movePagesBefore(scene, ["source-1", "source-3"], "source-3"), scene);
  assert.deepEqual(pageIds(scene), ["source-0", "source-1", "source-2", "source-3", "source-4", "source-5"]);
});

test("removing pages keeps the rest in order and refuses to remove every page", () => {
  const scene = sixPages();
  assert.deepEqual(pageIds(removePages(scene, ["source-1", "source-4"])!), ["source-0", "source-2", "source-3", "source-5"]);
  assert.equal(removePages(scene, pageIds(scene)), null);
});

test("rotation turns only the selected pages and wraps at 360", () => {
  let scene = sixPages();
  for (let turn = 0; turn < 4; turn += 1) scene = rotatePages(scene, ["source-2"]);
  scene = rotatePages(scene, ["source-0", "source-2"]);
  assert.deepEqual(scene.pages.map((page) => page.rotation), [90, 0, 90, 0, 0, 0]);
  assert.equal(rotatePages(scene, ["source-0"], -90).pages[0].rotation, 0);
});

test("rotation by a full turn or of no pages returns the same scene", () => {
  const scene = sixPages();
  assert.equal(rotatePages(scene, []), scene);
  assert.equal(rotatePages(scene, ["source-1"], 360), scene);
  assert.equal(rotatePages(rotatePages(scene, ["source-1"], 180), ["source-1"], 180).pages[1].rotation, 0);
});

test("resetting orientation turns only the chosen pages back and keeps every other edit", () => {
  let scene = rotatePages(sixPages(), ["source-1", "source-2", "source-4"], 90);
  scene = rotatePages(scene, ["source-2"], 90);
  scene = movePagesTo(scene, ["source-4"], 1);
  const reset = resetPageRotation(scene, ["source-2", "source-4", "source-5"]);
  assert.deepEqual(reset.pages.map((page) => [page.id, page.rotation]), [
    ["source-4", 0], ["source-0", 0], ["source-1", 90], ["source-2", 0], ["source-3", 0], ["source-5", 0],
  ]);
  assert.equal(reset.pages[1], scene.pages[1], "an untouched page keeps its identity");
  assert.equal(resetPageRotation(reset, ["source-2", "source-0"]), reset, "nothing to reset returns the same scene");
});

const orientations = [0, 90, 180, 270].flatMap((rotation) => [false, true].map((mirrored) => ({ rotation, mirrored })));
const edits = [{ rotate: 90 }, { rotate: -90 }, { rotate: 180 }, { mirror: "horizontal" as const }, { mirror: "vertical" as const }];
/** Turns a screen point (y down) clockwise by quarter turns. */
const turnPoint = ([x, y]: number[], degrees: number) => {
  for (let turn = 0; turn < ((degrees % 360) + 360) % 360; turn += 90) [x, y] = [-y, x];
  return [x, y];
};
/** Where an orientation sends the point (1, 2), as the thumbnail draws it: flipped left to right, then turned. */
const sendPoint = ({ rotation, mirrored }: { rotation: number; mirrored?: boolean }) => turnPoint([mirrored ? -1 : 1, 2], rotation);

test("orientations compose into one of the eight, matching the geometry of each edit", () => {
  const point = (edit: (typeof edits)[number], [x, y]: number[]) => "rotate" in edit ? turnPoint([x, y], edit.rotate)
    : edit.mirror === "horizontal" ? [-x, y] : [x, -y];
  for (const orientation of orientations) {
    for (const edit of edits) {
      const next = composeOrientation(orientation, edit);
      assert.ok([0, 90, 180, 270].includes(next.rotation), `${JSON.stringify(next)} stays a quarter turn`);
      assert.deepEqual(sendPoint(next), point(edit, sendPoint(orientation)), `${JSON.stringify(orientation)} then ${JSON.stringify(edit)}`);
    }
  }
});

test("a mirror twice on the same axis is the identity, and top to bottom is left to right turned 180°", () => {
  for (const orientation of orientations) {
    for (const mirror of ["horizontal", "vertical"] as const) {
      assert.deepEqual(composeOrientation(composeOrientation(orientation, { mirror }), { mirror }), orientation);
    }
    assert.deepEqual(composeOrientation(orientation, { mirror: "vertical" }),
      composeOrientation(composeOrientation(orientation, { mirror: "horizontal" }), { rotate: 180 }));
  }
  assert.deepEqual(composeOrientation({ rotation: 90 }, { mirror: "horizontal" }), { rotation: 270, mirrored: true });
  assert.deepEqual(composeOrientation({ rotation: 0, mirrored: true }, { rotate: 90 }), { rotation: 90, mirrored: true });
});

test("mirroring flips only the selected pages, keeps their turn and undoes with a second mirror", () => {
  const turned = rotatePages(sixPages(), ["source-1"], 90);
  const scene = mirrorPages(turned, ["source-1", "source-3"], "horizontal");
  assert.deepEqual(scene.pages.map((page) => [page.rotation, page.mirrored]),
    [[0, false], [270, true], [0, false], [0, true], [0, false], [0, false]]);
  assert.equal(scene.pages[0], turned.pages[0], "an untouched page keeps its identity");
  assert.deepEqual(mirrorPages(scene, ["source-1", "source-3"], "horizontal").pages, turned.pages);
  assert.deepEqual(mirrorPages(sixPages(), ["source-2"], "vertical").pages[2], { ...sixPages().pages[2], rotation: 180, mirrored: true });
  assert.equal(mirrorPages(scene, [], "vertical"), scene);
  assert.equal(rotatePages(scene, ["source-3"], 90).pages[3].mirrored, true, "a turn keeps the mirror");
});

test("resetting orientation also clears a mirror", () => {
  const scene = mirrorPages(rotatePages(sixPages(), ["source-0"], 90), ["source-0", "source-1"], "vertical");
  const reset = resetPageRotation(scene, ["source-0", "source-1"]);
  assert.deepEqual(reset.pages.slice(0, 2).map((page) => [page.rotation, page.mirrored]), [[0, false], [0, false]]);
});

test("swapping pages changes only the page order", () => {
  const scene = sixPages();
  assert.deepEqual(pageIds(swapPage(scene, 0, 1)).slice(0, 2), ["source-1", "source-0"]);
  assert.equal(swapPage(scene, 0, -1), scene);
});

const at = (positions: number[]) => positions.map((position) => `source-${position - 1}`);
const positionsOf = (scene: { pages: { id: string }[] }) => scene.pages.map((page) => Number(page.id.slice(7)) + 1);
const pagesOf = (count: number) => sceneFromDocument({
  path: "/local/many.pdf",
  pages: Array.from({ length: count }, (_, index) => ({ index, width: 300, height: 400 })),
});

test("moving to a position puts the block's first page there, keeping the pages together and in order", () => {
  const scene = sixPages();
  assert.deepEqual(positionsOf(movePagesTo(scene, at([4, 5, 6]), 1)), [4, 5, 6, 1, 2, 3]);
  assert.deepEqual(positionsOf(movePagesTo(scene, at([1, 2]), 3)), [3, 4, 1, 2, 5, 6]);
  // Scattered pages gather into one block in their current order.
  assert.deepEqual(positionsOf(movePagesTo(scene, at([5, 2]), 2)), [1, 2, 5, 3, 4, 6]);
  // Positions past the end clamp so the block ends last; below 1 clamps to the start.
  assert.deepEqual(positionsOf(movePagesTo(scene, at([1, 2]), 99)), [3, 4, 5, 6, 1, 2]);
  assert.deepEqual(positionsOf(movePagesTo(scene, at([6]), 0)), [6, 1, 2, 3, 4, 5]);
  // An unchanged order returns the same scene, so no undo step is added.
  assert.equal(movePagesTo(scene, at([1, 2]), 1), scene);
  assert.equal(movePagesTo(scene, [], 3), scene);
  assert.deepEqual(positionsOf(scene), [1, 2, 3, 4, 5, 6]);
});

test("dropping in a gap places the block before the page after that gap", () => {
  const scene = sixPages();
  // Gap 0 is before the first page, gap 6 after the last.
  assert.deepEqual(positionsOf(movePagesToGap(scene, at([2, 3]), 0)), [2, 3, 1, 4, 5, 6]);
  assert.deepEqual(positionsOf(movePagesToGap(scene, at([2, 3]), 6)), [1, 4, 5, 6, 2, 3]);
  assert.deepEqual(positionsOf(movePagesToGap(scene, at([2, 3]), 5)), [1, 4, 5, 2, 3, 6]);
  assert.deepEqual(positionsOf(movePagesToGap(scene, at([1, 5]), 3)), [2, 3, 1, 5, 4, 6]);
  // Gaps next to or inside the block leave a contiguous block where it is.
  for (const gap of [1, 2, 3]) assert.equal(movePagesToGap(scene, at([2, 3]), gap), scene);
});

test("stepping moves the block's leading edge by one position and stops at either end", () => {
  const scene = sixPages();
  assert.deepEqual(positionsOf(movePagesBy(scene, at([3, 4]), -1)), [1, 3, 4, 2, 5, 6]);
  assert.deepEqual(positionsOf(movePagesBy(scene, at([3, 4]), 1)), [1, 2, 5, 3, 4, 6]);
  // A scattered selection gathers next to its first page (earlier) or last page (later).
  assert.deepEqual(positionsOf(movePagesBy(scene, at([2, 5]), -1)), [2, 5, 1, 3, 4, 6]);
  assert.deepEqual(positionsOf(movePagesBy(scene, at([2, 5]), 1)), [1, 3, 4, 6, 2, 5]);
  assert.equal(movePagesBy(scene, at([1, 2]), -1), scene);
  assert.equal(movePagesBy(scene, at([5, 6]), 1), scene);
  // A scattered selection at the start still gathers.
  assert.deepEqual(positionsOf(movePagesBy(scene, at([1, 3]), -1)), [1, 3, 2, 4, 5, 6]);
});

test("reversing flips the selected pages within their own slots", () => {
  const scene = sixPages();
  assert.deepEqual(positionsOf(reversePages(scene, at([2, 3, 4]))), [1, 4, 3, 2, 5, 6]);
  assert.deepEqual(positionsOf(reversePages(scene, at([1, 3, 6]))), [6, 2, 3, 4, 5, 1]);
  assert.deepEqual(positionsOf(reversePages(scene, at([1, 2, 3, 4, 5, 6]))), [6, 5, 4, 3, 2, 1]);
  assert.equal(reversePages(scene, at([4])), scene);
});

test("interleaving a two-sided scan alternates fronts with backs scanned in reverse", () => {
  assert.deepEqual(positionsOf(interleaveDuplexScan(pagesOf(6))), [1, 6, 2, 5, 3, 4]);
  assert.deepEqual(positionsOf(interleaveDuplexScan(pagesOf(5))), [1, 5, 2, 4, 3]);
  const two = pagesOf(2);
  assert.equal(interleaveDuplexScan(two), two);
  assert.deepEqual(positionsOf(interleaveDuplexScan(pagesOf(236))).slice(0, 4), [1, 236, 2, 235]);
});

test("page-order edits keep each page's rotation, crop and objects", () => {
  const scene = rotatePages(sixPages(), ["source-1"]);
  const moved = movePagesTo(reversePages(interleaveDuplexScan(scene), at([1, 2])), at([2]), 6);
  assert.deepEqual(moved.pages.find((page) => page.id === "source-1"), scene.pages[1]);
  assert.equal(moved.pages.length, 6);
});

test("page ranges parse to sorted, unique page indices", () => {
  assert.deepEqual(parsePageRange("1-3, 7, 10-12", 12), [0, 1, 2, 6, 9, 10, 11]);
  assert.deepEqual(parsePageRange("3, 1-3, 3", 5), [0, 1, 2]);
  assert.deepEqual(parsePageRange(" 2 - 4 ,, ", 5), [1, 2, 3]);
  assert.deepEqual(parsePageRange("", 3), [0, 1, 2]);
});

test("page ranges report malformed, reversed and out-of-range input", () => {
  const issue = (value: string, pageCount: number) => {
    const parsed = parsePageRange(value, pageCount);
    assert.ok(!Array.isArray(parsed), `${value} should be rejected`);
    return parsed.message;
  };
  assert.equal(issue("1-3, abc", 10), "Invalid page selection: abc.");
  assert.equal(issue("2-", 10), "Invalid page selection: 2-.");
  assert.equal(issue("0-2", 10), "Page numbers start at 1.");
  assert.equal(issue("5-3", 10), "Write 5-3 from low to high, like 3-5.");
  assert.equal(issue("1-3, 250", 236), "Page 250 isn't in this PDF. It has 236 pages, so use numbers from 1 to 236.");
  assert.equal(issue("2", 1), "Page 2 isn't in this PDF. It has 1 page, so use numbers from 1 to 1.");
});
