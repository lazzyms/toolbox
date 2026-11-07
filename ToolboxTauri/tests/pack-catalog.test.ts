import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { registeredToolIds, RetiredToolIds } from "../src/registry/retiredToolIds.ts";

const catalogs = {
  "built-in": "shared/pack-catalog.json",
  fixture: "tests/fixtures/pack-catalog.json",
};

for (const [name, path] of Object.entries(catalogs)) {
  test(`every ${name} pack catalog tool id exists and is not retired`, () => {
    const catalog = JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), "utf8")) as { packs: Array<{ id: string; toolIds: string[] }> };
    for (const pack of catalog.packs) {
      assert.ok(pack.toolIds.length > 0, `${pack.id} names the tools that use it`);
      for (const toolId of pack.toolIds) {
        assert.ok(registeredToolIds.has(toolId), `${pack.id} names unknown tool ${toolId}`);
        assert.ok(!RetiredToolIds.replacements.has(toolId), `${pack.id} names retired tool ${toolId}`);
      }
    }
  });
}
