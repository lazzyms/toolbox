import assert from "node:assert/strict";
import test from "node:test";
import { formatBatchSaving, formatBytes, formatSavings, formatSizeChange, percentSaved } from "../src/shared/fileSize.ts";

test("byte sizes read like the design: whole bytes and KB, one decimal from MB", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(999), "999 B");
  assert.equal(formatBytes(1000), "1 KB");
  assert.equal(formatBytes(640_000), "640 KB");
  assert.equal(formatBytes(999_960), "1 MB");
  assert.equal(formatBytes(3_800_000), "3.8 MB");
  assert.equal(formatBytes(14_600_000), "14.6 MB");
  assert.equal(formatBytes(310_000_000), "310 MB");
  assert.equal(formatBytes(2_500_000_000), "2.5 GB");
});

test("percentage saved comes from the original and output sizes", () => {
  assert.equal(percentSaved(14_600_000, 3_800_000), 74);
  assert.equal(percentSaved(1000, 1000), 0);
  assert.equal(percentSaved(1000, 1250), -25, "a larger output is a negative saving");
  assert.equal(percentSaved(1000, 0), 100);
  assert.equal(percentSaved(0, 0), null, "nothing to compare against an empty original");
  assert.equal(percentSaved(0, 10), null);

  assert.equal(formatSavings(14_600_000, 3_800_000), "Saved 74%");
  assert.equal(formatSavings(1000, 1000), "No savings");
  assert.equal(formatSavings(4096, 4200), "No savings");
  assert.equal(formatSavings(100_000, 99_900), "Saved 1%", "a real saving never shows as 0%");
  assert.equal(formatSavings(100_000, 100), "Saved 99%", "a non-empty output never shows as 100%");
  assert.equal(formatSavings(1000, 0), "Saved 100%");
  assert.equal(formatSavings(0, 0), null);
});

test("a file's size change and a batch saving read as the Compress results board shows them", () => {
  assert.equal(formatSizeChange(14_600_000, 3_800_000), "14.6 MB → 3.8 MB, 74% smaller");
  assert.equal(formatSizeChange(640_000, 652_000), "640 KB → 652 KB, no savings");
  assert.equal(formatSizeChange(100_000, 99_900), "100 KB → 100 KB, 1% smaller");
  assert.equal(formatBatchSaving(25_400_000, 8_300_000), "Saved 17.1 MB, 67% smaller");
  assert.equal(formatBatchSaving(1000, 1000), "No savings");
});
