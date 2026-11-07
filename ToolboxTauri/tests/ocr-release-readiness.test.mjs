import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { COMMITTED_OCR_ROOT, dllClosure, peImports, readOcrPin, verifyCommittedEnglishData, verifyOcrArtifact } from "../scripts/ocr-release.mjs";
import { compareOutputs, readDifferences, readTargetOutputs } from "../scripts/compare-ocr-outputs.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtures = mkdtempSync(join(tmpdir(), "toolbox-ocr-release-"));
const pin = readOcrPin();
const englishData = fileURLToPath(new URL(pin.englishData.path, COMMITTED_OCR_ROOT));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

const ARM64_MACH_O = 0x0100000c;
const ARM64_PE = 0xaa64;
const X86_64_PE = 0x8664;

function machO(cpu) {
  const bytes = Buffer.alloc(64);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(cpu, 4);
  return bytes;
}

/** A tiny PE64 image for `machine` whose import table names `imports`. */
function pe(machine, imports = []) {
  const bytes = Buffer.alloc(0x400);
  bytes.write("MZ", 0, "latin1");
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.write("PE\0\0", 0x80, "latin1");
  bytes.writeUInt16LE(machine, 0x84);
  bytes.writeUInt16LE(1, 0x86);
  bytes.writeUInt16LE(240, 0x94);
  const optional = 0x98;
  bytes.writeUInt16LE(0x20b, optional);
  const section = optional + 240;
  bytes.write(".idata", section, "latin1");
  bytes.writeUInt32LE(0x200, section + 8);
  bytes.writeUInt32LE(0x1000, section + 12);
  bytes.writeUInt32LE(0x200, section + 16);
  bytes.writeUInt32LE(0x200, section + 20);
  if (imports.length) {
    bytes.writeUInt32LE(0x1000, optional + 112 + 8);
    let name = 0x1000 + (imports.length + 1) * 20;
    imports.forEach((dll, index) => {
      bytes.writeUInt32LE(name, 0x200 + index * 20 + 12);
      bytes.write(`${dll}\0`, 0x200 + (name - 0x1000), "latin1");
      name += dll.length + 1;
    });
  }
  return bytes;
}

function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
  return path;
}

/**
 * Lays out a Release artifact the way it is installed: a Windows install folder with
 * resources/ocr and resources/pdftoppm.exe, or Toolbox.app with Contents/Resources.
 */
function artifact(name, { target, engine, rasterizer, engineDll, withData = true, change = () => {} }) {
  const root = join(fixtures, name);
  rmSync(root, { recursive: true, force: true });
  const windows = target.includes("windows");
  const resources = windows ? root : join(root, "Contents", "Resources");
  const ocr = join(resources, "resources", "ocr");
  const engineName = windows ? "tesseract.exe" : "tesseract";
  if (engine) write(join(ocr, "bin", engineName), engine);
  if (engineDll) write(join(ocr, "bin", "libtesseract-5.5.dll"), engineDll);
  if (withData) { mkdirSync(join(ocr, "tessdata"), { recursive: true }); copyFileSync(englishData, join(ocr, "tessdata", "eng.traineddata")); }
  if (rasterizer) write(windows ? join(resources, "resources", "pdftoppm.exe") : join(resources, "pdf-bin", "pdftoppm"), rasterizer);
  // A file left out after staging keeps the checksum it was staged with.
  const entry = (path, version, staged = "0".repeat(64)) => {
    const file = join(ocr, path);
    let digest = staged;
    try { digest = sha256(readFileSync(file)); } catch {}
    return { path, version, sha256: digest, license: "Apache-2.0", maxBytes: 50 * 1024 * 1024 };
  };
  const manifest = {
    version: 1,
    architecture: target.split("-")[0],
    resources: { tesseract: entry(`bin/${engineName}`, "5.5.4"), engTraineddata: entry(pin.englishData.path, pin.englishData.version, pin.englishData.sha256) },
  };
  change(manifest, { ocr, resources });
  write(join(ocr, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return root;
}

const causes = (failures) => [...new Set(failures.map(({ cause }) => cause))].sort();
const arm64Windows = { target: "aarch64-pc-windows-msvc", engine: pe(ARM64_PE), rasterizer: pe(ARM64_PE), engineDll: pe(ARM64_PE) };

test("the committed English data is the pinned tessdata_fast file", () => {
  assert.equal(verifyCommittedEnglishData(pin), null);
  assert.equal(sha256(readFileSync(englishData)), pin.englishData.sha256);
});

test("a complete native Windows arm64 artifact passes", () => {
  assert.deepEqual(verifyOcrArtifact({ root: artifact("arm64-ok", arm64Windows), target: "aarch64-pc-windows-msvc", pin }), []);
});

test("a complete native macOS app passes", () => {
  const root = artifact("mac-ok", { target: "aarch64-apple-darwin", engine: machO(ARM64_MACH_O), rasterizer: machO(ARM64_MACH_O) });
  assert.deepEqual(verifyOcrArtifact({ root: join(root), target: "aarch64-apple-darwin", pin }), []);
  assert.deepEqual(verifyOcrArtifact({ root: join(root, "Contents", "Resources"), target: "aarch64-apple-darwin", pin }), []);
});

test("an artifact without the engine fails as missing", () => {
  const failures = verifyOcrArtifact({ root: artifact("no-engine", { ...arm64Windows, engine: null, engineDll: null }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["missing"]);
  assert.match(failures[0].message, /tesseract is missing/);
});

test("an artifact without the rasterizer fails as missing", () => {
  const failures = verifyOcrArtifact({ root: artifact("no-rasterizer", { ...arm64Windows, rasterizer: null }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["missing"]);
  assert.match(failures[0].message, /PDF rasterizer pdftoppm\.exe is missing/);
});

test("an artifact without the English data fails as missing", () => {
  const failures = verifyOcrArtifact({ root: artifact("no-data", { ...arm64Windows, withData: false }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["missing"]);
  assert.match(failures[0].message, /engTraineddata is missing/);
});

test("an artifact whose manifest has no English data entry fails as missing", () => {
  const root = artifact("no-data-entry", { ...arm64Windows, change: (manifest) => { delete manifest.resources.engTraineddata; } });
  assert.deepEqual(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })), ["missing"]);
});

test("an artifact without a manifest fails as missing", () => {
  const root = artifact("no-manifest", arm64Windows);
  rmSync(join(root, "resources", "ocr", "manifest.json"));
  assert.deepEqual(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })), ["missing"]);
});

test("a manifest entry whose checksum does not match its file fails", () => {
  const root = artifact("engine-checksum", { ...arm64Windows, change: (manifest) => { manifest.resources.tesseract.sha256 = "f".repeat(64); } });
  const failures = verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["checksum-mismatch"]);
});

test("English data changed after staging fails its checksum", () => {
  const root = artifact("data-checksum", { ...arm64Windows, change: (manifest, { ocr }) => { writeFileSync(join(ocr, "tessdata", "eng.traineddata"), "not the pinned data"); } });
  assert.deepEqual(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })), ["checksum-mismatch"]);
});

test("English data that is not the pinned file fails even when the manifest matches it", () => {
  const root = artifact("data-unpinned", { ...arm64Windows, change: (manifest, { ocr }) => {
    writeFileSync(join(ocr, "tessdata", "eng.traineddata"), "other English data");
    manifest.resources.engTraineddata.sha256 = sha256(Buffer.from("other English data"));
  } });
  assert.deepEqual(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })), ["unpinned-data"]);
});

test("an entry without a version fails, because the app refuses it", () => {
  const root = artifact("no-version", { ...arm64Windows, change: (manifest) => { delete manifest.resources.tesseract.version; } });
  const failures = verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["metadata"]);
  assert.match(failures[0].message, /no version/);
});

test("a manifest for another architecture fails", () => {
  const root = artifact("wrong-manifest-arch", { ...arm64Windows, change: (manifest) => { manifest.architecture = "x86_64"; } });
  assert.deepEqual(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })), ["manifest"]);
});

test("an entry that points outside the OCR folder fails", () => {
  const root = artifact("escape", { ...arm64Windows, change: (manifest) => { manifest.resources.tesseract.path = "../pdftoppm.exe"; } });
  assert.ok(causes(verifyOcrArtifact({ root, target: "aarch64-pc-windows-msvc", pin })).includes("manifest"));
});

test("an x64 engine fails on Windows arm64", () => {
  const failures = verifyOcrArtifact({ root: artifact("x64-engine", { ...arm64Windows, engine: pe(X86_64_PE) }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["wrong-architecture"]);
  assert.match(failures[0].message, /OCR engine .* is pe x86_64; aarch64-pc-windows-msvc needs a native pe aarch64 binary/);
});

test("an x64 rasterizer fails on Windows arm64", () => {
  const failures = verifyOcrArtifact({ root: artifact("x64-rasterizer", { ...arm64Windows, rasterizer: pe(X86_64_PE) }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["wrong-architecture"]);
  assert.match(failures[0].message, /PDF rasterizer/);
});

test("an x64 engine library fails on Windows arm64", () => {
  const failures = verifyOcrArtifact({ root: artifact("x64-dll", { ...arm64Windows, engineDll: pe(X86_64_PE) }), target: "aarch64-pc-windows-msvc", pin });
  assert.deepEqual(causes(failures), ["wrong-architecture"]);
  assert.match(failures[0].message, /libtesseract-5\.5\.dll/);
});

test("native x64 helpers pass on Windows x86_64", () => {
  const root = artifact("x64-all", { target: "x86_64-pc-windows-msvc", engine: pe(X86_64_PE), rasterizer: pe(X86_64_PE), engineDll: pe(X86_64_PE) });
  assert.deepEqual(verifyOcrArtifact({ root, target: "x86_64-pc-windows-msvc", pin }), []);
});

test("a missing artifact fails as missing", () => {
  assert.deepEqual(causes(verifyOcrArtifact({ root: join(fixtures, "absent"), target: "aarch64-pc-windows-msvc", pin })), ["missing"]);
});

test("a placeholder in the OCR pin fails the release check", () => {
  const path = write(join(fixtures, "placeholder-pin.json"), JSON.stringify({
    ...pin, windows: { "aarch64-pc-windows-msvc": { ...pin.windows["aarch64-pc-windows-msvc"], packages: [{ ...pin.windows["aarch64-pc-windows-msvc"].packages[0], sha256: "PLACEHOLDER-verify-before-release" }] } },
  }));
  assert.throws(() => readOcrPin(path), /placeholder/);
});

test("the Windows arm64 pin names native MSYS2 CLANGARM64 packages for the engine and rasterizer", () => {
  const arm64 = pin.windows["aarch64-pc-windows-msvc"];
  assert.equal(arm64.msystem, "CLANGARM64");
  const executables = arm64.packages.flatMap((item) => item.executables);
  for (const name of ["tesseract.exe", "pdftoppm.exe", "pdftotext.exe"]) assert.ok(executables.includes(name), name);
  for (const item of arm64.packages) assert.match(item.url, /^https:\/\/repo\.msys2\.org\/mingw\/clangarm64\//);
});

test("the PE import walk finds each DLL a helper loads, directly or through another DLL", () => {
  const bin = join(fixtures, "closure");
  write(join(bin, "tesseract.exe"), pe(ARM64_PE, ["libtesseract-5.5.dll", "KERNEL32.dll"]));
  write(join(bin, "libtesseract-5.5.dll"), pe(ARM64_PE, ["libleptonica-6.dll", "libc++.dll"]));
  write(join(bin, "libleptonica-6.dll"), pe(ARM64_PE, ["libc++.dll"]));
  write(join(bin, "libc++.dll"), pe(ARM64_PE));
  write(join(bin, "unrelated.dll"), pe(ARM64_PE));
  assert.deepEqual(peImports(readFileSync(join(bin, "tesseract.exe"))), ["libtesseract-5.5.dll", "KERNEL32.dll"]);
  assert.deepEqual(dllClosure([join(bin, "tesseract.exe")], bin).map((path) => path.split(/[\\/]/).pop()).sort(), ["libc++.dll", "libleptonica-6.dll", "libtesseract-5.5.dll"]);
});

function readiness(args) {
  return spawnSync(process.execPath, ["scripts/check-release-readiness.mjs", ...args], { cwd: projectRoot, encoding: "utf8" });
}

test("the release check exits 1 and names the cause for an artifact with an emulated engine", () => {
  const result = readiness(["--ocr-artifact", artifact("cli-x64", { ...arm64Windows, engine: pe(X86_64_PE) }), "--target", "aarch64-pc-windows-msvc"]);
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /OCR release check failed \(wrong-architecture\)/);
});

test("the release check exits 1 for an artifact missing its English data", () => {
  const result = readiness(["--ocr-artifact", artifact("cli-no-data", { ...arm64Windows, withData: false }), "--target", "aarch64-pc-windows-msvc"]);
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /OCR release check failed \(missing\)/);
});

test("the release check passes a complete native artifact without running the qpdf check", () => {
  const result = readiness(["--ocr-artifact", artifact("cli-ok", arm64Windows), "--target", "aarch64-pc-windows-msvc"]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /OCR release check passed/);
  assert.doesNotMatch(result.stdout, /qpdf release check/);
});

test("the release check needs a target for an artifact", () => {
  const result = readiness(["--ocr-artifact", artifact("cli-no-target", arm64Windows)]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /--ocr-artifact needs --target/);
});

function outputs(name, text, pdf) {
  const folder = join(fixtures, "outputs", name);
  write(join(folder, "text.txt"), text);
  write(join(folder, "searchable.pdf"), pdf);
  write(join(folder, "fingerprints.json"), JSON.stringify({ files: { "text.txt": sha256(Buffer.from(text)), "searchable.pdf": sha256(Buffer.from(pdf)) } }));
  return readTargetOutputs(name, folder);
}

test("matching outputs across targets agree", () => {
  const targets = ["macos-aarch64", "windows-x86_64", "windows-arm64"].map((name) => outputs(name, "Toolbox page one\n", "%PDF-1.7 same"));
  assert.deepEqual(compareOutputs(targets, []), []);
});

test("an undocumented difference fails and a documented one is a platform difference", () => {
  const targets = [outputs("macos-aarch64", "Toolbox page one\n", "%PDF a"), outputs("windows-arm64", "Toolbox page one\n", "%PDF b")];
  assert.deepEqual(compareOutputs(targets, []).map(({ output, documented }) => [output, documented]), [["searchable.pdf", false]]);
  const documented = compareOutputs(targets, [{ output: "searchable.pdf", target: "windows-arm64", reason: "Tesseract 5.5.4 places one word box a pixel lower." }]);
  assert.deepEqual(documented.map(({ documented: known, reason }) => [known, reason]), [[true, "Tesseract 5.5.4 places one word box a pixel lower."]]);
});

test("saved outputs must match their recorded fingerprints", () => {
  const target = outputs("tampered", "Toolbox page one\n", "%PDF a");
  writeFileSync(join(target.folder, "text.txt"), "changed\n");
  assert.throws(() => readTargetOutputs("tampered", target.folder), /not the recorded/);
});

test("the documented platform differences each name an output, a target and a reason", () => {
  assert.doesNotThrow(() => readDifferences());
  const path = write(join(fixtures, "bad-differences.json"), JSON.stringify({ differences: [{ output: "text.txt", target: "windows-arm64" }] }));
  assert.throws(() => readDifferences(path), /reason/);
});

test("the compare command exits 1 on an undocumented difference", () => {
  outputs("cli-mac", "Toolbox page one\n", "%PDF a");
  outputs("cli-win", "Toolbox page 0ne\n", "%PDF a");
  const result = spawnSync(process.execPath, ["scripts/compare-ocr-outputs.mjs", `macos-aarch64=${join(fixtures, "outputs", "cli-mac")}`, `windows-arm64=${join(fixtures, "outputs", "cli-win")}`], { cwd: projectRoot, encoding: "utf8" });
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /Undocumented OCR difference: text\.txt on windows-arm64/);
});

test.after(() => rmSync(fixtures, { recursive: true, force: true }));
