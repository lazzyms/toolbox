import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { pageRangesWithin, parsePageRanges } from "../src/shared/pageRanges.ts";
import { parsePageRange } from "../src/shared/workspaceValidation.ts";

type Cases = {
  accepted: { input: string; spans: [number, number | null][] }[];
  rejected: { input: string; message: string }[];
  within: { input: string; pageCount: number; pages?: [number, number][]; message?: string }[];
};

const cases: Cases = JSON.parse(readFileSync(new URL("../shared/page-range-cases.json", import.meta.url), "utf8"));

test("the UI parser accepts and rejects the shared native cases with the same messages", () => {
  for (const { input, spans } of cases.accepted) {
    assert.deepEqual(parsePageRanges(input), { ok: true, value: spans.map(([first, last]) => ({ first, last })) }, input);
  }
  for (const { input, message } of cases.rejected) {
    assert.deepEqual(parsePageRanges(input), { ok: false, message }, input);
  }
  for (const { input, pageCount, pages, message } of cases.within) {
    const parsed = parsePageRanges(input);
    assert.ok(parsed.ok, input);
    assert.deepEqual(pageRangesWithin(parsed.value, pageCount), pages ? { ok: true, value: pages } : { ok: false, message }, input);
  }
});

test("the workspace page selection uses the shared grammar and keeps blank as every page", () => {
  assert.deepEqual(parsePageRange("", 3), [0, 1, 2]);
  assert.deepEqual(parsePageRange("3, 2-", 3), [1, 2]);
  assert.deepEqual(parsePageRange("1-3,,2", 3), { field: "pageRange", message: "Entry 2 of the page range is empty. Remove the extra comma." });
  assert.deepEqual(parsePageRange("4", 3), { field: "pageRange", message: "The PDF has 3 pages, so page 4 can't be used." });
});
