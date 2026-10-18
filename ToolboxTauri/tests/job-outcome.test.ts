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
  assert.deepEqual(outcomes.map((outcome) => outcome.state), ["succeeded", "skipped", "skipped", "failed"]);

  const [succeeded, pointedSkip, plainSkip, failed] = outcomes;
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

  for (const [name, value] of Object.entries({
    skippedWithOutputs,
    failedWithoutError,
    succeededWithReason,
    unknownSkipCode,
    unknownToolPointer,
    fontWarningWithoutDetails,
  })) {
    assert.throws(() => parseJobOutcome(value), /Invalid job outcome/, name);
  }
});
