import assert from "node:assert/strict";
import test from "node:test";
import {
  clearPageSelection, clickPage, extendPageSelection, orientationBadge, orientationLabel, pageOptionName, rotationAnnouncement, selectAllPages, selectPagesByParity,
  pagePositionsLabel, selectPagesByRange, selectionCountLabel, togglePage,
} from "../src/features/pdf-editor/pageSelection";
import type { PageSelection } from "../src/features/pdf-editor/pageSelection";

const order = ["a", "b", "c", "d", "e", "f", "g"];
const none: PageSelection = { ids: [], anchor: null };

test("a plain click selects only that page and anchors there", () => {
  assert.deepEqual(clickPage(order, { ids: ["a", "c"], anchor: "a" }, "e", {}), { ids: ["e"], anchor: "e" });
});

test("a modifier click toggles one page and keeps document order", () => {
  let selection = clickPage(order, none, "e", { toggle: true });
  selection = clickPage(order, selection, "b", { toggle: true });
  assert.deepEqual(selection, { ids: ["b", "e"], anchor: "b" });
  assert.deepEqual(clickPage(order, selection, "e", { toggle: true }), { ids: ["b"], anchor: "e" });
});

test("a Shift-click selects the run from the anchor, in either direction, and keeps the anchor", () => {
  const anchored: PageSelection = { ids: ["c"], anchor: "c" };
  assert.deepEqual(clickPage(order, anchored, "f", { range: true }), { ids: ["c", "d", "e", "f"], anchor: "c" });
  assert.deepEqual(clickPage(order, anchored, "a", { range: true }), { ids: ["a", "b", "c"], anchor: "c" });
});

test("a Shift-click without an anchor runs from the focused page", () => {
  assert.deepEqual(clickPage(order, none, "d", { range: true, focused: "b" }), { ids: ["b", "c", "d"], anchor: "b" });
  const stale: PageSelection = { ids: [], anchor: "removed" };
  assert.deepEqual(clickPage(order, stale, "b", { range: true, focused: "a" }), { ids: ["a", "b"], anchor: "a" });
});

test("Space toggles the focused page", () => {
  const selection = togglePage(order, { ids: ["a"], anchor: "a" }, "c");
  assert.deepEqual(selection, { ids: ["a", "c"], anchor: "c" });
  assert.deepEqual(togglePage(order, selection, "a"), { ids: ["c"], anchor: "a" });
});

test("Shift+arrow extends from the anchor and can shrink back", () => {
  let selection = extendPageSelection(order, none, "b", "c");
  assert.deepEqual(selection, { ids: ["b", "c"], anchor: "b" });
  selection = extendPageSelection(order, selection, "c", "d");
  assert.deepEqual(selection, { ids: ["b", "c", "d"], anchor: "b" });
  selection = extendPageSelection(order, selection, "d", "a");
  assert.deepEqual(selection, { ids: ["a", "b"], anchor: "b" });
});

test("select all, odd, even and clear work on current positions", () => {
  assert.deepEqual(selectAllPages(order), { ids: order, anchor: null });
  assert.deepEqual(selectPagesByParity(order, "odd").ids, ["a", "c", "e", "g"]);
  assert.deepEqual(selectPagesByParity(order, "even").ids, ["b", "d", "f"]);
  assert.deepEqual(selectPagesByParity(["only"], "even").ids, []);
  assert.deepEqual(clearPageSelection(), none);
});

test("a typed range selects those positions through the shared parser", () => {
  const twelve = Array.from({ length: 12 }, (_, index) => `p${index + 1}`);
  assert.deepEqual(selectPagesByRange(twelve, "1-3, 7, 10-12"), { ids: ["p1", "p2", "p3", "p7", "p10", "p11", "p12"], anchor: null });
  assert.deepEqual(selectPagesByRange(twelve, "3, 1-3"), { ids: ["p1", "p2", "p3"], anchor: null });
});

test("a malformed, reversed, zero or out-of-range typed range returns the parser's message", () => {
  const message = (value: string) => {
    const result = selectPagesByRange(order, value);
    assert.ok(result !== null && "message" in result, `${value} should be rejected`);
    return result.message;
  };
  assert.equal(message("1-3, abc"), '"abc" isn\'t a page or range. Use pages and ranges like 1-3, 5, 8-.');
  assert.equal(message("0"), '"0" uses page 0. Page numbers start at 1.');
  assert.equal(message("5-3"), '"5-3" runs backwards. Put the lower page first, for example 3-5.');
  assert.equal(message("1-3, 250"), "The PDF has 7 pages, so page 250 can't be used.");
});

test("an empty typed range is not a selection", () => {
  assert.equal(selectPagesByRange(order, "   "), null);
  assert.equal(selectPagesByRange(order, " , "), null);
});

test("the count reads naturally for none, one and many", () => {
  assert.equal(selectionCountLabel(0), "No pages selected");
  assert.equal(selectionCountLabel(1), "1 page selected");
  assert.equal(selectionCountLabel(236), "236 pages selected");
});

test("each page is named by position, original page, orientation and selected state", () => {
  assert.equal(pageOptionName(1, { sourceIndex: 0, rotation: 0 }, false), "Position 1, original page 1");
  assert.equal(pageOptionName(2, { sourceIndex: 0, rotation: 90 }, true), "Position 2, original page 1, rotated 90° right, selected");
  assert.equal(pageOptionName(3, { sourceIndex: 4, rotation: 180 }, false), "Position 3, original page 5, rotated 180°");
  assert.equal(pageOptionName(4, { sourceIndex: 3, rotation: 270 }, true), "Position 4, original page 4, rotated 90° left, selected");
  assert.equal(pageOptionName(5, { sourceIndex: null, rotation: 0 }, false), "Position 5");
  assert.equal(pageOptionName(9, { sourceIndex: 8, rotation: 0 }, true, "loading"), "Position 9, original page 9, preview loading, selected");
  assert.equal(pageOptionName(10, { sourceIndex: 9, rotation: 90 }, false, "failed"), "Position 10, original page 10, rotated 90° right, preview unavailable");
});

test("orientation reads the same in the details, on the thumbnail and in announcements", () => {
  assert.deepEqual([0, 90, 180, 270, -90, 450].map(orientationLabel),
    ["As in the original", "Rotated 90° right", "Rotated 180°", "Rotated 90° left", "Rotated 90° left", "Rotated 90° right"]);
  assert.deepEqual([0, 90, 180, 270].map(orientationBadge), [null, "↻ 90°", "↻ 180°", "↺ 90°"]);
  assert.equal(rotationAnnouncement([5], 90), "Page 5 rotated 90° right");
  assert.equal(rotationAnnouncement([4, 5, 6], -90), "Pages 4 to 6 rotated 90° left");
  assert.equal(rotationAnnouncement([2, 7], 180), "Pages 2 and 7 rotated 180°");
});

test("page positions read as a short phrase for announcements", () => {
  assert.equal(pagePositionsLabel([5]), "Page 5");
  assert.equal(pagePositionsLabel([8, 9]), "Pages 8 and 9");
  assert.equal(pagePositionsLabel([4, 5, 6]), "Pages 4 to 6");
  assert.equal(pagePositionsLabel([2, 7]), "Pages 2 and 7");
  assert.equal(pagePositionsLabel([1, 2, 3, 7, 9]), "Pages 1 to 3, 7 and 9");
  assert.equal(pagePositionsLabel([9, 1, 8]), "Pages 1, 8 and 9");
});
