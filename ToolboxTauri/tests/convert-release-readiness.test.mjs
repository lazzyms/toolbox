import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { RUST_SOURCES, conversionHelperUses, conversionSources, verifyConvertArtifact } from "../scripts/convert-release.mjs";
import { FAMILIES, compareOutputs, readDifferences, readTargetOutputs, uncoveredFamilies } from "../scripts/compare-convert-outputs.mjs";
import { sourceFontsDir } from "../scripts/fallback-fonts.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtures = mkdtempSync(join(tmpdir(), "toolbox-convert-release-"));
test.after(() => rmSync(fixtures, { recursive: true, force: true }));

const MAC = "aarch64-apple-darwin";
const WINDOWS = "x86_64-pc-windows-msvc";

function machO(cpu = 0x0100000c) {
  const bytes = Buffer.alloc(64);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(cpu, 4);
  return bytes;
}

function pe(machine = 0x8664) {
  const bytes = Buffer.alloc(0x100);
  bytes.write("MZ", 0, "latin1");
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.write("PE\0\0", 0x80, "latin1");
  bytes.writeUInt16LE(machine, 0x84);
  return bytes;
}

function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
  return path;
}

/** A Release artifact laid out as installed: Toolbox.app, or a Windows install folder. */
function artifact(name, { target, renderer, fonts = true }) {
  const root = join(fixtures, name);
  rmSync(root, { recursive: true, force: true });
  const windows = target === WINDOWS;
  const resources = windows ? root : join(root, "Contents", "Resources");
  mkdirSync(resources, { recursive: true });
  if (renderer) write(windows ? join(root, "toolbox.exe") : join(root, "Contents", "MacOS", "toolbox"), renderer);
  if (fonts) cpSync(sourceFontsDir, join(resources, "resources", "fonts"), { recursive: true });
  return root;
}

const causes = (failures) => [...new Set(failures.map(({ cause }) => cause))].sort();
const readiness = (args) => spawnSync(process.execPath, ["scripts/check-release-readiness.mjs", ...args], { cwd: projectRoot, encoding: "utf8" });

test("a complete macOS app and Windows install pass", () => {
  const mac = artifact("mac-ok", { target: MAC, renderer: machO() });
  assert.deepEqual(verifyConvertArtifact({ root: mac, target: MAC }), []);
  assert.deepEqual(verifyConvertArtifact({ root: join(mac, "Contents", "Resources"), target: MAC }), []);
  assert.deepEqual(verifyConvertArtifact({ root: artifact("windows-ok", { target: WINDOWS, renderer: pe() }), target: WINDOWS }), []);
});

test("an artifact without the renderer fails as missing", () => {
  const failures = verifyConvertArtifact({ root: artifact("no-renderer", { target: WINDOWS, renderer: null }), target: WINDOWS });
  assert.deepEqual(causes(failures), ["missing"]);
  assert.match(failures[0].message, /app binary that renders Convert to PDF is missing at .*toolbox\.exe/);
});

test("a renderer for another architecture fails", () => {
  const failures = verifyConvertArtifact({ root: artifact("wrong-renderer", { target: WINDOWS, renderer: pe(0xaa64) }), target: WINDOWS });
  assert.deepEqual(causes(failures), ["wrong-architecture"]);
});

test("an artifact without the fallback fonts fails", () => {
  const failures = verifyConvertArtifact({ root: artifact("no-fonts", { target: MAC, renderer: machO(), fonts: false }), target: MAC });
  assert.deepEqual(causes(failures), ["fonts-missing"]);
  assert.match(failures[0].message, /fallback font manifest is missing/);
});

test("an artifact with a fallback font missing or changed fails", () => {
  const missing = artifact("font-missing", { target: MAC, renderer: machO() });
  rmSync(join(missing, "Contents", "Resources", "resources", "fonts", "LiberationSans-Regular.ttf"));
  assert.deepEqual(causes(verifyConvertArtifact({ root: missing, target: MAC })), ["fonts-missing"]);
  const changed = artifact("font-changed", { target: MAC, renderer: machO() });
  writeFileSync(join(changed, "Contents", "Resources", "resources", "fonts", "DejaVuSans.ttf"), "not a font");
  assert.deepEqual(causes(verifyConvertArtifact({ root: changed, target: MAC })), ["fonts-changed"]);
});

test("the conversion sources are every family module convert.rs reaches, and none of them uses a helper", () => {
  const sources = conversionSources();
  for (const module of ["convert", "layout", "font_resolver", "unicode_font", "compound", "delimited", "doc", "docx", "epub", "html", "markdown", "mobi", "odp", "ods", "odt", "ppt", "pptx", "rtf", "xls", "xlsx", "presentation", "spreadsheet", "wordprocessing"]) {
    assert.ok(sources.includes(`kit/pdf/${module}.rs`), `${module}.rs is not checked`);
  }
  assert.ok(!sources.includes("kit/pdf/mod.rs"), "the shared PDF module belongs to other tools");
  assert.deepEqual(conversionHelperUses(), []);
});

test("a conversion source that starts a helper fails the artifact as bundled-helper", () => {
  const sources = join(fixtures, "sources");
  cpSync(RUST_SOURCES, sources, { recursive: true });
  const docx = join(sources, "kit", "pdf", "docx.rs");
  const original = readFileSync(docx, "utf8");
  // Test code may start helpers; only shipped code counts.
  writeFileSync(docx, `${original}\n#[cfg(test)]\nfn spawns() { std::process::Command::new("soffice"); }\n`);
  assert.deepEqual(conversionHelperUses(sources), []);
  writeFileSync(docx, `fn render() { let _ = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF); }\n${original}`);
  const failures = verifyConvertArtifact({ root: artifact("helper", { target: MAC, renderer: machO() }), target: MAC, sources });
  assert.deepEqual(causes(failures), ["bundled-helper"]);
  assert.match(failures[0].message, /kit\/pdf\/docx\.rs resolves or starts a helper process/);
});

test("release readiness checks a Convert to PDF artifact and needs its target", () => {
  const passed = readiness(["--convert-artifact", artifact("cli-ok", { target: MAC, renderer: machO() }), "--target", MAC]);
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /Convert to PDF release check passed: .* carries the native aarch64-apple-darwin renderer and the fallback fonts/);
  assert.doesNotMatch(passed.stdout, /qpdf release check/);

  const noRenderer = readiness(["--convert-artifact", artifact("cli-no-renderer", { target: MAC, renderer: null }), "--target", MAC]);
  assert.equal(noRenderer.status, 1);
  assert.match(noRenderer.stderr, /Convert to PDF release check failed \(missing\)/);

  const noFonts = readiness(["--convert-artifact", artifact("cli-no-fonts", { target: MAC, renderer: machO(), fonts: false }), "--target", MAC]);
  assert.equal(noFonts.status, 1);
  assert.match(noFonts.stderr, /Convert to PDF release check failed \(fonts-missing\)/);

  const usage = readiness(["--convert-artifact", fixtures]);
  assert.equal(usage.status, 1);
  assert.match(usage.stderr, /--convert-artifact needs --target/);
});

test("the release workflow checks every artifact and compares snapshots before publishing", () => {
  const workflow = readFileSync(join(projectRoot, "..", ".github", "workflows", "tauri-release.yml"), "utf8");
  assert.match(workflow, /check-release-readiness\.mjs --convert-artifact "\$OCR_RELEASE_ROOT" --target \$\{\{ matrix\.target \}\}/);
  assert.match(workflow, /cargo test command_tests::release_artifact_converts_every_family_with_its_own_fonts -- --exact --nocapture/);
  assert.match(workflow, /name: convert-outputs-\$\{\{ matrix\.artifact \}\}/);
  for (const target of ["macos-aarch64", "windows-x86_64", "windows-arm64"]) assert.ok(workflow.includes(`${target}=convert-outputs/${target}`), target);
  assert.match(workflow, /publish:\n\s+needs: \[version, build, compare-ocr, compare-convert\]/);
});

test("the snapshot families are the release test's fixtures and cover every available family", () => {
  const convert = readFileSync(join(RUST_SOURCES, "kit", "pdf", "convert.rs"), "utf8");
  const fixtureBlock = convert.slice(convert.indexOf("pub fn family_documents"), convert.indexOf("mod tests"));
  const extensions = [...fixtureBlock.matchAll(/\("note\.(\w+)"/g)].map(([, extension]) => extension);
  assert.deepEqual([...extensions].sort(), [...FAMILIES].sort());
  assert.deepEqual(uncoveredFamilies(), []);
  const capabilities = [{ command: "convert_to_pdf", formatFamilies: [{ id: "comics", extensions: [".cbz"], nativeAvailability: "available" }, { id: "pdf", extensions: [".pdf"], nativeAvailability: "available" }] }];
  assert.deepEqual(uncoveredFamilies(capabilities), ["comics"]);
});

const page = (text, width = 595, height = 842) => ({ width, height, text });
function snapshots(name, change = () => {}) {
  const families = Object.fromEntries(FAMILIES.map((family) => [family, { pages: [page(`Toolbox ${family}`)] }]));
  change(families);
  const folder = join(fixtures, "snapshots", name);
  write(join(folder, "snapshots.json"), `${JSON.stringify({ os: name, architecture: "aarch64", families }, null, 2)}\n`);
  return readTargetOutputs(name, folder);
}

test("matching snapshots have no differences", () => {
  assert.deepEqual(compareOutputs([snapshots("macos-aarch64"), snapshots("windows-x86_64"), snapshots("windows-arm64")], []), []);
});

test("text and geometry differences fail unless documented", () => {
  const reference = snapshots("macos-aarch64");
  const windows = snapshots("windows-x86_64", (families) => {
    families.docx.pages[0].text = "Toolbox d o c x";
    families.pptx.pages = [page("Toolbox pptx", 960, 541)];
  });
  assert.deepEqual(compareOutputs([reference, windows], []).map(({ family, aspect, documented }) => [family, aspect, documented]), [["docx", "text", false], ["pptx", "geometry", false]]);
  const documented = [{ family: "docx", aspect: "text", target: "windows-x86_64", reason: "Kerning pairs extract as spaces." }];
  const found = compareOutputs([reference, windows], documented);
  assert.deepEqual(found.map(({ family, documented: known, reason }) => [family, known, reason]), [["docx", true, "Kerning pairs extract as spaces."], ["pptx", false, undefined]]);
  assert.deepEqual(compareOutputs([reference, windows], [{ ...documented[0], reference: "windows-arm64" }])[0].documented, false);
});

test("a snapshot missing a family is refused", () => {
  assert.throws(() => snapshots("partial", (families) => { delete families.epub; }), /partial has no pages for epub/);
  assert.throws(() => readTargetOutputs("empty", join(fixtures, "nothing")), /empty saved no Convert to PDF snapshots/);
});

test("documented differences need a family, an aspect, a target and a reason", () => {
  assert.deepEqual(readDifferences(), []);
  const path = write(join(fixtures, "differences.json"), JSON.stringify({ differences: [{ family: "docx", aspect: "fonts", target: "windows-x86_64", reason: "x" }] }));
  assert.throws(() => readDifferences(path), /needs a family .*, an aspect \(text or geometry\)/);
  write(path, JSON.stringify({ differences: [{ family: "docx", aspect: "text", target: "windows-x86_64", reason: " " }] }));
  assert.throws(() => readDifferences(path), /a reason/);
});

test("the compare script exits non-zero on an undocumented difference", () => {
  const run = (...targets) => spawnSync(process.execPath, ["scripts/compare-convert-outputs.mjs", ...targets.map(({ name, folder }) => `${name}=${folder}`)], { cwd: projectRoot, encoding: "utf8" });
  const same = run(snapshots("macos-aarch64"), snapshots("windows-arm64"));
  assert.equal(same.status, 0, same.stderr);
  assert.match(same.stdout, /Convert to PDF snapshots agree across macos-aarch64, windows-arm64\./);
  const differs = run(snapshots("macos-aarch64"), snapshots("windows-arm64", (families) => { families.md.pages.push(page("extra")); }));
  assert.equal(differs.status, 1);
  assert.match(differs.stderr, /Undocumented Convert to PDF difference: md text on windows-arm64 differs from macos-aarch64/);
});
