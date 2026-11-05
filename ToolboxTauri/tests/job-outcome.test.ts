import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { warningText } from "../src/components/resultRow.ts";
import { parseJobOutcome, parseToolResult } from "../src/jobOutcome.ts";

const nativeSamples: unknown[] = JSON.parse(
  readFileSync(new URL("../shared/job-outcome-samples.json", import.meta.url), "utf8"),
);

const sample = (index: number) => structuredClone(nativeSamples[index]) as Record<string, unknown>;

test("native-shaped outcomes parse into the TypeScript contract for all three states", () => {
  const outcomes = parseToolResult(nativeSamples);

  assert.deepEqual(outcomes, nativeSamples);
  assert.deepEqual(outcomes.map((outcome) => outcome.state), ["succeeded", "skipped", "skipped", "failed", "failed", "succeeded"]);

  const [succeeded, pointedSkip, plainSkip, failed, limited, images] = outcomes;
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
  assert.deepEqual(failed.failure.pointer, { kind: "pack", packId: "script-pack" });
  assert.deepEqual(failed.outputPaths, ["scan-page-1.png"], "failure keeps already published outputs");

  assert.ok(limited.state === "failed" && limited.failure.kind === "limitExceeded");
  assert.equal(limited.failure.details.limit, "time");
  assert.deepEqual(limited.outputPaths, ["drawings-page-1.png"], "a stopped run keeps the parts it saved");

  assert.deepEqual(images.warnings.map((warning) => warning.code), ["imageUnsupported", "imageDamaged", "imageTooLarge", "jpegColourKept"]);
  assert.deepEqual(images.warnings.map(warningText), images.warnings.map((warning) => warning.message), "rows say what the native side says");
});

test("image warnings name the pages as runs and keep singular wording for one image", () => {
  const kept = (colour: string, pages: number[], images: number) =>
    warningText(parseJobOutcome({ ...sample(5), warnings: [{ code: "jpegColourKept", message: "", details: { colour, pages, images } }] }).warnings[0]);

  assert.equal(kept("inverted", [22], 1), "Page 22 — 1 inverted JPEG, copied as it is. Some viewers show it in negative.");
  assert.equal(kept("inverted", [1, 2, 3, 7], 5), "Pages 1-3, 7 — 5 inverted JPEGs, copied as they are. Some viewers show them in negative.");
  assert.equal(kept("cmyk", [4], 1), "Page 4 — 1 JPEG uses CMYK colour and was copied as it is. Some viewers show it with the wrong colours.");
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
  const textPage = { ...sample(5), warnings: [{ code: "imageDamaged", message: "Damaged.", details: { page: "19", image: 1 } }] };
  const fractionalSize = { ...sample(5), warnings: [{ code: "imageTooLarge", message: "Big.", details: { page: 1, image: 1, width: 1.5, height: 2 } }] };
  const unknownColour = { ...sample(5), warnings: [{ code: "jpegColourKept", message: "Kept.", details: { colour: "sepia", pages: [1], images: 1 } }] };
  const pagesNotList = { ...sample(5), warnings: [{ code: "jpegColourKept", message: "Kept.", details: { colour: "cmyk", pages: 1, images: 1 } }] };

  for (const [name, value] of Object.entries({
    skippedWithOutputs,
    failedWithoutError,
    succeededWithReason,
    unknownSkipCode,
    unknownToolPointer,
    fontWarningWithoutDetails,
    limitWithoutDetails,
    unknownLimit,
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
