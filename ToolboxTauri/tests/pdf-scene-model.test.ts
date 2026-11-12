import assert from "node:assert/strict";
import test from "node:test";
import { headerFooterOverlaps, headerFooterParts, headerFooterVariant, isRangedKind, legacyWatermarkPatterns, movePagesBefore, objectsOnPage, pageNumberRect, pageNumberText, pagesInRange, removePages, resolvePageTokens, restorePages, rotatePages, sceneFromDocument, swapPage, tokenDate, watermarkMarks, type HeaderFooterOptions, type HeaderFooterText, type PageNumberFormat, type PdfScene, type SceneObject, interleaveDuplexScan, movePagesBy, movePagesTo, movePagesToGap, reversePages, resetPageRotation, composeOrientation, mirrorPages } from "../src/features/pdf-editor/scene";
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
  // Anchors 0 and 1 land on the corner placements, and 0.5 on the centre.
  assert.deepEqual(watermarkMarks(area, picture({ anchorX: 0, anchorY: 0 })).centers, watermarkMarks(area, picture("top-left")).centers);
  assert.deepEqual(watermarkMarks(area, picture({ anchorX: 1, anchorY: 1 })).centers, watermarkMarks(area, picture("bottom-right")).centers);
  assert.deepEqual(watermarkMarks(area, picture({ anchorX: 0.5, anchorY: 0.5 })).centers, [{ x: 300, y: 400 }]);
  const tiles = watermarkMarks(area, picture("tile")).centers;
  assert.ok(tiles.length > 4);
  assert.ok(tiles.some(({ x, y }) => x === 300 && y === 400));
  assert.deepEqual(watermarkMarks(area, { watermark: { logoPath: "/local/logo.png", opacity: 50 }, watermarkImageSize: { width: 900, height: 100 } } as SceneObject), { width: 180, height: 20, rotation: 0, centers: [{ x: 300, y: 400 }] });
});

const pageNumbers = (format: PageNumberFormat, start = 1, pageRange = "all"): SceneObject => ({
  id: "numbers", kind: "pageNumbers", rect: { x: 0, y: 0, width: 612, height: 792 }, text: "", fontSize: 10, color: "#000000", opacity: 1,
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
  id: "hf", kind: "headerFooter", rect: { x: 0, y: 0, width: 612, height: 792 }, text: "", fontSize: 9, color: "#555555", opacity: 1, pageRange,
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

test("removing the page that stores a page-number style keeps the style on the first remaining page", () => {
  const page = (id: string, objects: SceneObject[] = []) => ({ id, sourceIndex: Number(id), width: 612, height: 792, rotation: 0, crop: null, sourceRotation: 0, sourceBox: null, objects });
  const style = pageNumbers("number", 1, "all");
  const next = removePages({ pages: [page("0", [style]), page("1"), page("2")] }, ["0"])!;
  assert.deepEqual(next.pages.map((item) => item.id), ["1", "2"]);
  assert.deepEqual(next.pages[0].objects, [style]);
  assert.deepEqual(next.pages[1].objects, []);
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

test("restoring removed pages puts them back where they were, as opened, even after other pages moved", () => {
  const opened = sixPages();
  const edited = { pages: opened.pages.map((page) => page.id === "source-3" ? { ...page, rotation: 90 } : page) };
  const removed = removePages(edited, ["source-1", "source-3", "source-4"])!;
  const moved = movePagesTo(removed, ["source-5"], 0);
  const restored = restorePages(moved, opened, [4, 3]);
  assert.deepEqual(pageIds(restored), ["source-5", "source-0", "source-2", "source-3", "source-4"]);
  assert.equal(restored.pages.find((page) => page.id === "source-3")!.rotation, 0);
  assert.deepEqual(pageIds(restorePages(removed, opened, [5, 0, 1])), ["source-0", "source-1", "source-2", "source-5"]);
  assert.equal(restorePages(removed, opened, [0, 2]), removed);
});

test("restoring pages keeps the scene's text edits", () => {
  const edit = { pageIndex: 0, run: 0, original: "Rent £1,450", replacement: "Rent £1,475" };
  const opened = sixPages();
  const removed = removePages({ ...opened, textEdits: [edit] }, ["source-3"])!;
  assert.deepEqual(restorePages(removed, opened, [3]).textEdits, [edit]);
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
