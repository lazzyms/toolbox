import assert from "node:assert/strict";
import test from "node:test";
import {
  assignBatesRanges,
  batesFormatIssue,
  batesIdentifier,
  batesRangeLabel,
  defaultBatesFormat,
  unusedBatesRanges,
} from "../src/features/pdf-editor/bates";

const format = { ...defaultBatesFormat, prefix: "HL" };

test("identifiers use the prefix, zero-padded number and suffix", () => {
  assert.equal(batesIdentifier(format, 1), "HL000001");
  assert.equal(batesIdentifier({ ...format, suffix: "-C", digits: 2 }, 123), "HL123-C");
  assert.equal(batesRangeLabel(format, { first: 17, pages: 3 }), "HL000017 to HL000019");
  assert.equal(batesRangeLabel(format, { first: 5, pages: 1 }), "HL000005");
});

test("ranges are set aside in file order and continue across files", () => {
  assert.deepEqual(assignBatesRanges(1, [14, 2, 3, 9]), [
    { first: 1, pages: 14 }, { first: 15, pages: 2 }, { first: 17, pages: 3 }, { first: 20, pages: 9 },
  ]);
});

test("a file with an unknown page count gets no range and the next file continues", () => {
  assert.deepEqual(assignBatesRanges(100, [2, null, 3]), [{ first: 100, pages: 2 }, null, { first: 102, pages: 3 }]);
});

test("a failed file keeps its range as an unused gap while the others keep theirs", () => {
  const ranges = assignBatesRanges(1, [14, 2, 3, 9]);
  assert.deepEqual(unusedBatesRanges(ranges, [false, false, true, false]), [{ first: 17, pages: 3 }]);
  assert.deepEqual(ranges[3], { first: 20, pages: 9 }, "the file after the failure keeps its numbers");
  assert.deepEqual(unusedBatesRanges([null], [true]), [], "a file with no range leaves no gap");
});

test("the same counts and settings always give the same ranges", () => {
  assert.deepEqual(assignBatesRanges(7, [3, null, 1]), assignBatesRanges(7, [3, null, 1]));
});

test("format issues match the native limits", () => {
  assert.equal(batesFormatIssue(format), null);
  assert.match(batesFormatIssue({ ...format, digits: 0 }) ?? "", /Digits/);
  assert.match(batesFormatIssue({ ...format, digits: 13 }) ?? "", /Digits/);
  assert.match(batesFormatIssue({ ...format, prefix: "A\nB" }) ?? "", /prefix/);
  assert.match(batesFormatIssue({ ...format, suffix: "x".repeat(41) }) ?? "", /suffix/);
  assert.match(batesFormatIssue({ ...format, start: 1.5 }) ?? "", /Start at/);
  assert.match(batesFormatIssue({ ...format, fontSize: 80 }) ?? "", /Size/);
});
