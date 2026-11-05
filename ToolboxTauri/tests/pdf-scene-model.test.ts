import assert from "node:assert/strict";
import test from "node:test";
import { headerFooterOverlaps, headerFooterParts, headerFooterVariant, isRangedKind, legacyWatermarkPatterns, objectsOnPage, pageNumberRect, pageNumberText, pagesInRange, resolvePageTokens, sceneFromDocument, tokenDate, watermarkMarks, type HeaderFooterOptions, type HeaderFooterText, type PageNumberFormat, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";

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

const slots = (left = "", center = "", right = "") => ({ left, center, right });
const texts = (header: ReturnType<typeof slots>, footer: ReturnType<typeof slots>): HeaderFooterText => ({ header, footer });
const headerFooter = (options: Partial<HeaderFooterOptions> = {}, pageRange = "all"): SceneObject => ({
  id: "hf", kind: "headerFooter", rect: { x: 0, y: 0, width: 612, height: 792 }, text: "", fontSize: 9, color: "#555555", opacity: 1, strokes: [], pageRange,
  headerFooter: { ...texts(slots("{file}", "", "{date}"), slots("", "{page} of {pages}", "")), firstPage: null, even: null, date: "2026-10-13", fileName: "report.pdf", margin: 36, ...options },
});
const area = { x: 0, y: 0, width: 612, height: 792 };
const resolved = (object: SceneObject, pageIndex: number, pageCount: number) =>
  Object.fromEntries(headerFooterParts(area, object, pageIndex, pageCount).filter(({ text }) => text).map(({ band, slot, text }) => [`${band} ${slot}`, text]));

test("every page token resolves once, including the fixed date and file name", () => {
  const tokens = { page: 4, pages: 12, date: "2026-10-13", file: "{page} notes.pdf" };
  assert.equal(resolvePageTokens("{page}", tokens), "4");
  assert.equal(resolvePageTokens("{pages}", tokens), "12");
  assert.equal(resolvePageTokens("{date}", tokens), "13 October 2026");
  assert.equal(resolvePageTokens("{file}", tokens), "{page} notes.pdf");
  assert.equal(resolvePageTokens("{date} {file} {x}", { page: 1, pages: 1 }), "{date} {file} {x}");
  assert.equal(tokenDate("2024-02-29"), "29 February 2024");
  for (const bad of ["2023-02-29", "2026-13-01", "2026-04-31", "2026-1-01", ""]) assert.equal(tokenDate(bad), null, bad);
  assert.equal(resolvePageTokens("{date}", { page: 1, pages: 1, date: "not a date" }), "");
});

test("header and footer variants pick the first page of the range, then even pages, then the main text", () => {
  const first = texts(slots("Cover"), slots("", "First"));
  const even = texts(slots("", "", "Even {page}"), slots("{file}"));
  const options = headerFooter({ firstPage: first, even }).headerFooter!;
  assert.equal(headerFooterVariant(options, 0, 0), first);
  assert.equal(headerFooterVariant(options, 1, 0), even);
  assert.deepEqual(headerFooterVariant(options, 2, 0), texts(options.header, options.footer));
  assert.equal(headerFooterVariant({ ...options, firstPage: null }, 0, 0).header.left, "{file}");
  assert.equal(headerFooterVariant({ ...options, even: null }, 1, 0).header.left, "{file}");
  assert.equal(headerFooterVariant(options, 3, 3), first, "an odd-numbered first page of the range still uses the first-page text");
});

test("header and footer text follows the final page order, first-page and even variants", () => {
  const object = headerFooter({ firstPage: texts(slots("Cover"), slots("", "First")), even: texts(slots("", "", "Even {page}"), slots("{file}")) });
  assert.deepEqual(resolved(object, 0, 3), { "header left": "Cover", "footer center": "First" });
  assert.deepEqual(resolved(object, 1, 3), { "header right": "Even 2", "footer left": "report.pdf" });
  assert.deepEqual(resolved(object, 2, 3), { "header left": "report.pdf", "header right": "13 October 2026", "footer center": "3 of 3" });
  const skipFirst = headerFooter({ firstPage: texts(slots("Start"), slots()) }, "2-");
  assert.deepEqual(resolved(skipFirst, 1, 3), { "header left": "Start" });
  const page = (id: string, objects: SceneObject[] = []) => ({ id, sourceIndex: Number(id), width: 612, height: 792, rotation: 0, crop: null, sourceRotation: 0, sourceBox: null, objects });
  const before: PdfScene = { pages: [page("0", [object]), page("1"), page("2")] };
  const reordered: PdfScene = { pages: [before.pages[1], before.pages[2], before.pages[0]] };
  assert.deepEqual(objectsOnPage(reordered, 2).map(({ object: item }) => resolved(item, 2, 3)["footer center"]), ["3 of 3"]);
  assert.ok(isRangedKind("headerFooter") && isRangedKind("pageNumbers") && !isRangedKind("text"));
});

test("header and footer slots split each band into thirds and report overlap with page text", () => {
  const parts = headerFooterParts(area, headerFooter(), 0, 1);
  assert.deepEqual(parts.map(({ band, slot, rect }) => [band, slot, rect]), [
    ["header", "left", { x: 36, y: 36, width: 180, height: 9 * 1.2 }], ["header", "center", { x: 216, y: 36, width: 180, height: 9 * 1.2 }],
    ["header", "right", { x: 396, y: 36, width: 180, height: 9 * 1.2 }], ["footer", "left", { x: 36, y: 792 - 36 - 9 * 1.2, width: 180, height: 9 * 1.2 }],
    ["footer", "center", { x: 216, y: 792 - 36 - 9 * 1.2, width: 180, height: 9 * 1.2 }], ["footer", "right", { x: 396, y: 792 - 36 - 9 * 1.2, width: 180, height: 9 * 1.2 }],
  ]);
  assert.equal(headerFooterOverlaps(parts, 9, [{ x: 40, y: 38, width: 50, height: 10 }]), true);
  assert.equal(headerFooterOverlaps(parts, 9, [{ x: 40, y: 100, width: 300, height: 10 }]), false);
  assert.equal(headerFooterOverlaps(parts, 9, [{ x: 220, y: 38, width: 20, height: 10 }]), false, "the empty header centre does not collide");
  assert.equal(headerFooterOverlaps(parts, 9, [{ x: 290, y: 740, width: 20, height: 10 }]), true, "the centred footer text collides");
});
