import assert from "node:assert/strict";
import test from "node:test";
import { SOURCE_THUMBNAIL_BATCH_SIZE, SOURCE_THUMBNAIL_CACHE_MAX_ENTRIES, createSourceThumbnailQueue } from "../src/features/pdf-editor/sourceThumbnails";
import type { SourceThumbnail } from "../src/features/pdf-editor/sourceThumbnails";

const preview = (index: number) => ({ dataUrl: `data:image/png;base64,${index}`, width: 232, height: 300 });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** A fake helper whose requests resolve only when the test says so. */
const deferredHelper = (fail: ReadonlySet<number> = new Set()) => {
  const requests: number[][] = [];
  const pending: (() => void)[] = [];
  const fetch = (pageIndices: number[]) => {
    requests.push(pageIndices);
    return new Promise<SourceThumbnail[]>((resolve) => pending.push(() => resolve(pageIndices.map((pageIndex) => fail.has(pageIndex)
      ? { pageIndex, thumbnail: null, error: "pdftoppm could not render this PDF" }
      : { pageIndex, thumbnail: preview(pageIndex), error: null }))));
  };
  const finishNext = async () => { pending.shift()?.(); await settle(); };
  return { requests, fetch, finishNext };
};

test("the recorded batch size fits the native limit and the cache holds several screens of pages", () => {
  assert.equal(SOURCE_THUMBNAIL_BATCH_SIZE, 6);
  assert.ok(SOURCE_THUMBNAIL_BATCH_SIZE <= 12);
  assert.equal(SOURCE_THUMBNAIL_CACHE_MAX_ENTRIES, 240);
});

test("only wanted pages are requested, one batch at a time, and scrolled-away pages are dropped before they are sent", async () => {
  const helper = deferredHelper();
  let changes = 0;
  const queue = createSourceThumbnailQueue({ fetch: helper.fetch, onChange: () => { changes += 1; }, batchSize: 3 });
  queue.want([0, 1, 2, 3, 4, 5]);
  assert.deepEqual(helper.requests, [[0, 1, 2]]);
  assert.equal(queue.get(0).status, "loading");
  queue.want([40, 41]);
  await helper.finishNext();
  assert.equal(changes, 1);
  assert.deepEqual(helper.requests, [[0, 1, 2], [40, 41]], "pages 3 to 5 scrolled away before they were requested");
  assert.deepEqual(queue.get(1), { status: "ready", value: preview(1) });
  await helper.finishNext();
  assert.equal(queue.get(41).status, "ready");
  queue.want([0, 1, 2, 40, 41]);
  await settle();
  assert.equal(helper.requests.length, 2, "cached pages are reused when scrolling back");
});

test("a failed page stays a placeholder without failing its batch or retrying", async () => {
  const helper = deferredHelper(new Set([1]));
  const queue = createSourceThumbnailQueue({ fetch: helper.fetch, onChange: () => {} });
  queue.want([0, 1, 2]);
  await helper.finishNext();
  assert.equal(queue.get(0).status, "ready");
  assert.deepEqual(queue.get(1), { status: "failed" });
  assert.equal(queue.get(2).status, "ready");
  queue.want([0, 1, 2]);
  await settle();
  assert.equal(helper.requests.length, 1);
});

test("a rejected request marks its pages failed and the queue moves on", async () => {
  const requests: number[][] = [];
  const queue = createSourceThumbnailQueue({ batchSize: 2, onChange: () => {}, fetch: async (pageIndices) => {
    requests.push(pageIndices);
    if (pageIndices.includes(0)) throw new Error("helper unavailable");
    return pageIndices.map((pageIndex) => ({ pageIndex, thumbnail: preview(pageIndex), error: null }));
  } });
  queue.want([0, 1, 2]);
  await settle(); await settle();
  assert.deepEqual(requests, [[0, 1], [2]]);
  assert.equal(queue.get(0).status, "failed");
  assert.equal(queue.get(2).status, "ready");
});

test("the cache is a bounded LRU that keeps the pages on screen", async () => {
  const requests: number[][] = [];
  const queue = createSourceThumbnailQueue({ batchSize: 2, maxEntries: 4, onChange: () => {}, fetch: async (pageIndices) => {
    requests.push(pageIndices);
    return pageIndices.map((pageIndex) => ({ pageIndex, thumbnail: preview(pageIndex), error: null }));
  } });
  const load = async (pages: number[]) => { queue.want(pages); for (let step = 0; step < 4; step += 1) await settle(); };
  await load([0, 1, 2, 3]);
  await load([0, 1]);
  await load([10, 11]);
  assert.equal(queue.get(0).status, "ready", "recently wanted pages stay cached");
  assert.equal(queue.get(1).status, "ready");
  assert.equal(queue.get(2).status, "loading", "the oldest page that isn't wanted is evicted");
  assert.equal(queue.get(3).status, "loading");
  const before = requests.length;
  await load([2]);
  assert.deepEqual(requests.slice(before), [[2]], "an evicted page is requested again");
});

test("a disposed queue ignores late results and sends nothing more", async () => {
  const helper = deferredHelper();
  let changes = 0;
  const queue = createSourceThumbnailQueue({ fetch: helper.fetch, onChange: () => { changes += 1; }, batchSize: 1 });
  queue.want([0, 1]);
  queue.dispose();
  await helper.finishNext();
  assert.equal(changes, 0);
  assert.equal(queue.get(0).status, "loading");
  assert.deepEqual(helper.requests, [[0]]);
});
