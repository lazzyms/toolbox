import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { ocrRequest } from "../src/ocrRequest.ts";

const nativeSample = JSON.parse(readFileSync(new URL("../shared/ocr-request-sample.json", import.meta.url), "utf8"));

test("the OCR request the view sends matches the shape the native command reads", () => {
  assert.deepEqual(ocrRequest(["/scans/contract.pdf"], "alongsideInput", "1-3, 5, 8-", "eng"), nativeSample);
});

test("OCR sends English text recognition and no range when every page is chosen", () => {
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], { customFolder: "/out" }, null, "eng"), {
    paths: ["/scans/a.pdf"],
    pageRange: null,
    mode: "text",
    force: false,
    languages: ["eng"],
    outputLocation: { customFolder: "/out" },
  });
});

test("OCR reads in the language picked for the run", () => {
  assert.deepEqual(ocrRequest(["/scans/a.pdf"], "alongsideInput", null, "hin").languages, ["hin"]);
});
