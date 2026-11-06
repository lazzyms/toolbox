// Release checks for the OCR Bundled helpers: the engine, the PDF rasterizer and the
// English data that every Release artifact must carry, pinned in ocr-pin.json and in the
// OCR resource manifest that stage-ocr-resources.mjs writes.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, isAbsolute, join, normalize } from "node:path";
import { TARGETS, binaryArchitectures } from "./qpdf-pin.mjs";

export const OCR_PIN_PATH = new URL("../ocr-pin.json", import.meta.url);
export const COMMITTED_OCR_ROOT = new URL("../src-tauri/resources/ocr/", import.meta.url);
export const REQUIRED_OCR_RESOURCES = ["tesseract", "engTraineddata"];
const PLACEHOLDER = /PLACEHOLDER/i;
const SHA256 = /^[0-9a-f]{64}$/;

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const fileSha256 = (path) => sha256(readFileSync(path));

/** Reads ocr-pin.json and refuses placeholders, so an unpinned value can never ship. */
export function readOcrPin(path = OCR_PIN_PATH) {
  const pin = JSON.parse(readFileSync(path, "utf8"));
  const problems = [];
  const pinned = (label, value, pattern) => {
    if (typeof value !== "string" || !value.trim()) problems.push(`${label} is not pinned`);
    else if (PLACEHOLDER.test(value)) problems.push(`${label} is a placeholder (${value})`);
    else if (pattern && !pattern.test(value)) problems.push(`${label} is malformed (${value})`);
  };
  const english = pin.englishData;
  pinned("englishData.version", english?.version);
  pinned("englishData.sha256", english?.sha256, SHA256);
  pinned("englishData.license", english?.license);
  pinned("englishData.source", english?.source, /^https:\/\//);
  if (!(english?.maxBytes > 0)) problems.push("englishData.maxBytes is not a positive size limit");
  for (const [target, entry] of Object.entries(pin.windows ?? {})) {
    if (TARGETS[target]?.format !== "pe") problems.push(`windows.${target} is not a Windows release target`);
    if (!Array.isArray(entry.packages) || !entry.packages.length) problems.push(`windows.${target} pins no packages`);
    for (const [index, item] of (entry.packages ?? []).entries()) {
      const label = `windows.${target}.packages[${index}]`;
      pinned(`${label}.name`, item.name);
      pinned(`${label}.version`, item.version);
      pinned(`${label}.url`, item.url, /^https:\/\//);
      pinned(`${label}.sha256`, item.sha256, SHA256);
      if (item.url && item.version && !item.url.includes(`${item.version}-any.pkg.tar.zst`)) problems.push(`${label}.url does not name version ${item.version}`);
    }
  }
  if (problems.length) throw new Error(`ocr-pin.json is not release-ready: ${problems.join("; ")}`);
  return pin;
}

/** The DLL names a PE image imports, from its import and delay-import tables. */
export function peImports(bytes) {
  if (bytes.length < 0x40 || bytes.toString("latin1", 0, 2) !== "MZ") return [];
  const header = bytes.readUInt32LE(0x3c);
  if (header + 24 > bytes.length || bytes.toString("latin1", header, header + 4) !== "PE\0\0") return [];
  const sections = bytes.readUInt16LE(header + 6);
  const optionalSize = bytes.readUInt16LE(header + 20);
  const optional = header + 24;
  const magic = bytes.readUInt16LE(optional);
  const directories = optional + (magic === 0x20b ? 112 : 96);
  const table = optional + optionalSize;
  const offsetOf = (rva) => {
    for (let index = 0; index < sections; index += 1) {
      const section = table + index * 40;
      const virtualSize = bytes.readUInt32LE(section + 8);
      const virtualAddress = bytes.readUInt32LE(section + 12);
      const rawSize = bytes.readUInt32LE(section + 16);
      const rawOffset = bytes.readUInt32LE(section + 20);
      if (rva >= virtualAddress && rva < virtualAddress + Math.max(virtualSize, rawSize)) return rva - virtualAddress + rawOffset;
    }
    return -1;
  };
  const name = (rva) => {
    const start = offsetOf(rva);
    if (start < 0) return null;
    const end = bytes.indexOf(0, start);
    return bytes.toString("latin1", start, end < 0 ? bytes.length : end);
  };
  const names = new Set();
  const walk = (directoryIndex, stride, nameField) => {
    const entry = directories + directoryIndex * 8;
    if (entry + 8 > bytes.length) return;
    const rva = bytes.readUInt32LE(entry);
    if (!rva) return;
    for (let descriptor = offsetOf(rva); descriptor >= 0 && descriptor + stride <= bytes.length; descriptor += stride) {
      const nameRva = bytes.readUInt32LE(descriptor + nameField);
      if (!nameRva) break;
      const dll = name(nameRva);
      if (dll) names.add(dll);
    }
  };
  walk(1, 20, 12);
  walk(13, 32, 4);
  return [...names];
}

/**
 * The DLLs from `searchDir` that `executables` load, directly or through each other.
 * DLLs that are not in `searchDir` come from Windows itself and are left out.
 */
export function dllClosure(executables, searchDir) {
  const available = new Map(readdirSync(searchDir).map((file) => [file.toLowerCase(), join(searchDir, file)]));
  const found = new Map();
  const queue = [...executables];
  while (queue.length) {
    const image = queue.pop();
    for (const dll of peImports(readFileSync(image))) {
      const path = available.get(dll.toLowerCase());
      if (!path || found.has(dll.toLowerCase())) continue;
      found.set(dll.toLowerCase(), path);
      queue.push(path);
    }
  }
  return [...found.values()].sort();
}

const firstExisting = (paths) => paths.find((path) => existsSync(path) && statSync(path).isFile());
const insideRoot = (relativePath) => typeof relativePath === "string" && relativePath.length > 0 && !isAbsolute(relativePath)
  && !normalize(relativePath).split(/[\\/]/).some((part) => part === ".." || part === "");

/**
 * Where an artifact keeps its OCR resources and rasterizer. `root` is an installed Windows
 * app folder, a Tauri release folder, a macOS Toolbox.app or its Contents/Resources, or the
 * src-tauri/resources folder before bundling. The lookup order follows the app's own.
 */
export function ocrArtifactLayout(root, target) {
  const resources = existsSync(join(root, "Contents", "Resources")) ? join(root, "Contents", "Resources") : root;
  const ocrRoot = [join(resources, "ocr"), join(resources, "resources", "ocr")].find((path) => existsSync(join(path, "manifest.json")))
    ?? [join(resources, "ocr"), join(resources, "resources", "ocr")].find((path) => existsSync(path))
    ?? join(resources, "resources", "ocr");
  const rasterizerName = TARGETS[target]?.format === "pe" ? "pdftoppm.exe" : "pdftoppm";
  const rasterizer = firstExisting([join(resources, "pdf-bin", rasterizerName), join(resources, "resources", rasterizerName), join(resources, rasterizerName)]);
  return { resources, ocrRoot, rasterizer, rasterizerName };
}

function nativeProblem(path, target, label) {
  const expected = TARGETS[target];
  const found = binaryArchitectures(readFileSync(path));
  if (found.format === expected.format && found.architectures.includes(expected.architecture)) return null;
  const described = found.format === "unknown" ? "not a native executable" : `${found.format} ${found.architectures.join("+")}`;
  return { cause: "wrong-architecture", message: `${label} at ${path} is ${described}; ${target} needs a native ${expected.format} ${expected.architecture} binary and would otherwise run emulated or not at all` };
}

/**
 * Checks one Release artifact for the OCR Bundled helpers. Returns every failure found, each
 * with a cause: missing, manifest, checksum-mismatch, metadata, over-size-limit, unpinned-data
 * or wrong-architecture. An empty list means the artifact passes.
 */
export function verifyOcrArtifact({ root, target, pin = readOcrPin() }) {
  const expected = TARGETS[target];
  if (!expected) throw new Error(`Unsupported release target ${target}; expected one of ${Object.keys(TARGETS).join(", ")}`);
  if (!root || !existsSync(root)) return [{ cause: "missing", message: `The Release artifact is missing at ${root}` }];
  const failures = [];
  const fail = (cause, message) => failures.push({ cause, message });
  const layout = ocrArtifactLayout(root, target);
  const manifestPath = join(layout.ocrRoot, "manifest.json");
  let manifest = null;
  if (!existsSync(manifestPath)) fail("missing", `The OCR resource manifest is missing at ${manifestPath}`);
  else {
    try { manifest = JSON.parse(readFileSync(manifestPath, "utf8")); }
    catch (error) { fail("manifest", `The OCR resource manifest at ${manifestPath} is not valid JSON: ${error.message}`); }
  }
  if (manifest) {
    if (manifest.version !== 1) fail("manifest", `The OCR resource manifest has version ${manifest.version}; the app reads version 1`);
    if (manifest.architecture !== "all" && manifest.architecture !== expected.architecture) {
      fail("manifest", `The OCR resource manifest is for ${manifest.architecture}; ${target} needs ${expected.architecture}`);
    }
    const entries = manifest.resources ?? {};
    for (const name of REQUIRED_OCR_RESOURCES) {
      if (!entries[name]) fail("missing", `The OCR resource manifest has no ${name} entry`);
    }
    for (const [name, spec] of Object.entries(entries)) {
      if (!insideRoot(spec.path)) { fail("manifest", `The OCR resource manifest lists ${name} outside its folder: ${spec.path}`); continue; }
      const path = join(layout.ocrRoot, spec.path);
      if (!existsSync(path) || !statSync(path).isFile()) { fail("missing", `${name} is missing at ${path}`); continue; }
      if (!String(spec.version ?? "").trim()) fail("metadata", `${name} has no version in the OCR resource manifest, so the app would refuse it`);
      if (!String(spec.license ?? "").trim()) fail("metadata", `${name} has no license in the OCR resource manifest, so the app would refuse it`);
      const bytes = readFileSync(path);
      if (!(bytes.length <= spec.maxBytes)) fail("over-size-limit", `${name} is ${bytes.length} bytes, over its size limit of ${spec.maxBytes}`);
      const actual = sha256(bytes);
      if (actual !== spec.sha256) fail("checksum-mismatch", `${name} at ${path} has SHA-256 ${actual}, but the manifest pins ${spec.sha256}`);
    }
    const english = entries.engTraineddata;
    if (english && english.sha256 !== pin.englishData.sha256) {
      fail("unpinned-data", `The English data in the manifest (${english.sha256}) is not the pinned ${pin.englishData.version} data (${pin.englishData.sha256})`);
    }
    const engine = entries.tesseract && insideRoot(entries.tesseract.path) ? join(layout.ocrRoot, entries.tesseract.path) : null;
    if (engine && existsSync(engine)) {
      const problem = nativeProblem(engine, target, "The OCR engine");
      if (problem) failures.push(problem);
      if (expected.format === "pe") failures.push(...nativeLibraries(dirname(engine), target, "OCR engine library"));
    }
  }
  if (!layout.rasterizer) fail("missing", `The PDF rasterizer ${layout.rasterizerName} is missing from ${layout.resources}`);
  else {
    const problem = nativeProblem(layout.rasterizer, target, "The PDF rasterizer");
    if (problem) failures.push(problem);
    if (expected.format === "pe") failures.push(...nativeLibraries(dirname(layout.rasterizer), target, "PDF rasterizer library"));
  }
  return failures;
}

function nativeLibraries(directory, target, label) {
  return readdirSync(directory)
    .filter((file) => file.toLowerCase().endsWith(".dll"))
    .map((file) => nativeProblem(join(directory, file), target, `The ${label} ${file}`))
    .filter(Boolean);
}

/** The committed English data must be the pinned file, so every target stages the same bytes. */
export function verifyCommittedEnglishData(pin = readOcrPin(), ocrRoot = COMMITTED_OCR_ROOT) {
  const path = new URL(pin.englishData.path, ocrRoot);
  if (!existsSync(path)) return `The pinned English data is missing at ${path.pathname}`;
  const actual = fileSha256(path);
  return actual === pin.englishData.sha256 ? null : `The committed English data has SHA-256 ${actual}, not the pinned ${pin.englishData.sha256}`;
}

export const describe = (failures) => failures.map(({ cause, message }) => `  (${cause}) ${message}`).join("\n");
