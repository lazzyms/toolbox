import assert from "node:assert/strict";
import test from "node:test";
import { legacyWatermarkPatterns, objectsOnPage, pageNumberRect, pageNumberText, pagesInRange, resolvePageTokens, sceneFromDocument, watermarkMarks, type PageNumberFormat, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";

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

const pageNumbers = (format: PageNumberFormat, start = 1, pageRange = "all"): SceneObject => ({
  id: "numbers", kind: "pageNumbers", rect: { x: 0, y: 0, width: 612, height: 792 }, text: "", fontSize: 10, color: "#000000", opacity: 1, strokes: [],
  pageRange, pageNumbers: { format, start, position: "bottom-center", margin: 36 },
});

test("page tokens resolve every format against the page position and count", () => {
  assert.equal(resolvePageTokens("Page {page} of {pages} {pagesx}", { page: 2, pages: 9 }), "Page 2 of 9 {pagesx}");
  const formats: [PageNumberFormat, string][] = [["number", "3"], ["page", "Page 3"], ["page-of-total", "Page 3 of 5"], ["number-of-total", "3 / 5"], ["dashes", "- 3 -"]];
  for (const [format, expected] of formats) assert.equal(pageNumberText(pageNumbers(format), 2, 5), expected);
  assert.equal(pageNumberText(pageNumbers("page-of-total", 10), 0, 5), "Page 10 of 14");
});

test("page numbers follow the final page order after a reorder", () => {
  const page = (id: string, objects: SceneObject[] = []) => ({ id, sourceIndex: Number(id), width: 612, height: 792, rotation: 0, crop: null, sourceRotation: 0, sourceBox: null, objects });
  const style = pageNumbers("page-of-total", 1, "2-");
  const before: PdfScene = { pages: [page("0", [style]), page("1"), page("2")] };
  const numbered = (scene: PdfScene) => scene.pages.map((item, index) => [item.id, objectsOnPage(scene, index).map(({ object }) => pageNumberText(object, index, scene.pages.length))]);
  assert.deepEqual(numbered(before), [["0", []], ["1", ["Page 2 of 3"]], ["2", ["Page 3 of 3"]]]);
  const reordered: PdfScene = { pages: [before.pages[2], before.pages[0], before.pages[1]] };
  assert.deepEqual(numbered(reordered), [["2", []], ["0", ["Page 2 of 3"]], ["1", ["Page 3 of 3"]]]);
});

test("page-number placement mirrors the native corners, centres and margin", () => {
  const area = { x: 10, y: 20, width: 500, height: 600 };
  const object = pageNumbers("number");
  const at = (position: NonNullable<SceneObject["pageNumbers"]>["position"]) => pageNumberRect(area, { ...object, pageNumbers: { ...object.pageNumbers!, position } }, "12");
  assert.deepEqual(at("top-left"), { x: 46, y: 56, width: 11, height: 12 });
  assert.deepEqual(at("bottom-right"), { x: 10 + 500 - 36 - 11, y: 20 + 600 - 36 - 12, width: 11, height: 12 });
  assert.deepEqual(at("middle-center"), { x: 10 + (500 - 11) / 2, y: 20 + (600 - 12) / 2, width: 11, height: 12 });
});
