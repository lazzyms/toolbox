import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { ocrRequest } from "../src/ocrRequest.ts";

const nativeSample = JSON.parse(readFileSync(new URL("../shared/ocr-request-sample.json", import.meta.url), "utf8"));

test("the OCR request the view sends matches the shape the native command reads", () => {
  assert.deepEqual(ocrRequest(["/scans/contract.pdf"], "alongsideInput", "1-3, 5, 8-", "text"), nativeSample);
});

test("OCR sends English recognition in the chosen mode and no range when every page is chosen", () => {
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], { customFolder: "/out" }, null, "searchablePdf"), {
    paths: ["/scans/a.pdf"],
    pageRange: null,
    mode: "searchablePdf",
    force: false,
    languages: ["eng"],
    outputLocation: { customFolder: "/out" },
  });
});

test("Force asks OCR to read pages again whose only text is an earlier invisible layer", () => {
  assert.equal(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, "text").force, false);
  assert.equal(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, "text", true).force, true);
});
