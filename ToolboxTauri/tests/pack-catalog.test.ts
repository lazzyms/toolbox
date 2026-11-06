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

type OcrLanguagePack = { id: string; toolIds: string[]; ocrLanguage?: { code: string; name: string } };

// OCR points a file at the pack whose ocrLanguage code matches its language, so every such pack is
// one OCR uses, and each language has exactly one pack. English ships with the app.
for (const [name, path] of Object.entries(catalogs)) {
  test(`every ${name} OCR language pack is one OCR PDF uses, with a code no other pack claims`, () => {
    const catalog = JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), "utf8")) as { packs: OcrLanguagePack[] };
    const codes = new Set<string>(["eng"]);
    for (const pack of catalog.packs) {
      if (!pack.ocrLanguage) continue;
      const { code, name: language } = pack.ocrLanguage;
      assert.ok(pack.toolIds.includes("pdf-ocr"), `${pack.id} reads ${language} for OCR PDF`);
      assert.match(code, /^[a-z_]+$/, `${pack.id} has a Tesseract language code`);
      assert.ok(language.trim().length > 0, `${pack.id} names its language`);
      assert.ok(!codes.has(code), `${pack.id} claims ${code}, which another pack or the app already has`);
      codes.add(code);
    }
  });
}

test("the fixture catalog has OCR language packs, so the checks above are not vacuous", () => {
  const catalog = JSON.parse(readFileSync(new URL(`../${catalogs.fixture}`, import.meta.url), "utf8")) as { packs: OcrLanguagePack[] };
  assert.ok(catalog.packs.filter((pack) => pack.ocrLanguage).length >= 2);
});

// Edit PDF and Convert to PDF both write text in the scripts a pack's fonts cover, so a script pack
// names both, and Settings lists both as using it.
for (const [name, path] of Object.entries(catalogs)) {
  test(`every ${name} script pack names Edit PDF and Convert to PDF`, () => {
    const catalog = JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), "utf8")) as { packs: Array<{ id: string; toolIds: string[]; scriptFonts?: Record<string, string> }> };
    for (const pack of catalog.packs) {
      if (!pack.scriptFonts || Object.keys(pack.scriptFonts).length === 0) continue;
      for (const toolId of ["pdf-edit", "convert-to-pdf"]) assert.ok(pack.toolIds.includes(toolId), `${pack.id} names ${toolId}`);
    }
  });
}
