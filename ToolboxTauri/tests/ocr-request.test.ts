import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { ocrRequest } from "../src/ocrRequest.ts";

const nativeSample = JSON.parse(readFileSync(new URL("../shared/ocr-request-sample.json", import.meta.url), "utf8"));

test("the OCR request the view sends matches the shape the native command reads", () => {
  assert.deepEqual(ocrRequest(["/scans/contract.pdf"], "alongsideInput", "1-3, 5, 8-", ["eng"], "text"), nativeSample);
});

test("OCR sends English recognition in the chosen mode and no range when every page is chosen", () => {
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], { customFolder: "/out" }, null, ["eng"], "searchablePdf"), {
    paths: ["/scans/a.pdf"],
    pageRange: null,
    mode: "searchablePdf",
    force: false,
    languages: ["eng"],
    outputLocation: { customFolder: "/out" },
  });
});

test("OCR reads in the languages picked for the run, in their order", () => {
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, ["hin"], "text").languages, ["hin"]);
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, ["hin", "eng"], "text").languages, ["hin", "eng"], "languages keep the order chosen");
});

test("Force asks OCR to read pages again whose only text is an earlier invisible layer", () => {
  assert.equal(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, ["eng"], "text").force, false);
  assert.equal(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, ["eng"], "text", true).force, true);
});

test("OCR sends the range in page order, and no range when the field is empty", () => {
  const range = (typed: string | null) => ocrRequest(["/scans/a.pdf"], "alongsideInput", typed, ["eng"], "text").pageRange;
  assert.equal(range("9-, 5, 1-3"), "1-3, 5, 9-");
  assert.equal(range(" 4 ,2-3,1 "), "1, 2-3, 4");
  assert.equal(range(""), null);
  assert.equal(range("   "), null);
  assert.equal(range(null), null);
});
