import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { resultSummary, rowView, warningText } from "../src/components/resultRow.ts";
import { parseJobOutcome, parseToolResult } from "../src/jobOutcome.ts";

const nativeSamples: unknown[] = JSON.parse(
  readFileSync(new URL("../shared/job-outcome-samples.json", import.meta.url), "utf8"),
);

const sample = (index: number) => structuredClone(nativeSamples[index]) as Record<string, unknown>;

test("native-shaped outcomes parse into the TypeScript contract for all three states", () => {
  const outcomes = parseToolResult(nativeSamples);

  assert.deepEqual(outcomes, nativeSamples);
  assert.deepEqual(outcomes.map((outcome) => outcome.state), ["succeeded", "skipped", "skipped", "failed", "failed", "succeeded", "skipped", "succeeded", "succeeded", "succeeded", "succeeded", "succeeded"]);

  const [succeeded, pointedSkip, plainSkip, failed, limited, , , notCarried, converted, images, blankPages, flattened] = outcomes;
  for (const outcome of outcomes.slice(0, 5)) {
    assert.ok(!("originalBytes" in outcome) && !("outputBytes" in outcome), "outcomes without sizes stay unchanged");
  }
  assert.equal(succeeded.fonts, undefined, "outcomes without fonts carry no font list");
  assert.deepEqual(converted.fonts, [
    { requestedFamily: "Courier New", requestedStyle: "regular", fontUsed: "Liberation Mono", styleUsed: "regular", source: "bundled fallback" },
  ]);
  assert.equal(succeeded.state === "succeeded" && succeeded.outputPaths[0], "signed-edited.pdf");
  assert.deepEqual(succeeded.warnings.map((warning) => warning.code), ["signatureRemoved", "fontSubstituted"]);
  assert.deepEqual(
    succeeded.warnings[1].code === "fontSubstituted" && succeeded.warnings[1].details,
    { requestedFamily: "Garamond", actualFamily: "Times New Roman" },
  );

  assert.ok(pointedSkip.state === "skipped");
  assert.deepEqual(pointedSkip.outputPaths, []);
  assert.equal(pointedSkip.reason.code, "unsupportedFormat");
  assert.deepEqual(pointedSkip.reason.pointer, { kind: "tool", toolId: "heic-convert" });
  assert.ok(plainSkip.state === "skipped" && plainSkip.reason.pointer === null);

  assert.ok(failed.state === "failed");
  assert.equal(failed.failure.kind, "unavailable");
  assert.deepEqual(failed.failure.pointer, { kind: "pack", packId: "script-pack", name: "Indic script", reason: "damaged" });
  assert.deepEqual(failed.outputPaths, ["scan-page-1.png"], "failure keeps already published outputs");

  assert.ok(limited.state === "failed" && limited.failure.kind === "limitExceeded");
  assert.equal(limited.failure.details.limit, "time");
  assert.deepEqual(limited.outputPaths, ["drawings-page-1.png"], "a stopped run keeps the parts it saved");

  assert.deepEqual(notCarried.warnings.map((warning) => warning.code), ["bookmarksNotCarried", "tagsNotCarried"]);
  assert.deepEqual(
    notCarried.warnings[0].code === "bookmarksNotCarried" && notCarried.warnings[0].details,
    { into: "the merged file" },
  );

  assert.deepEqual(images.warnings.map((warning) => warning.code), ["imageUnsupported", "imageDamaged", "imageTooLarge", "jpegColourKept"]);
  assert.deepEqual(images.warnings.map(warningText), images.warnings.map((warning) => warning.message), "rows say what the native side says");
  assert.deepEqual(blankPages.warnings.map(warningText), blankPages.warnings.map((warning) => warning.message));
  assert.deepEqual(flattened.warnings.map((warning) => warning.code), ["transparencyFlattened"]);
});

test("pages without text are a note that points at OCR, not a warning", () => {
  const withPages = (pages: number[], extra: unknown[] = []) =>
    parseJobOutcome({ ...sample(10), warnings: [{ code: "pagesWithoutText", message: "", details: { pages } }, ...extra] });
  assert.equal(warningText(withPages([3]).warnings[0]), "Page 3 has no selectable text. OCR can read it.");
  assert.equal(warningText(withPages([2, 3, 6]).warnings[0]), "Pages 2-3, 6 have no selectable text. OCR can read them.");

  const note = rowView(withPages([30, 31, 32, 33]), "Extracted");
  assert.equal(note.tone, "note");
  assert.equal(note.spokenState, "Extracted with a note");
  assert.deepEqual(note.detailLines.map((line) => line.text), ["Pages 30-33 have no selectable text. OCR can read them.", "PDF text extracted in reading order"]);
  assert.deepEqual(note.pointer?.pointer, { kind: "tool", toolId: "pdf-ocr" });
  assert.equal(note.openOutput?.path, "scan-archive-text.txt", "the text file was still written");
  assert.deepEqual(resultSummary([withPages([2])], "Extracted").hints, ["1 extracted file has a warning or note."], "a note is counted with warnings");

  const kept = sample(9).warnings as unknown[];
  const warned = rowView(withPages([2], kept.slice(-1)), "Extracted");
  assert.equal(warned.tone, "warn", "a real warning still wins the row's tone");
  assert.deepEqual(warned.pointer?.pointer, { kind: "tool", toolId: "pdf-ocr" });
  assert.equal(rowView(sample(9) as never, "Extracted").pointer, null, "rows without blank pages keep no pointer");
});

test("skipped PDFs keep their reason and OCR pointer apart from failures", () => {
  const scanned = parseJobOutcome({
    ...sample(2),
    inputPath: "scan.pdf",
    reason: { code: "needsOcr", message: "This PDF has no text layer. Run OCR to read its text.", pointer: { kind: "tool", toolId: "pdf-ocr" } },
  });
  const range = parseJobOutcome({ ...sample(2), inputPath: "memo.pdf", reason: { code: "rangeEmptyAfterClamping", message: "No pages in 3-5, 8- are in this PDF, which has 2 pages.", pointer: null } });
  const scannedRow = rowView(scanned, "Extracted");
  assert.deepEqual([scannedRow.tone, scannedRow.stateLabel, scannedRow.detailLines[0]?.text], ["skip", "Skipped", "This PDF has no text layer. Run OCR to read its text."]);
  assert.deepEqual(scannedRow.pointer?.pointer, { kind: "tool", toolId: "pdf-ocr" });
  assert.equal(scannedRow.canRetry, false);
  const rangeRow = rowView(range, "Extracted");
  assert.deepEqual([rangeRow.tone, rangeRow.detailLines.map((line) => line.text), rangeRow.pointer], ["skip", ["No pages in 3-5, 8- are in this PDF, which has 2 pages."], null]);
});

test("image warnings name the pages as runs and keep singular wording for one image", () => {
  const kept = (colour: string, pages: number[], images: number) =>
    warningText(parseJobOutcome({ ...sample(9), warnings: [{ code: "jpegColourKept", message: "", details: { colour, pages, images } }] }).warnings[0]);

  assert.equal(kept("inverted", [22], 1), "Page 22 — 1 inverted JPEG, copied as it is. Some viewers show it in negative.");
  assert.equal(kept("inverted", [1, 2, 3, 7], 5), "Pages 1-3, 7 — 5 inverted JPEGs, copied as they are. Some viewers show them in negative.");
  assert.equal(kept("cmyk", [4], 1), "Page 4 — 1 JPEG uses CMYK colour and was copied as it is. Some viewers show it with the wrong colours.");
});

test("native-shaped outcomes carry original and output byte sizes when reported", () => {
  const [compressed, noSavings] = parseToolResult(nativeSamples).slice(5);

  assert.ok(compressed.state === "succeeded");
  assert.deepEqual([compressed.originalBytes, compressed.outputBytes], [2_500_000, 1_000_000]);
  assert.deepEqual(compressed.outputPaths, ["report-compressed.pdf"]);

  assert.ok(noSavings.state === "skipped" && noSavings.reason.code === "noSavings");
  assert.deepEqual(noSavings.outputPaths, []);
  assert.deepEqual([noSavings.originalBytes, noSavings.outputBytes], [4096, 4200]);

  const explicitNulls = parseJobOutcome({ ...sample(0), originalBytes: null, outputBytes: null });
  assert.ok(!("originalBytes" in explicitNulls) && !("outputBytes" in explicitNulls));
  const onlyOriginal = parseJobOutcome({ ...sample(6), outputBytes: undefined });
  assert.deepEqual([onlyOriginal.originalBytes, onlyOriginal.outputBytes], [4096, undefined]);

  for (const bad of [-1, 1.5, "4096", Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => parseJobOutcome({ ...sample(5), outputBytes: bad }), /Invalid job outcome: outcome.outputBytes/);
  }
});

test("outcomes from before the state field are rejected at the boundary", () => {
  const legacy = { inputPath: "a.pdf", outputPaths: ["a-1.pdf"], detail: "Saved", failure: null };
  assert.throws(() => parseJobOutcome(legacy), /Invalid job outcome/);
});

test("inconsistent native outcomes are rejected", () => {
  const skippedWithOutputs = { ...sample(1), outputPaths: ["photo-1.jpg"] };
  const failedWithoutError = { ...sample(3), failure: null };
  const succeededWithReason = { ...sample(0), reason: sample(1).reason };
  const unknownSkipCode = { ...sample(2), reason: { code: "bored", message: "No.", pointer: null } };
  const unknownToolPointer = { ...sample(1), reason: { ...(sample(1).reason as object), pointer: { kind: "tool", toolId: "retired-tool" } } };
  const fontWarningWithoutDetails = { ...sample(0), warnings: [{ code: "fontSubstituted", message: "Changed." }] };
  const limitWithoutDetails = { ...sample(4), failure: { kind: "limitExceeded", message: "Too big.", pointer: null } };
  const unknownLimit = { ...sample(4), failure: { kind: "limitExceeded", details: { limit: "patience" }, message: "Too big.", pointer: null } };
  const packPointer = (pointer: object) => ({ ...sample(3), failure: { kind: "unavailable", message: "Needs a pack.", pointer: { kind: "pack", packId: "ocr-hindi", name: "Hindi", reason: "notInstalled", ...pointer } } });
  const packWithoutName = packPointer({ name: undefined });
  const unknownPackNeed = packPointer({ reason: "bored" });
  const unknownFontSource = { ...sample(8), fonts: [{ ...(sample(8).fonts as object[])[0], source: "borrowed" }] };
  const fontsNotAList = { ...sample(8), fonts: { fontUsed: "Liberation Mono" } };
  const textPage = { ...sample(9), warnings: [{ code: "imageDamaged", message: "Damaged.", details: { page: "19", image: 1 } }] };
  const fractionalSize = { ...sample(9), warnings: [{ code: "imageTooLarge", message: "Big.", details: { page: 1, image: 1, width: 1.5, height: 2 } }] };
  const unknownColour = { ...sample(9), warnings: [{ code: "jpegColourKept", message: "Kept.", details: { colour: "sepia", pages: [1], images: 1 } }] };
  const pagesNotList = { ...sample(9), warnings: [{ code: "jpegColourKept", message: "Kept.", details: { colour: "cmyk", pages: 1, images: 1 } }] };

  for (const [name, value] of Object.entries({
    skippedWithOutputs,
    failedWithoutError,
    succeededWithReason,
    unknownSkipCode,
    unknownToolPointer,
    fontWarningWithoutDetails,
    limitWithoutDetails,
    unknownLimit,
    packWithoutName,
    unknownPackNeed,
    unknownFontSource,
    fontsNotAList,
    textPage,
    fractionalSize,
    unknownColour,
    pagesNotList,
  })) {
    assert.throws(() => parseJobOutcome(value), /Invalid job outcome/, name);
  }
});

test("a read-only output folder failure carries the output folder pointer", () => {
  const readOnly = {
    ...sample(3),
    outputPaths: [],
    failure: { kind: "processing", message: "The output folder is read-only. Nothing was saved.", pointer: { kind: "outputFolder" } },
    warnings: [],
  };
  const outcome = parseJobOutcome(readOnly);
  assert.ok(outcome.state === "failed");
  assert.deepEqual(outcome.failure.pointer, { kind: "outputFolder" });
});
