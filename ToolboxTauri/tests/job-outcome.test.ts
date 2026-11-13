import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { parseJobOutcome, parseToolResult } from "../src/jobOutcome.ts";

const nativeSamples: unknown[] = JSON.parse(
  readFileSync(new URL("../shared/job-outcome-samples.json", import.meta.url), "utf8"),
);

const sample = (index: number) => structuredClone(nativeSamples[index]) as Record<string, unknown>;

test("native-shaped outcomes parse into the TypeScript contract for all three states", () => {
  const outcomes = parseToolResult(nativeSamples);

  assert.deepEqual(outcomes, nativeSamples);
  assert.deepEqual(outcomes.map((outcome) => outcome.state), ["succeeded", "skipped", "skipped", "failed", "failed", "succeeded", "skipped", "succeeded", "succeeded"]);

  const [succeeded, pointedSkip, plainSkip, failed, limited, , , notCarried, converted] = outcomes;
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
  assert.deepEqual(failed.failure.pointer, { kind: "pack", packId: "script-pack" });
  assert.deepEqual(failed.outputPaths, ["scan-page-1.png"], "failure keeps already published outputs");

  assert.ok(limited.state === "failed" && limited.failure.kind === "limitExceeded");
  assert.equal(limited.failure.details.limit, "time");
  assert.deepEqual(limited.outputPaths, ["drawings-page-1.png"], "a stopped run keeps the parts it saved");

  assert.deepEqual(notCarried.warnings.map((warning) => warning.code), ["bookmarksNotCarried", "tagsNotCarried"]);
  assert.deepEqual(
    notCarried.warnings[0].code === "bookmarksNotCarried" && notCarried.warnings[0].details,
    { into: "the merged file" },
  );
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
  const unknownFontSource = { ...sample(8), fonts: [{ ...(sample(8).fonts as object[])[0], source: "borrowed" }] };
  const fontsNotAList = { ...sample(8), fonts: { fontUsed: "Liberation Mono" } };

  for (const [name, value] of Object.entries({
    skippedWithOutputs,
    failedWithoutError,
    succeededWithReason,
    unknownSkipCode,
    unknownToolPointer,
    fontWarningWithoutDetails,
    limitWithoutDetails,
    unknownLimit,
    unknownFontSource,
    fontsNotAList,
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
