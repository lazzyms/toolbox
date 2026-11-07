import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { cutsOff, papers, placePage, type Orientation, type PageSize, type Placement } from "../src/features/pdf-crop/geometry.ts";

type Case = {
  name: string;
  page: PageSize & { rotate?: number };
  size: { preset: string } | { custom: PageSize };
  orientation: Orientation;
  placement: Placement;
  expected: { target: PageSize; scale: number; offset: { x: number; y: number }; cut: PageSize } | null;
};

const { cases }: { cases: Case[] } = JSON.parse(readFileSync(new URL("../shared/resize-placement-cases.json", import.meta.url), "utf8"));

const near = (actual: number, expected: number, label: string) => assert.ok(Math.abs(actual - expected) < 1e-3, `${label}: ${actual} != ${expected}`);

test("the preview places each shared case where the native resize puts it", () => {
  for (const { name, page, size, orientation, placement, expected } of cases) {
    const paper = "preset" in size ? papers.find(({ id }) => id === size.preset)! : size.custom;
    const placed = placePage({ width: page.width, height: page.height }, paper, orientation, placement);
    if (!expected) { assert.equal(placed, null, name); continue; }
    assert.ok(placed, name);
    near(placed.target.width, expected.target.width, `${name} target width`);
    near(placed.target.height, expected.target.height, `${name} target height`);
    near(placed.scale, expected.scale, `${name} scale`);
    near(placed.offset.x, expected.offset.x, `${name} offset x`);
    near(placed.offset.y, expected.offset.y, `${name} offset y`);
    near(placed.cut.width, expected.cut.width, `${name} cut width`);
    near(placed.cut.height, expected.cut.height, `${name} cut height`);
    assert.equal(cutsOff(placed), expected.cut.width > 1.5 || expected.cut.height > 1.5, `${name} cuts off`);
  }
});
