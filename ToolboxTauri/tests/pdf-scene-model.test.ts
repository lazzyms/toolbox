import assert from "node:assert/strict";
import test from "node:test";
import { legacyWatermarkPatterns, movePagesBefore, objectsOnPage, pagesInRange, removePages, rotatePages, sceneFromDocument, swapPage, watermarkMarks, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";
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

test("page ranges expand keywords and clamp open and closed ranges to the document", () => {
  assert.deepEqual(pagesInRange("all", 4), [0, 1, 2, 3]);
  assert.deepEqual(pagesInRange("Odd", 5), [0, 2, 4]);
  assert.deepEqual(pagesInRange("even", 5), [1, 3]);
  assert.deepEqual(pagesInRange("even, 1", 4), [0, 1, 3]);
  assert.deepEqual(pagesInRange("5-", 7), [4, 5, 6]);
  assert.deepEqual(pagesInRange("2-9", 3), [1, 2]);
  assert.deepEqual(pagesInRange(" 3 , 1-2, 2 ", 5), [0, 1, 2]);
  assert.throws(() => pagesInRange("5-", 3), { message: "Pages 5- are not in this document. It has 3 pages." });
  assert.throws(() => pagesInRange("even", 1), { message: "Pages even are not in this document. It has 1 page." });
  for (const range of ["", "odd-2", "evens", "3-1", "0", "1,,2", "a-b"]) {
    assert.throws(() => pagesInRange(range, 5), { message: `“${range}” is not a page list. Use page numbers and ranges like 1-3, 7, 10-, or all, odd or even.` });
  }
});

test("range-applied objects are drawn on every page in their range and nowhere else", () => {
  const object = (id: string, pageRange?: string | null) => ({ id, kind: "watermark", pageRange }) as SceneObject;
  const page = (objects: SceneObject[]) => ({ objects }) as PdfScene["pages"][number];
  const scene = { pages: [page([object("own")]), page([object("odd", "odd"), object("broken", "9-1")]), page([])] } as PdfScene;
  const ids = (index: number) => objectsOnPage(scene, index).map(({ object, origin }) => `${object.id}@${origin}`);
  assert.deepEqual(ids(0), ["own@0", "odd@1"]);
  assert.deepEqual(ids(1), []);
  assert.deepEqual(ids(2), ["odd@1"]);
});

test("legacy watermark patterns map to a position and a clockwise rotation", () => {
  assert.deepEqual(legacyWatermarkPatterns, {
    "across-page": { position: "tile", rotation: -35 },
    "bottom-right-to-top-left": { position: "center", rotation: -45 },
    "top-right-to-bottom-left": { position: "center", rotation: 45 },
    "center-horizontal": { position: "center", rotation: 0 },
    "center-vertical": { position: "center", rotation: 90 },
  });
});

test("watermark marks follow the native placement geometry inside the visible area", () => {
  const area = { x: 50, y: 100, width: 500, height: 600 };
  const picture = (position: NonNullable<SceneObject["watermark"]>["position"], rotation = 0) => ({
    watermark: { logoPath: "/local/logo.png", opacity: 50, size: 0.2, position, rotation },
    watermarkImageSize: { width: 200, height: 100 },
  }) as SceneObject;
  assert.deepEqual(watermarkMarks(area, picture("center")).centers, [{ x: 300, y: 400 }]);
  assert.deepEqual(watermarkMarks(area, picture("top-left")).centers, [{ x: 50 + 20 + 100, y: 100 + 20 + 50 }]);
  assert.deepEqual(watermarkMarks(area, picture("bottom-right")).centers, [{ x: 550 - 20 - 100, y: 700 - 20 - 50 }]);
  assert.deepEqual(watermarkMarks(area, picture("top-right", 90)).centers, [{ x: 550 - 20 - 50, y: 100 + 20 + 100 }]);
  assert.deepEqual(watermarkMarks(area, picture({ x: 10, y: 20 })).centers, [{ x: 50 + 10 + 100, y: 100 + 20 + 50 }]);
  const tiles = watermarkMarks(area, picture("tile")).centers;
  assert.ok(tiles.length > 4);
  assert.ok(tiles.some(({ x, y }) => x === 300 && y === 400));
  assert.deepEqual(watermarkMarks(area, { watermark: { logoPath: "/local/logo.png", opacity: 50 }, watermarkImageSize: { width: 900, height: 100 } } as SceneObject), { width: 180, height: 20, rotation: 0, centers: [{ x: 300, y: 400 }] });
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

test("page edits keep the text edits of the pages that stay and drop the rest", () => {
  const edit = (pageIndex: number) => ({ pageIndex, run: 0, original: "Rent £1,450", replacement: "Rent £1,475" });
  const scene = { ...sixPages(), textEdits: [edit(1), edit(2)] };
  assert.deepEqual(removePages(scene, ["source-1"])!.textEdits, [edit(2)]);
  assert.equal("textEdits" in removePages(scene, ["source-1", "source-2"])!, false);
  assert.deepEqual(rotatePages(scene, ["source-1"]).textEdits, scene.textEdits);
  assert.deepEqual(swapPage(scene, 0, 1).textEdits, scene.textEdits);
  assert.deepEqual(movePagesBefore(scene, ["source-2"], "source-0").textEdits, scene.textEdits);
});

test("rotation turns only the selected pages and wraps at 360", () => {
  let scene = sixPages();
  for (let turn = 0; turn < 4; turn += 1) scene = rotatePages(scene, ["source-2"]);
  scene = rotatePages(scene, ["source-0", "source-2"]);
  assert.deepEqual(scene.pages.map((page) => page.rotation), [90, 0, 90, 0, 0, 0]);
  assert.equal(rotatePages(scene, ["source-0"], -90).pages[0].rotation, 0);
});

test("swapping pages changes only the page order", () => {
  const scene = sixPages();
  assert.deepEqual(pageIds(swapPage(scene, 0, 1)).slice(0, 2), ["source-1", "source-0"]);
  assert.equal(swapPage(scene, 0, -1), scene);
});

test("page ranges parse to sorted, unique page indices", () => {
  assert.deepEqual(parsePageRange("1-3, 7, 10-12", 12), [0, 1, 2, 6, 9, 10, 11]);
  assert.deepEqual(parsePageRange("3, 1-3, 3", 5), [0, 1, 2]);
  assert.deepEqual(parsePageRange(" 2 - 4 ", 5), [1, 2, 3]);
  assert.deepEqual(parsePageRange("8-", 10), [7, 8, 9]);
  assert.deepEqual(parsePageRange("", 3), [0, 1, 2]);
});

test("page ranges report malformed, reversed and out-of-range input", () => {
  const issue = (value: string, pageCount: number) => {
    const parsed = parsePageRange(value, pageCount);
    assert.ok(!Array.isArray(parsed), `${value} should be rejected`);
    return parsed.message;
  };
  assert.equal(issue("1-3, abc", 10), "\"abc\" isn't a page or range. Use pages and ranges like 1-3, 5, 8-.");
  assert.equal(issue("2,,4", 10), "Entry 2 of the page range is empty. Remove the extra comma.");
  assert.equal(issue("0-2", 10), "\"0-2\" uses page 0. Page numbers start at 1.");
  assert.equal(issue("5-3", 10), "\"5-3\" runs backwards. Put the lower page first, for example 3-5.");
  assert.equal(issue("1-3, 250", 236), "The PDF has 236 pages, so page 250 can't be used.");
  assert.equal(issue("2", 1), "The PDF has 1 page, so page 2 can't be used.");
});
