import assert from "node:assert/strict";
import test from "node:test";
import {
  BLANK_PAGE_BATCH_SIZE, blankPagesAnnouncement, blankPagesFailedAnnouncement, blankPagesFoundLabel, blankPagesRunningAnnouncement,
  blankScenePageIds, createBlankPageFinder,
} from "../src/features/pdf-editor/blankPages";
import type { BlankPageClassification } from "../src/features/pdf-editor/blankPages";
import { pageOptionName, pagePositionsLabel, positionsList } from "../src/features/pdf-editor/pageSelection";

/** A fake helper: `blank` pages are blank, `broken` pages come back with a note, the rest are not blank. */
const fakeHelper = ({ blank = [], broken = [] }: { blank?: number[]; broken?: number[] }) => {
  const requests: number[][] = [];
  const fetch = async (pageIndices: number[]): Promise<BlankPageClassification[]> => {
    requests.push(pageIndices);
    return pageIndices.map((pageIndex) => broken.includes(pageIndex)
      ? { pageIndex, blank: false, note: "Couldn't check this page, so it isn't counted as blank. pdftoppm could not render this PDF" }
      : { pageIndex, blank: blank.includes(pageIndex), note: null });
  };
  return { requests, fetch };
};

test("the batch size fits the native limit", () => {
  assert.equal(BLANK_PAGE_BATCH_SIZE, 12);
});

test("source pages are checked in batches, once each, and only blank ones are reported", async () => {
  const helper = fakeHelper({ blank: [6, 10, 15] });
  const finder = createBlankPageFinder({ fetch: helper.fetch, batchSize: 12 });
  const sources = Array.from({ length: 20 }, (_, index) => index);
  assert.deepEqual(await finder.find([...sources, 6]), { blank: [6, 10, 15], unchecked: [] });
  assert.deepEqual(helper.requests, [sources.slice(0, 12), sources.slice(12)]);
  assert.deepEqual(await finder.find(sources), { blank: [6, 10, 15], unchecked: [] });
  assert.equal(helper.requests.length, 2, "answers are reused for the same document");
});

test("a page that couldn't be checked is reported apart, never as blank, and is tried again next time", async () => {
  const helper = fakeHelper({ blank: [0, 2], broken: [2, 3] });
  const finder = createBlankPageFinder({ fetch: helper.fetch });
  assert.deepEqual(await finder.find([0, 1, 2, 3]), { blank: [0], unchecked: [2, 3] });
  await finder.find([0, 1, 2, 3]);
  assert.deepEqual(helper.requests, [[0, 1, 2, 3], [2, 3]]);
});

test("a page the helper leaves out counts as unchecked", async () => {
  const finder = createBlankPageFinder({ fetch: async () => [{ pageIndex: 0, blank: true, note: null }] });
  assert.deepEqual(await finder.find([0, 1]), { blank: [0], unchecked: [1] });
});

test("disposing cancels: later batches are never sent and the result is dropped", async () => {
  const requests: number[][] = [];
  let release!: () => void;
  const finder = createBlankPageFinder({
    batchSize: 2,
    fetch: (pageIndices) => { requests.push(pageIndices); return new Promise((resolve) => { release = () => resolve(pageIndices.map((pageIndex) => ({ pageIndex, blank: true, note: null }))); }); },
  });
  const pending = finder.find([0, 1, 2, 3]);
  finder.dispose();
  release();
  assert.equal(await pending, null);
  assert.deepEqual(requests, [[0, 1]]);
});

test("a helper failure rejects, unless the finder was disposed first", async () => {
  const failing = createBlankPageFinder({ fetch: async () => { throw new Error("pdftotext is required to find blank pages."); } });
  await assert.rejects(failing.find([0]), /pdftotext/);
  let reject!: (error: Error) => void;
  const cancelled = createBlankPageFinder({ fetch: () => new Promise((_, fail) => { reject = fail; }) });
  const pending = cancelled.find([0]);
  cancelled.dispose();
  reject(new Error("late"));
  assert.equal(await pending, null);
});

test("every copy of a blank source page is blank, but not one with added content or no source", () => {
  const page = (id: string, sourceIndex: number | null, objects: unknown[] = []) => ({ id, sourceIndex, objects }) as never;
  const pages = [page("a", 0), page("b", 1), page("c", 0), page("d", 2, [{}]), page("e", null)];
  assert.deepEqual(blankScenePageIds(pages, [0, 2]), ["a", "c"]);
  assert.deepEqual(blankScenePageIds(pages, []), []);
});

test("the announcement names the count and positions, and always says nothing was removed", () => {
  assert.equal(blankPagesAnnouncement([7, 11, 16]), "3 blank pages found and selected: 7, 11 and 16. Nothing was removed.");
  assert.equal(blankPagesAnnouncement([4]), "1 blank page found and selected: 4. Nothing was removed.");
  assert.equal(blankPagesAnnouncement([2, 3, 4, 9]), "4 blank pages found and selected: 2 to 4 and 9. Nothing was removed.");
  assert.equal(blankPagesAnnouncement([]), "No blank pages found. Nothing was removed.");
  assert.equal(blankPagesAnnouncement([1], [5]), "1 blank page found and selected: 1. Page 5 couldn't be checked, so it wasn't selected. Nothing was removed.");
  assert.equal(blankPagesAnnouncement([], [2, 3]), "No blank pages found. Pages 2 and 3 couldn't be checked, so they weren't selected. Nothing was removed.");
  assert.equal(blankPagesRunningAnnouncement, "Looking for blank pages…");
  assert.match(blankPagesFailedAnnouncement, /nothing was removed/);
});

test("the found chip counts the blank pages and hides at zero", () => {
  assert.equal(blankPagesFoundLabel(3), "3 found");
  assert.equal(blankPagesFoundLabel(0), null);
});

test("blank pages are named in the page option, before preview and selected", () => {
  assert.equal(pageOptionName(7, { sourceIndex: 6, rotation: 0 }, true, undefined, true), "Position 7, original page 7, blank, selected");
  assert.equal(pageOptionName(2, { sourceIndex: 6, rotation: 90 }, false, "loading", true), "Position 2, original page 7, rotated 90° right, blank, preview loading");
});

test("position lists share the page-label wording", () => {
  assert.equal(positionsList([16, 7, 11]), "7, 11 and 16");
  assert.equal(positionsList([1, 2, 3, 7, 9]), "1 to 3, 7 and 9");
  assert.equal(pagePositionsLabel([5]), "Page 5");
  assert.equal(pagePositionsLabel([9, 8]), "Pages 8 and 9");
});
