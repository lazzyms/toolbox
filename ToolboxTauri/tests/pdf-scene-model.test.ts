import assert from "node:assert/strict";
import test from "node:test";
import { legacyWatermarkPatterns, objectsOnPage, pagesInRange, sceneFromDocument, watermarkMarks, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";

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
