import assert from "node:assert/strict";
import test from "node:test";
import { insertPageAfter, movePagesBefore, removePages, rotatePages, sceneFromDocument, swapPage } from "../src/features/pdf-editor/scene";
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

test("swapping and inserting pages change only the page order", () => {
  const scene = sixPages();
  assert.deepEqual(pageIds(swapPage(scene, 0, 1)).slice(0, 2), ["source-1", "source-0"]);
  assert.equal(swapPage(scene, 0, -1), scene);
  const blank = { ...scene.pages[0], id: "blank", sourceIndex: null };
  assert.deepEqual(pageIds(insertPageAfter(scene, 1, blank)).slice(0, 4), ["source-0", "source-1", "blank", "source-2"]);
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
