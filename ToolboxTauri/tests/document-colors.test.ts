import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { colorTokens, documentColor } from "../src/features/pdf-editor/colorTokens";

test("document colors accept only #RRGGBB, matching the native scene contract", () => {
  assert.equal(documentColor(colorTokens.documentText, " #1B1B1B "), "#1b1b1b");
  for (const value of ["oklch(0.22 0 0)", "#fff", "#11223344", "rgb(0 0 0)", "1b1b1b"]) {
    assert.throws(() => documentColor(colorTokens.documentText, value), /must be #RRGGBB/);
  }
  assert.throws(() => documentColor(colorTokens.documentText, "  "), /is missing/);
});

test("scene stylesheet defines every document color token as #RRGGBB", () => {
  const css = readFileSync(new URL("../src/features/pdf-editor/scene-canvas.css", import.meta.url), "utf8");
  for (const name of Object.values(colorTokens)) {
    const values = [...css.matchAll(new RegExp(`${name}:\\s*([^;]+);`, "g"))].map((match) => match[1]);
    assert.ok(values.length > 0, `${name} is not defined`);
    for (const value of values) documentColor(name, value);
  }
});
