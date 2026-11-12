import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { inspectFallbackFonts, readFontManifest, sourceFontsDir } from "../scripts/fallback-fonts.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtures = mkdtempSync(join(tmpdir(), "toolbox-fonts-release-"));
test.after(() => rmSync(fixtures, { recursive: true, force: true }));

// The fallback fonts and install-size budget chosen for #589, shared with Document Conversion (#424)
// and Formatted PDF and EPUB (#428). Changing either is a deliberate release decision.
const FALLBACK_BUDGET_BYTES = 6_000_000;
const FALLBACK_FONTS = {
  "Liberation Sans": { substitutes: ["Arial", "Helvetica"], files: ["LiberationSans-Regular.ttf", "LiberationSans-Bold.ttf"] },
  "Liberation Serif": {
    substitutes: ["Times New Roman", "Times"],
    files: ["LiberationSerif-Regular.ttf", "LiberationSerif-Bold.ttf", "LiberationSerif-Italic.ttf", "LiberationSerif-BoldItalic.ttf"],
  },
  "Liberation Mono": { substitutes: ["Courier New", "Courier"], files: ["LiberationMono-Regular.ttf"] },
  Carlito: { substitutes: ["Calibri"], files: ["Carlito-Regular.ttf", "Carlito-Bold.ttf", "Carlito-Italic.ttf", "Carlito-BoldItalic.ttf"] },
  Caladea: { substitutes: ["Cambria"], files: ["Caladea-Regular.ttf", "Caladea-Bold.ttf", "Caladea-Italic.ttf", "Caladea-BoldItalic.ttf"] },
};
const DEFAULT_FONT = { family: "DejaVu Sans", file: "DejaVuSans.ttf" };

function copyFonts(name) {
  const directory = join(fixtures, name);
  cpSync(sourceFontsDir, directory, { recursive: true });
  return directory;
}

const readiness = (args) => spawnSync(process.execPath, ["scripts/check-release-readiness.mjs", ...args], { cwd: projectRoot, encoding: "utf8" });

test("the font manifest records the chosen fallback fonts, licenses and budget", () => {
  const manifest = readFontManifest();
  const policy = manifest.fallbackFonts;
  assert.equal(policy.budgetBytes, FALLBACK_BUDGET_BYTES);
  const fileOf = (key) => manifest.resources[key].path;
  assert.deepEqual(
    Object.fromEntries(policy.families.map((family) => [family.family, { substitutes: family.substitutes, files: Object.values(family.faces).map(fileOf) }])),
    FALLBACK_FONTS,
  );
  assert.deepEqual({ family: policy.default.family, file: fileOf(policy.default.faces.regular) }, DEFAULT_FONT);
  for (const family of policy.families) {
    for (const key of Object.values(family.faces)) {
      const spec = manifest.resources[key];
      assert.match(spec.sha256, /^[0-9a-f]{64}$/, key);
      assert.match(spec.license, /SIL Open Font License 1\.1; see \S+-LICENSE\.txt/, key);
      readFileSync(join(sourceFontsDir, spec.license.split("see ")[1]));
    }
  }
});

test("the shipped fallback fonts pass within the recorded budget", () => {
  const result = inspectFallbackFonts();
  assert.equal(result.cause, null, result.message);
  assert.match(result.message, /15 fallback faces in 5 families/);
});

test("fails when a fallback font is missing", () => {
  const fontsDir = copyFonts("missing");
  rmSync(join(fontsDir, "Carlito-Italic.ttf"));
  const result = inspectFallbackFonts({ fontsDir });
  assert.equal(result.cause, "missing");
  assert.match(result.message, /Carlito-Italic\.ttf/);
});

test("fails when a fallback font has changed", () => {
  const fontsDir = copyFonts("changed");
  const path = join(fontsDir, "LiberationSerif-Bold.ttf");
  const bytes = readFileSync(path);
  bytes[bytes.length - 1] ^= 0xff;
  writeFileSync(path, bytes);
  const result = inspectFallbackFonts({ fontsDir });
  assert.equal(result.cause, "changed");
  assert.match(result.message, /LiberationSerif-Bold\.ttf/);
});

test("fails when the bundled default font has changed", () => {
  const fontsDir = copyFonts("default-changed");
  writeFileSync(join(fontsDir, DEFAULT_FONT.file), "not a font");
  assert.equal(inspectFallbackFonts({ fontsDir }).cause, "changed");
});

test("fails when the fallback fonts are over the recorded budget", () => {
  const fontsDir = copyFonts("over-budget");
  const manifest = readFontManifest(fontsDir);
  manifest.fallbackFonts.families.push({ family: "Extra", substitutes: [], faces: { regular: "watermarkFont" } });
  const result = inspectFallbackFonts({ fontsDir, manifest });
  assert.equal(result.cause, "over-budget");
  assert.match(result.message, new RegExp(`over the ${FALLBACK_BUDGET_BYTES}-byte budget`));
});

test("release readiness fails a built bundle whose fallback fonts are missing, changed or over budget", () => {
  const missing = copyFonts("readiness-missing");
  rmSync(join(missing, "LiberationMono-Regular.ttf"));
  let result = readiness(["--fonts", missing]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Fallback font release check failed \(missing\)/);

  const changed = copyFonts("readiness-changed");
  writeFileSync(join(changed, "Caladea-Regular.ttf"), "changed");
  result = readiness(["--fonts", changed]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Fallback font release check failed \(changed\)/);

  const overBudget = copyFonts("readiness-over-budget");
  const manifest = readFontManifest(overBudget);
  manifest.fallbackFonts.budgetBytes = FALLBACK_BUDGET_BYTES / 2;
  writeFileSync(join(overBudget, "manifest.json"), JSON.stringify(manifest, null, 2));
  result = readiness(["--fonts", overBudget]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Fallback font release check failed \(over-budget\)/);
});

test("release readiness passes the source fonts", () => {
  const result = readiness([]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Fallback font release check passed/);
});
