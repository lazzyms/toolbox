import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { packMatrixFailures, requiredPacks } from "../scripts/check-tool-matrix.mjs";
import {
  buildRelease, codeSigned, inspectPackRelease, inspectUninstallers, minisignKey, packBudgets, packSources,
  SUPPORTED_TARGETS, tauriNsisTemplate, trueTypeOutlines, unzip, verifyMinisign, zip,
} from "../scripts/pack-release.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const committedFixture = join(projectRoot, "tests/fixtures/pack-release");
const work = mkdtempSync(join(tmpdir(), "toolbox-pack-release-"));
let folders = 0;
const folder = () => join(work, String(folders++));

// A fixed test key: the committed fixture and the native install test verify against it.
const key = minisignKey(Buffer.alloc(32, 0x5a), Buffer.from("TOOLBOX!"));
const otherKey = minisignKey(Buffer.alloc(32, 0x33), Buffer.from("TOOLBOX!"));
const HINDI_MODEL = Buffer.from("hindi model".repeat(100));
const APP_VERSION = "1.0.14";

function ocrSource({ id = "ocr-hindi", license = "Apache-2.0", fileLicense = license, data = HINDI_MODEL } = {}) {
  return {
    pack: {
      id, name: "OCR language: Hindi", description: "Lets OCR PDF read Hindi text in scanned pages.", toolIds: ["pdf-ocr"],
      ocrLanguage: { code: "hin", name: "Hindi" }, license, version: "1.0", minPackFormat: 1, appVersions: { from: "1.0.0" },
    },
    target: "all",
    files: [{ path: "tessdata/hin.traineddata", data, license: fileLicense, sourceUrl: "https://example.test/hin.traineddata" }],
  };
}

function nativeSource(target, binary) {
  return {
    pack: { id: "native-tool", name: "Native tool", description: "d", toolIds: ["pdf-ocr"], license: "MIT", version: "1.0", minPackFormat: 1, appVersions: { from: "1.0.0" } },
    target,
    files: [{ path: "bin/tool", data: binary, license: "MIT" }],
  };
}

/** Builds `sources` into a fresh folder signed by `signer`. */
async function release(sources, signer = key) {
  const outDir = folder();
  await buildRelease({ outDir, version: APP_VERSION, sources, sign: signer.sign });
  return outDir;
}

const catalogOf = (dir) => JSON.parse(readFileSync(join(dir, "pack-catalog.json"), "utf8"));
const writeCatalog = (dir, catalog) => writeFileSync(join(dir, "pack-catalog.json"), JSON.stringify(catalog));
const envelopeFile = (dir) => join(dir, readdirSync(dir).find((name) => name.endsWith(".toolboxpack")));
const causes = (dir, budgets) => inspectPackRelease({ dir, publicKey: key.publicKey, ...(budgets ? { budgets } : {}) }).map(({ cause }) => cause);

// --- native binaries -------------------------------------------------------------------------

const ARM64_MACH_O = 0x0100000c;
const ARM64_PE = 0xaa64;
const X86_64_PE = 0x8664;

/** A Mach-O with a code signature: none, ad hoc, or a certificate (a CMS blob). */
function machO(cpu, signing = "certificate") {
  const header = Buffer.alloc(32 + 16);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(cpu, 4);
  if (signing === "none") return header;
  header.writeUInt32LE(1, 16);
  header.writeUInt32LE(16, 20);
  const directory = Buffer.alloc(44);
  directory.writeUInt32BE(0xfade0c02, 0);
  directory.writeUInt32BE(directory.length, 4);
  directory.writeUInt32BE(0x20400, 8);
  directory.writeUInt32BE(signing === "ad-hoc" ? 0x2 : 0, 12);
  const cms = Buffer.alloc(signing === "ad-hoc" ? 8 : 64);
  cms.writeUInt32BE(0xfade0b01, 0);
  cms.writeUInt32BE(cms.length, 4);
  const index = Buffer.alloc(12 + 16);
  index.writeUInt32BE(0xfade0cc0, 0);
  index.writeUInt32BE(2, 8);
  index.writeUInt32BE(0, 12);
  index.writeUInt32BE(index.length, 16);
  index.writeUInt32BE(0x10000, 20);
  index.writeUInt32BE(index.length + directory.length, 24);
  const blob = Buffer.concat([index, directory, cms]);
  blob.writeUInt32BE(blob.length, 4);
  header.writeUInt32LE(0x1d, 32);
  header.writeUInt32LE(16, 36);
  header.writeUInt32LE(header.length, 40);
  header.writeUInt32LE(blob.length, 44);
  return Buffer.concat([header, blob]);
}

/** A PE32+ file, with or without an Authenticode certificate table. */
function pe(machine, signed = true) {
  const bytes = Buffer.alloc(0x200);
  bytes.write("MZ", 0, "latin1");
  bytes.writeUInt32LE(0x40, 0x3c);
  bytes.write("PE\0\0", 0x40, "latin1");
  bytes.writeUInt16LE(machine, 0x44);
  const optional = 0x40 + 24;
  bytes.writeUInt16LE(0x20b, optional);
  bytes.writeUInt32LE(16, optional + 108);
  if (signed) {
    bytes.writeUInt32LE(0x180, optional + 112 + 32);
    bytes.writeUInt32LE(0x80, optional + 112 + 36);
  }
  return bytes;
}

test("binaries count as code-signed only with a certificate", () => {
  assert.equal(codeSigned(machO(ARM64_MACH_O)), true);
  assert.equal(codeSigned(machO(ARM64_MACH_O, "ad-hoc")), false);
  assert.equal(codeSigned(machO(ARM64_MACH_O, "none")), false);
  assert.equal(codeSigned(pe(ARM64_PE)), true);
  assert.equal(codeSigned(pe(ARM64_PE, false)), false);
});

// --- signatures and archives -----------------------------------------------------------------

test("minisign signatures verify only against their key and data", () => {
  const signature = key.sign(Buffer.from("payload"));
  assert.equal(verifyMinisign(Buffer.from("payload"), signature, key.publicKey), true);
  assert.equal(verifyMinisign(Buffer.from("payloaD"), signature, key.publicKey), false);
  assert.equal(verifyMinisign(Buffer.from("payload"), signature, otherKey.publicKey), false);
  assert.equal(verifyMinisign(Buffer.from("payload"), "not a signature", key.publicKey), false);
});

test("pack zips read back exactly and the same input gives the same bytes", () => {
  const entries = [["a.txt", Buffer.from("alpha")], ["dir/b.bin", Buffer.from([0, 1, 2])]];
  assert.deepEqual([...unzip(zip(entries))], entries);
  assert.deepEqual(zip(entries), zip(entries));
});

// --- release readiness -----------------------------------------------------------------------

test("a valid release passes and its catalog describes each signed payload", async () => {
  const dir = await release([ocrSource(), nativeSource("macos-aarch64", machO(ARM64_MACH_O)), nativeSource("windows-x86_64", pe(X86_64_PE)), nativeSource("windows-aarch64", pe(ARM64_PE))]);
  assert.deepEqual(causes(dir), []);
  const catalog = catalogOf(dir);
  assert.deepEqual(catalog.packs.map(({ id, payloads }) => [id, payloads.map(({ target }) => target)]), [["ocr-hindi", ["all"]], ["native-tool", SUPPORTED_TARGETS]]);
  const [payload] = catalog.packs[0].payloads;
  assert.equal(payload.path, `tauri-v${APP_VERSION}/ocr-hindi-1.0-all.toolboxpack`);
  assert.equal(payload.downloadBytes, readFileSync(join(dir, "ocr-hindi-1.0-all.toolboxpack")).length);
});

const failing = [
  {
    name: "a missing payload file",
    cause: "missing-payload",
    async build() {
      const dir = await release([ocrSource()]);
      rmSync(envelopeFile(dir));
      return dir;
    },
  },
  {
    name: "a catalog entry without payloads",
    cause: "missing-payload",
    async build() {
      const dir = await release([ocrSource()]);
      const catalog = catalogOf(dir);
      catalog.packs[0].payloads = [];
      writeCatalog(dir, catalog);
      return dir;
    },
  },
  {
    name: "a payload without a signature",
    cause: "missing-signature",
    async build() {
      const dir = await release([ocrSource()]);
      const file = envelopeFile(dir);
      writeFileSync(file, zip([...unzip(readFileSync(file))].filter(([name]) => name !== "payload.zip.sig")));
      const catalog = catalogOf(dir);
      catalog.packs[0].payloads[0].downloadBytes = readFileSync(file).length;
      writeCatalog(dir, catalog);
      return dir;
    },
  },
  { name: "a payload signed with another key", cause: "bad-signature", build: () => release([ocrSource()], otherKey) },
  { name: "a pack without a license", cause: "missing-license", build: () => release([ocrSource({ license: "" })]) },
  { name: "a payload file without a license", cause: "missing-license", build: () => release([ocrSource({ fileLicense: "" })]) },
  { name: "a payload over its size budget", cause: "over-budget", build: () => release([ocrSource({ data: Buffer.alloc(packBudgets().get("ocr-hindi") + 1) })]) },
  { name: "an emulated Windows arm64 payload", cause: "emulated-arm64", build: () => release([nativeSource("windows-aarch64", pe(X86_64_PE))]) },
  { name: "an unsigned native binary", cause: "unsigned-native", build: () => release([nativeSource("windows-x86_64", pe(X86_64_PE, false))]) },
  { name: "an ad hoc signed macOS binary", cause: "unsigned-native", build: () => release([nativeSource("macos-aarch64", machO(ARM64_MACH_O, "ad-hoc"))]) },
  { name: "a native binary in a payload for every target", cause: "native-in-data-pack", build: () => release([nativeSource("all", pe(X86_64_PE))]) },
  { name: "a payload for another target's architecture", cause: "wrong-architecture", build: () => release([nativeSource("macos-aarch64", pe(ARM64_PE))]) },
  { name: "a payload for a target the app does not support", cause: "unsupported-target", build: () => release([nativeSource("linux-x86_64", pe(X86_64_PE))]) },
  {
    name: "a catalog checksum that is not the payload's",
    cause: "checksum-mismatch",
    async build() {
      const dir = await release([ocrSource()]);
      const catalog = catalogOf(dir);
      catalog.packs[0].payloads[0].sha256 = "0".repeat(64);
      writeCatalog(dir, catalog);
      return dir;
    },
  },
  {
    name: "a catalog size that is not the file's",
    cause: "size-mismatch",
    async build() {
      const dir = await release([ocrSource()]);
      const catalog = catalogOf(dir);
      catalog.packs[0].payloads[0].downloadBytes += 1;
      writeCatalog(dir, catalog);
      return dir;
    },
  },
  {
    name: "a missing catalog",
    cause: "missing-catalog",
    async build() {
      const dir = await release([ocrSource()]);
      rmSync(join(dir, "pack-catalog.json"));
      return dir;
    },
  },
];

for (const { name, cause, build } of failing) {
  test(`release readiness fails ${name} with ${cause}`, async () => {
    const found = causes(await build());
    assert.ok(found.includes(cause), `expected ${cause}, found ${found.join(", ") || "no failure"}`);
  });
}

test("every release pack comes from a pinned source the plan names", () => {
  const ocr = JSON.parse(readFileSync(join(projectRoot, "shared/ocr-language-packs.json"), "utf8"));
  const script = JSON.parse(readFileSync(join(projectRoot, "shared/script-packs.json"), "utf8"));
  const sources = packSources(APP_VERSION);
  assert.deepEqual(sources.map(({ pack }) => pack.id), [...ocr.languages.filter(({ bundled }) => !bundled).map(({ packId }) => packId), script.pack.id]);
  for (const { pack, target, files } of sources) {
    assert.equal(target, "all");
    assert.deepEqual(pack.appVersions, { from: APP_VERSION });
    assert.ok(pack.license && pack.version && files.length);
    for (const { download } of files) assert.ok(/^https:\/\/github\.com\/[^/]+\/[^/]+\/raw\/[0-9a-f]{40}\//.test(download.url) && download.gitBlobSha1 && download.bytes, download.url);
  }
  const ocrFiles = sources.filter(({ pack }) => pack.ocrLanguage).map(({ files }) => files);
  assert.ok(ocrFiles.every((files) => files.length === 1 && files[0].download.url.includes(ocr.models.commit)));
});

test("the script pack names a font for every planned script and ships each font's license", () => {
  const plan = JSON.parse(readFileSync(join(projectRoot, "shared/script-packs.json"), "utf8"));
  const { pack, files } = packSources(APP_VERSION).find(({ pack }) => pack.id === plan.pack.id);
  assert.equal(pack.name, plan.pack.name);
  assert.deepEqual(pack.toolIds, ["pdf-edit", "convert-to-pdf", "formatted-pdf"]);
  const planned = plan.fonts.flatMap(({ path, scripts }) => scripts.map((name) => [name, path]));
  assert.deepEqual(Object.entries(pack.scriptFonts).sort(), planned.sort());
  for (const name of [...plan.baseScripts, ...plan.families.flatMap(({ scripts }) => scripts)]) {
    assert.equal(name in pack.scriptFonts, !plan.baseScripts.includes(name), name);
  }
  assert.deepEqual(files.filter(({ font }) => font).map(({ path }) => path), plan.fonts.map(({ path }) => path));
  assert.deepEqual(files.filter(({ font }) => !font).map(({ path }) => path), plan.licenses.map(({ path }) => path));
  assert.ok(plan.fonts.reduce((sum, { bytes }) => sum + bytes, 0) < plan.downloadBudgetBytes);
});

/** An sfnt header and table directory with `tags`, which is all `trueTypeOutlines` reads. */
function sfnt(version, tags) {
  const bytes = Buffer.alloc(12 + tags.length * 16);
  bytes.writeUInt32BE(version, 0);
  bytes.writeUInt16BE(tags.length, 4);
  tags.forEach((tag, index) => bytes.write(tag, 12 + index * 16, "latin1"));
  return bytes;
}

const TRUETYPE_TABLES = ["cmap", "glyf", "head", "hhea", "hmtx", "loca", "maxp"];

test("only static TrueType-outline fonts count as script pack fonts", () => {
  assert.equal(trueTypeOutlines(sfnt(0x00010000, TRUETYPE_TABLES)), true);
  assert.equal(trueTypeOutlines(sfnt(0x4f54544f, ["CFF ", "cmap", "head", "hhea", "hmtx", "maxp"])), false);
  assert.equal(trueTypeOutlines(sfnt(0x00010000, [...TRUETYPE_TABLES, "fvar", "gvar"])), false);
  assert.equal(trueTypeOutlines(sfnt(0x00010000, TRUETYPE_TABLES.filter((tag) => tag !== "glyf"))), false);
  assert.equal(trueTypeOutlines(Buffer.from("wOFF")), false);
});

test("a release refuses a script pack font that isn't static TrueType", async () => {
  const fontSource = (data) => ({
    pack: {
      id: "unicode-scripts", name: "Unicode scripts", description: "d", toolIds: ["pdf-edit", "convert-to-pdf", "formatted-pdf"],
      scriptFonts: { Arabic: "fonts/Arabic.ttf" }, license: "OFL-1.1", version: "1.0", minPackFormat: 1, appVersions: { from: "1.0.0" },
    },
    target: "all",
    files: [{ path: "fonts/Arabic.ttf", font: true, license: "OFL-1.1", download: { url: "https://example.test/Arabic.ttf" } }],
    fetched: data,
  });
  const build = (source) => buildRelease({ outDir: folder(), version: APP_VERSION, sources: [source], sign: key.sign, fetchFile: async () => source.fetched });
  await assert.rejects(build(fontSource(sfnt(0x00010000, [...TRUETYPE_TABLES, "fvar"]))), /fonts\/Arabic\.ttf is not a static TrueType-outline font/);
  await build(fontSource(sfnt(0x00010000, TRUETYPE_TABLES)));
});

// --- tool matrix -----------------------------------------------------------------------------

const registryIds = ["pdf-ocr", "pdf-edit"];

function coveringCatalog() {
  return {
    formatVersion: 1,
    packs: requiredPacks().map(({ id, toolId, ocrLanguage }) => ({
      id, toolIds: [toolId], ...(ocrLanguage ? { ocrLanguage: { code: ocrLanguage, name: ocrLanguage } } : {}), payloads: [{ target: "all" }],
    })),
  };
}

test("the tool matrix passes a catalog with every pack a tool uses", () => {
  assert.deepEqual(packMatrixFailures(coveringCatalog(), { registryIds }), []);
});

test("the tool matrix fails a catalog that misses a pack, a target or a tool", () => {
  const matrixCauses = (change) => {
    const catalog = coveringCatalog();
    change(catalog);
    return packMatrixFailures(catalog, { registryIds }).map(({ cause }) => cause);
  };
  assert.deepEqual(matrixCauses((catalog) => catalog.packs.pop()), ["missing-pack"]);
  assert.deepEqual(matrixCauses((catalog) => { catalog.packs[0].payloads = [{ target: "macos-aarch64" }, { target: "windows-x86_64" }]; }), ["missing-target"]);
  assert.deepEqual(matrixCauses((catalog) => { catalog.packs[0].payloads = SUPPORTED_TARGETS.map((target) => ({ target })); }), []);
  assert.deepEqual(matrixCauses((catalog) => { catalog.packs[0].toolIds = ["pdf-edit"]; }), ["wrong-tool"]);
  assert.deepEqual(matrixCauses((catalog) => { catalog.packs[0].ocrLanguage.code = "xxx"; }), ["wrong-language"]);
  assert.deepEqual(matrixCauses((catalog) => { catalog.packs[0].toolIds.push("retired-tool"); }), ["unknown-tool"]);
});

test("the tool matrix passes the packs a release builds", () => {
  const packs = packSources(APP_VERSION).map(({ pack }) => ({ ...pack, payloads: [{ target: "all" }] }));
  assert.deepEqual(packMatrixFailures({ formatVersion: 1, packs }, { registryIds: [...registryIds, "convert-to-pdf", "formatted-pdf"] }), []);
});

// --- uninstallers ----------------------------------------------------------------------------

test("the Windows uninstallers remove the app data that holds packs and macOS states it has none", () => {
  const config = JSON.parse(readFileSync(join(projectRoot, "src-tauri/tauri.conf.json"), "utf8"));
  const packsSource = readFileSync(join(projectRoot, "src-tauri/src/packs.rs"), "utf8");
  const template = tauriNsisTemplate();
  const results = inspectUninstallers({ config, packsSource, nsisTemplate: template });
  assert.deepEqual(results.map(({ target, removes, cause }) => [target, removes, cause]), [
    ["macos-aarch64", false, null], ["windows-x86_64", true, null], ["windows-aarch64", true, null],
  ]);
  assert.match(results[0].message, /no uninstaller[\s\S]*~\/Library\/Application Support\/com\.toolbox\.desktop/);

  const windowsCauses = (options) => inspectUninstallers({ config, packsSource, nsisTemplate: template, ...options })
    .filter(({ target }) => target.startsWith("windows")).map(({ cause }) => cause);
  const custom = { ...config, bundle: { ...config.bundle, windows: { ...config.bundle.windows, nsis: { ...config.bundle.windows.nsis, template: "custom.nsi" } } } };
  assert.deepEqual(windowsCauses({ config: custom }), ["uninstaller-keeps-app-data", "uninstaller-keeps-app-data"]);
  assert.deepEqual(windowsCauses({ nsisTemplate: template.replaceAll("$LOCALAPPDATA", "$TEMP") }), ["uninstaller-keeps-app-data", "uninstaller-keeps-app-data"]);
  assert.deepEqual(windowsCauses({ nsisTemplate: null }), ["uninstaller-keeps-app-data", "uninstaller-keeps-app-data"]);
  assert.deepEqual(windowsCauses({ packsSource: packsSource.replaceAll("app_local_data_dir()", "app_config_dir()") }), ["uninstaller-keeps-app-data", "uninstaller-keeps-app-data"]);
});

// --- the committed fixture -------------------------------------------------------------------

// The release the native install test (src-tauri/src/packs/download.rs) serves from a stand-in
// server. Set UPDATE_PACK_RELEASE_FIXTURE=1 to rewrite it.
test("the committed pack release fixture is what the release build makes and it passes", async () => {
  const dir = await release([ocrSource()]);
  writeFileSync(join(dir, "public-key.txt"), `${key.publicKey}\n`);
  if (process.env.UPDATE_PACK_RELEASE_FIXTURE === "1") {
    rmSync(committedFixture, { recursive: true, force: true });
    mkdirSync(committedFixture, { recursive: true });
    for (const name of readdirSync(dir)) writeFileSync(join(committedFixture, name), readFileSync(join(dir, name)));
  }
  assert.deepEqual(readdirSync(committedFixture).sort(), readdirSync(dir).sort());
  for (const name of readdirSync(dir)) assert.deepEqual(readFileSync(join(committedFixture, name)), readFileSync(join(dir, name)), name);
  assert.deepEqual(causes(committedFixture), []);
});

test.after(() => rmSync(work, { recursive: true, force: true }));
