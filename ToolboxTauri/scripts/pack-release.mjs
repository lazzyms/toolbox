// Builds, signs and checks the optional packs a release publishes.
//
// A pack file (`.toolboxpack`) is the envelope src-tauri/src/packs/install.rs reads: a zip of
// `pack.json`, `payload.zip` (the pack manifest and files) and `payload.zip.sig` (the minisign
// signature of `payload.zip`, base64 encoded like the updater's `.sig` files and made with the
// updater key). The release workflow builds every pack with `buildRelease`, gates the release on
// `inspectPackRelease`, and compiles the catalog it writes into each release artifact.
//
//   node scripts/pack-release.mjs --out <dir> --version <x.y.z>
//
// signs with `tauri signer sign`, which reads TAURI_SIGNING_PRIVATE_KEY and
// TAURI_SIGNING_PRIVATE_KEY_PASSWORD, the secrets the updater artifacts are signed with.
import { spawnSync } from "node:child_process";
import { createHash, createPrivateKey, createPublicKey, sign as signBytes, verify as verifyBytes } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { crc32, inflateRawSync } from "node:zlib";
import { binaryArchitectures, TARGETS } from "./qpdf-pin.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const readJson = (path) => JSON.parse(readFileSync(join(projectRoot, path), "utf8"));

export const ALL_TARGETS = "all";
export const SUPPORTED_TARGETS = Object.values(TARGETS).map(({ pack }) => pack);
const PACK_FORMAT = 1;
const CATALOG_FILE = "pack-catalog.json";

// --- zip -------------------------------------------------------------------------------------

const DOS_DATE_1980_01_01 = 0x21;

/** A zip of `[name, bytes]` entries, stored uncompressed with fixed timestamps so equal input
 *  gives equal bytes. */
export function zip(entries) {
  const parts = [];
  const directory = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(DOS_DATE_1980_01_01, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(DOS_DATE_1980_01_01, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    parts.push(local, nameBytes, data);
    directory.push(central, nameBytes);
    offset += local.length + nameBytes.length + data.length;
  }
  const centralDirectory = Buffer.concat(directory);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, centralDirectory, end]);
}

/** The files in a zip, by name. Reads stored and deflated entries; skips folders. */
export function unzip(bytes) {
  let end = bytes.length - 22;
  while (end >= 0 && bytes.readUInt32LE(end) !== 0x06054b50) end -= 1;
  if (end < 0) throw new Error("not a zip archive");
  const files = new Map();
  let at = bytes.readUInt32LE(end + 16);
  for (let index = 0; index < bytes.readUInt16LE(end + 10); index += 1) {
    if (bytes.readUInt32LE(at) !== 0x02014b50) throw new Error("damaged zip directory");
    const method = bytes.readUInt16LE(at + 10);
    const size = bytes.readUInt32LE(at + 20);
    const nameLength = bytes.readUInt16LE(at + 28);
    const local = bytes.readUInt32LE(at + 42);
    const name = bytes.toString("utf8", at + 46, at + 46 + nameLength);
    const start = local + 30 + bytes.readUInt16LE(local + 26) + bytes.readUInt16LE(local + 28);
    const raw = bytes.subarray(start, start + size);
    if (!name.endsWith("/")) {
      if (method === 0) files.set(name, Buffer.from(raw));
      else if (method === 8) files.set(name, inflateRawSync(raw));
      else throw new Error(`${name} uses unsupported zip method ${method}`);
    }
    at += 46 + nameLength + bytes.readUInt16LE(at + 30) + bytes.readUInt16LE(at + 32);
  }
  return files;
}

// --- minisign --------------------------------------------------------------------------------

const ED25519_PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const encode = (text) => Buffer.from(text, "utf8").toString("base64");
const decode = (text) => Buffer.from(String(text).trim(), "base64").toString("utf8");

/** A minisign key from a 32-byte seed, for fixtures. `publicKey` is in the updater
 *  configuration's format; `sign` makes a prehashed signature in the updater's `.sig` format,
 *  as `tauri signer sign` does. */
export function minisignKey(seed, keyId) {
  const privateKey = createPrivateKey({ key: Buffer.concat([ED25519_PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
  const raw = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(ED25519_SPKI_PREFIX.length);
  const id = Buffer.from(keyId);
  if (seed.length !== 32 || id.length !== 8) throw new Error("A minisign key needs a 32-byte seed and an 8-byte key id");
  const keyLine = Buffer.concat([Buffer.from("Ed"), id, raw]).toString("base64");
  return {
    publicKey: encode(`untrusted comment: minisign public key ${id.toString("hex").toUpperCase()}\n${keyLine}\n`),
    sign(data) {
      const signature = signBytes(null, createHash("blake2b512").update(data).digest(), privateKey);
      const trusted = "timestamp:0\tfile:payload.zip";
      const global = signBytes(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
      const line = Buffer.concat([Buffer.from("ED"), id, signature]).toString("base64");
      return encode(`untrusted comment: signature from minisign secret key\n${line}\ntrusted comment: ${trusted}\n${global.toString("base64")}\n`);
    },
  };
}

/** Whether `encodedSignature` (the updater's `.sig` format) signs `data` with `encodedPublicKey`
 *  (the updater configuration's format). Checks the signature and its trusted comment, in the
 *  legacy and prehashed modes the app accepts. */
export function verifyMinisign(data, encodedSignature, encodedPublicKey) {
  try {
    const key = Buffer.from(decode(encodedPublicKey).split(/\r?\n/)[1], "base64");
    const lines = decode(encodedSignature).split(/\r?\n/);
    const line = Buffer.from(lines[1], "base64");
    const algorithm = line.toString("latin1", 0, 2);
    if (key.length !== 42 || key.toString("latin1", 0, 2) !== "Ed" || line.length !== 74) return false;
    if (!["Ed", "ED"].includes(algorithm) || !line.subarray(2, 10).equals(key.subarray(2, 10))) return false;
    if (!lines[2]?.startsWith("trusted comment: ")) return false;
    const publicKey = createPublicKey({ key: Buffer.concat([ED25519_SPKI_PREFIX, key.subarray(10)]), format: "der", type: "spki" });
    const signature = line.subarray(10);
    const message = algorithm === "ED" ? createHash("blake2b512").update(data).digest() : data;
    const trusted = Buffer.from(lines[2].slice("trusted comment: ".length));
    return verifyBytes(null, message, publicKey, signature)
      && verifyBytes(null, Buffer.concat([signature, trusted]), publicKey, Buffer.from(lines[3], "base64"));
  } catch {
    return false;
  }
}

/** Signs with the updater key through `tauri signer sign`, which reads the key and its password
 *  from TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD. */
export function updaterKeySigner() {
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error("TAURI_SIGNING_PRIVATE_KEY is required to sign pack payloads");
  const folder = mkdtempSync(join(tmpdir(), "toolbox-pack-sign-"));
  process.once("exit", () => rmSync(folder, { recursive: true, force: true }));
  return (data) => {
    const file = join(folder, "payload.zip");
    rmSync(`${file}.sig`, { force: true });
    writeFileSync(file, data);
    const npx = process.platform === "win32" ? "npx.cmd" : "npx";
    const result = spawnSync(npx, ["tauri", "signer", "sign", file], { cwd: projectRoot, stdio: ["ignore", "ignore", "inherit"], shell: process.platform === "win32" });
    if (result.status !== 0 || !existsSync(`${file}.sig`)) throw new Error("tauri signer could not sign a pack payload");
    return readFileSync(`${file}.sig`, "utf8").trim();
  };
}

// --- building --------------------------------------------------------------------------------

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const json = (value) => Buffer.from(JSON.stringify(value, null, 2));

/** One pack file for one target. `pack` is the catalog entry without payloads; `files` are
 *  `{ path, data, license, sourceUrl? }`. Returns the file and its catalog payload. */
export function buildPack({ pack, target, files, sign, releaseTag }) {
  const resources = Object.fromEntries(files.map(({ path, data, license, sourceUrl }) => [path, {
    path, version: pack.version, sha256: sha256(data), license, ...(sourceUrl ? { sourceUrl } : {}), maxBytes: data.length,
  }]));
  const manifest = json({
    version: 1,
    architecture: target === ALL_TARGETS ? ALL_TARGETS : target.split("-")[1],
    resources,
    pack: { id: pack.id, version: pack.version, target, packFormat: PACK_FORMAT, appVersions: pack.appVersions },
  });
  const payload = zip([["manifest.json", manifest], ...files.map(({ path, data }) => [path, data])]);
  const header = json({ formatVersion: 1, id: pack.id, version: pack.version, target });
  const envelope = zip([["pack.json", header], ["payload.zip", payload], ["payload.zip.sig", Buffer.from(sign(payload))]]);
  const fileName = `${pack.id}-${pack.version}-${target}.toolboxpack`;
  return {
    fileName,
    envelope,
    payload: {
      target,
      downloadBytes: envelope.length,
      installedBytes: manifest.length + files.reduce((total, { data }) => total + data.length, 0),
      sha256: sha256(payload),
      path: `${releaseTag}/${fileName}`,
    },
  };
}

/** Every pack the release publishes, with where each file comes from. OCR language packs carry
 *  the tessdata_fast model pinned in shared/ocr-language-packs.json; the script pack carries the
 *  fonts and license texts pinned in shared/script-packs.json. */
export function packSources(appVersion) {
  return [...ocrPackSources(appVersion), scriptPackSource(appVersion)];
}

function scriptPackSource(appVersion) {
  const plan = readJson("shared/script-packs.json");
  const pinned = ({ path, source, file, license, bytes, gitBlobSha1: blob }) => {
    const { repository, commit } = plan.sources[source];
    return { path, license, sourceUrl: `${repository}/blob/${commit}/${file}`, download: { url: `${repository}/raw/${commit}/${file}`, bytes, gitBlobSha1: blob } };
  };
  return {
    pack: {
      id: plan.pack.id,
      name: plan.pack.name,
      description: plan.description,
      toolIds: ["pdf-edit", "convert-to-pdf", "formatted-pdf"],
      scriptFonts: Object.fromEntries(plan.fonts.flatMap(({ path, scripts }) => scripts.map((script) => [script, path]))),
      license: plan.license,
      version: plan.packVersion,
      minPackFormat: PACK_FORMAT,
      appVersions: { from: appVersion },
    },
    target: ALL_TARGETS,
    files: [...plan.fonts.map((font) => ({ ...pinned(font), font: true })), ...plan.licenses.map(pinned)],
  };
}

/** Whether `bytes` is a single TrueType-outline font, the only kind Edit PDF can embed. */
export function trueTypeOutlines(bytes) {
  if (bytes.length < 12 || bytes.readUInt32BE(0) !== 0x00010000) return false;
  const tags = new Set();
  for (let index = 0; index < bytes.readUInt16BE(4); index += 1) tags.add(bytes.toString("latin1", 12 + index * 16, 16 + index * 16));
  return ["glyf", "loca", "head", "hmtx", "cmap"].every((tag) => tags.has(tag)) && !tags.has("fvar");
}

function ocrPackSources(appVersion) {
  const plan = readJson("shared/ocr-language-packs.json");
  const { source, commit, license, packVersion } = plan.models;
  return plan.languages.filter(({ bundled }) => !bundled).map(({ code, name, packId, model }) => ({
    pack: {
      id: packId,
      name: `OCR language: ${name}`,
      description: `Lets OCR PDF read ${name} text in scanned pages.`,
      toolIds: ["pdf-ocr"],
      ocrLanguage: { code, name },
      license,
      version: packVersion,
      minPackFormat: PACK_FORMAT,
      appVersions: { from: appVersion },
    },
    target: ALL_TARGETS,
    files: [{
      path: `tessdata/${model.file}`,
      license,
      sourceUrl: `${source}/blob/${commit}/${model.file}`,
      download: { url: `${source}/raw/${commit}/${model.file}`, bytes: model.bytes, gitBlobSha1: model.gitBlobSha1 },
    }],
  }));
}

const gitBlobSha1 = (bytes) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");

async function fetchPinned({ url, bytes, gitBlobSha1: pinned }) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  if (data.length !== bytes || gitBlobSha1(data) !== pinned) throw new Error(`${url} does not match its pin (${bytes} bytes, blob ${pinned})`);
  return data;
}

/** Builds every pack in `sources` into `outDir` and writes the catalog beside them. */
export async function buildRelease({ outDir, version, sources = packSources(version), sign, fetchFile = fetchPinned }) {
  mkdirSync(outDir, { recursive: true });
  const packs = [];
  for (const source of sources) {
    const files = [];
    for (const { font, ...file } of source.files) {
      const data = file.data ?? await fetchFile(file.download);
      if (font && !trueTypeOutlines(data)) throw new Error(`${source.pack.id}: ${file.path} is not a static TrueType-outline font`);
      files.push({ ...file, data });
    }
    const built = buildPack({ pack: source.pack, target: source.target, files, sign, releaseTag: `tauri-v${version}` });
    writeFileSync(join(outDir, built.fileName), built.envelope);
    let entry = packs.find(({ id }) => id === source.pack.id);
    if (!entry) packs.push(entry = { ...source.pack, payloads: [] });
    entry.payloads.push(built.payload);
  }
  const catalog = { formatVersion: 1, packs };
  writeFileSync(join(outDir, CATALOG_FILE), `${JSON.stringify(catalog, null, 2)}\n`);
  return catalog;
}

// --- release readiness -----------------------------------------------------------------------

/** Download budgets by pack id: each OCR language pack's own, and the script budget for the
 *  script pack and every script family pack. */
export function packBudgets() {
  const ocr = readJson("shared/ocr-language-packs.json");
  const script = readJson("shared/script-packs.json");
  const budgets = new Map(ocr.languages.filter(({ packId }) => packId).map(({ packId, downloadBudgetBytes }) => [packId, downloadBudgetBytes]));
  for (const id of [script.pack.id, ...script.families.map((family) => family.id)]) budgets.set(id, script.downloadBudgetBytes);
  return budgets;
}

function machOSlices(bytes) {
  if (bytes.readUInt32LE(0) === 0xfeedfacf) return [bytes];
  const fat = bytes.readUInt32BE(0);
  const wide = fat === 0xcafebabf;
  const slices = [];
  for (let index = 0; index < bytes.readUInt32BE(4); index += 1) {
    const at = 8 + index * (wide ? 32 : 20);
    const offset = wide ? Number(bytes.readBigUInt64BE(at + 8)) : bytes.readUInt32BE(at + 8);
    const size = wide ? Number(bytes.readBigUInt64BE(at + 16)) : bytes.readUInt32BE(at + 12);
    slices.push(bytes.subarray(offset, offset + size));
  }
  return slices;
}

/** A Mach-O slice signed with a certificate: a code signature whose CodeDirectory is not ad hoc
 *  and that carries a CMS signature. */
function machOSliceSigned(slice) {
  if (slice.length < 32 || slice.readUInt32LE(0) !== 0xfeedfacf) return false;
  let at = 32;
  for (let index = 0; index < slice.readUInt32LE(16) && at + 16 <= slice.length; index += 1) {
    if (slice.readUInt32LE(at) === 0x1d) {
      const blob = slice.subarray(slice.readUInt32LE(at + 8), slice.readUInt32LE(at + 8) + slice.readUInt32LE(at + 12));
      if (blob.length < 12 || blob.readUInt32BE(0) !== 0xfade0cc0) return false;
      let flags = null;
      let cms = false;
      for (let entry = 0; entry < blob.readUInt32BE(8); entry += 1) {
        const type = blob.readUInt32BE(12 + entry * 8);
        const offset = blob.readUInt32BE(16 + entry * 8);
        if (type === 0 && blob.readUInt32BE(offset) === 0xfade0c02) flags = blob.readUInt32BE(offset + 12);
        if (type === 0x10000 && blob.readUInt32BE(offset) === 0xfade0b01 && blob.readUInt32BE(offset + 4) > 8) cms = true;
      }
      const AD_HOC = 0x2;
      return flags !== null && (flags & AD_HOC) === 0 && cms;
    }
    at += slice.readUInt32LE(at + 4);
  }
  return false;
}

/** A PE file with an Authenticode certificate table. */
function peSigned(bytes) {
  const optional = bytes.readUInt32LE(0x3c) + 24;
  const wide = bytes.readUInt16LE(optional) === 0x20b;
  const count = bytes.readUInt32LE(optional + (wide ? 108 : 92));
  const security = optional + (wide ? 112 : 96) + 4 * 8;
  return count > 4 && security + 8 <= bytes.length && bytes.readUInt32LE(security) > 0 && bytes.readUInt32LE(security + 4) > 0;
}

export function codeSigned(bytes) {
  const { format } = binaryArchitectures(bytes);
  try {
    if (format === "mach-o") return machOSlices(bytes).every(machOSliceSigned);
    if (format === "pe") return peSigned(bytes);
  } catch {
    return false;
  }
  return false;
}

function inspectNative(name, bytes, target, fail) {
  const found = binaryArchitectures(bytes);
  if (found.format === "unknown") return;
  const described = `${name} (${found.format} ${found.architectures.join("+")})`;
  if (target === ALL_TARGETS) {
    fail("native-in-data-pack", `${described} is a native binary in a payload for every target; native code needs a payload per Supported target`);
    return;
  }
  const expected = Object.values(TARGETS).find(({ pack }) => pack === target);
  if (found.format !== expected.format || !found.architectures.includes(expected.architecture)) {
    const emulated = expected.architecture === "aarch64" && found.format === expected.format;
    fail(emulated ? "emulated-arm64" : "wrong-architecture", `${described} is not native ${expected.architecture} for ${target}${emulated ? " and would run emulated" : ""}`);
  }
  if (!codeSigned(bytes)) fail("unsigned-native", `${described} is not code-signed with a certificate; unsigned and ad hoc signed binaries are refused`);
}

/** Checks a release's pack folder: the catalog and every pack file it names. Returns every
 *  failure as `{ cause, pack, target, message }`; none means the release may ship. */
export function inspectPackRelease({ dir, publicKey, budgets = packBudgets() }) {
  const failures = [];
  const catalogPath = join(dir, CATALOG_FILE);
  if (!existsSync(catalogPath)) return [{ cause: "missing-catalog", pack: null, target: null, message: `${catalogPath} does not exist` }];
  const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));
  for (const pack of catalog.packs ?? []) {
    const fail = (cause, target, message) => failures.push({ cause, pack: pack.id, target, message: `${pack.id}${target ? ` for ${target}` : ""}: ${message}` });
    if (!String(pack.license ?? "").trim()) fail("missing-license", null, "the catalog entry has no license");
    if (!pack.payloads?.length) fail("missing-payload", null, "the catalog entry declares no payload");
    for (const payload of pack.payloads ?? []) {
      const target = payload.target;
      if (target !== ALL_TARGETS && !SUPPORTED_TARGETS.includes(target)) {
        fail("unsupported-target", target, `${target} is not a Supported target`);
        continue;
      }
      const file = join(dir, String(payload.path ?? "").split("/").pop());
      if (!payload.path || !existsSync(file)) {
        fail("missing-payload", target, `${payload.path} is not in the release`);
        continue;
      }
      const envelope = readFileSync(file);
      let entries;
      try {
        entries = unzip(envelope);
      } catch (error) {
        fail("unreadable-payload", target, `${payload.path} is not a pack file: ${error.message}`);
        continue;
      }
      const archive = entries.get("payload.zip");
      if (!archive) {
        fail("missing-payload", target, `${payload.path} has no payload.zip`);
        continue;
      }
      const signature = entries.get("payload.zip.sig");
      if (!signature) fail("missing-signature", target, `${payload.path} has no payload.zip.sig`);
      else if (!verifyMinisign(archive, signature.toString("utf8"), publicKey)) fail("bad-signature", target, `${payload.path} is not signed with the updater key`);
      if (payload.sha256 !== sha256(archive)) fail("checksum-mismatch", target, "the catalog checksum is not the payload's");
      if (payload.downloadBytes !== envelope.length) fail("size-mismatch", target, `the catalog says ${payload.downloadBytes} bytes; the file is ${envelope.length}`);
      const budget = budgets.get(pack.id);
      if (budget !== undefined && Math.max(envelope.length, payload.downloadBytes ?? 0) > budget) {
        fail("over-budget", target, `the download is ${Math.max(envelope.length, payload.downloadBytes ?? 0)} bytes, over its ${budget} byte budget`);
      }
      let files;
      try {
        files = unzip(archive);
      } catch (error) {
        fail("unreadable-payload", target, `payload.zip is not an archive: ${error.message}`);
        continue;
      }
      let manifest = null;
      try {
        manifest = JSON.parse(files.get("manifest.json")?.toString("utf8") ?? "null");
      } catch {
        manifest = null;
      }
      if (!manifest) fail("missing-manifest", target, "payload.zip has no readable manifest.json");
      else {
        if (manifest.pack?.id !== pack.id || manifest.pack?.version !== pack.version || manifest.pack?.target !== target) {
          fail("manifest-mismatch", target, "the signed manifest names another pack, version or target than the catalog");
        }
        for (const [name, resource] of Object.entries(manifest.resources ?? {})) {
          if (!String(resource?.license ?? "").trim()) fail("missing-license", target, `${name} has no license in the manifest`);
        }
      }
      for (const [name, bytes] of files) inspectNative(name, bytes, target, (cause, message) => fail(cause, target, message));
    }
  }
  return failures;
}

// --- uninstallers ----------------------------------------------------------------------------

/** What each Supported target's uninstaller does with the app-data folder that holds `packs/`.
 *  Windows: the NSIS uninstaller removes it when the person ticks "Delete the application data".
 *  macOS: there is no uninstaller. */
export const UNINSTALL_APP_DATA = {
  "windows-x86_64": "nsis",
  "windows-aarch64": "nsis",
  "macos-aarch64": "Toolbox for macOS has no uninstaller: moving Toolbox.app to the Trash leaves ~/Library/Application Support/<identifier>, packs included. Delete that folder to remove packs.",
};

const NSIS_REMOVES_APP_DATA = /\$\{If\} \$DeleteAppDataCheckboxState = 1[\s\S]{0,1200}?RmDir \/r "\$LOCALAPPDATA\\\$\{BUNDLEID\}"/;

/** The NSIS template compiled into the Tauri CLI that builds the Windows installers. */
export function tauriNsisTemplate() {
  const cli = dirname(fileURLToPath(import.meta.resolve("@tauri-apps/cli/package.json")));
  for (const folder of readdirSync(dirname(cli)).filter((name) => name.startsWith("cli-"))) {
    for (const file of readdirSync(join(dirname(cli), folder)).filter((name) => name.endsWith(".node"))) {
      const text = readFileSync(join(dirname(cli), folder, file)).toString("latin1");
      const start = text.indexOf("Section Uninstall");
      if (start >= 0) return text.slice(start, start + 20_000);
    }
  }
  return null;
}

/** Whether each target's uninstaller removes the app-data folder packs live in. Returns
 *  `{ target, removes, cause, message }` per Supported target; a target whose platform has no
 *  uninstaller passes with its stated difference. */
export function inspectUninstallers({ config, packsSource, nsisTemplate }) {
  const identifier = config.identifier;
  const keepsPacksInLocalAppData = /app_local_data_dir\(\)/.test(packsSource);
  return SUPPORTED_TARGETS.map((target) => {
    const plan = UNINSTALL_APP_DATA[target];
    if (plan !== "nsis") return { target, removes: false, cause: null, message: plan.replace("<identifier>", identifier) };
    const nsis = config.bundle?.windows?.nsis ?? {};
    const problem = nsis.template ? "a custom NSIS template replaces the one whose uninstaller removes app data"
      : !keepsPacksInLocalAppData ? "packs are not kept in the local app-data folder the uninstaller removes"
      : !nsisTemplate ? "the Tauri CLI's NSIS template could not be read"
      : !NSIS_REMOVES_APP_DATA.test(nsisTemplate) ? "the NSIS uninstaller no longer removes $LOCALAPPDATA\\<identifier>"
      : null;
    return problem
      ? { target, removes: false, cause: "uninstaller-keeps-app-data", message: `${target}: ${problem}` }
      : { target, removes: true, cause: null, message: `${target}: "Delete the application data" removes %LOCALAPPDATA%\\${identifier}, packs included` };
  });
}

// --- command line ----------------------------------------------------------------------------

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { out: { type: "string" }, version: { type: "string" } } });
  if (!values.out || !/^\d+\.\d+\.\d+$/.test(values.version ?? "")) throw new Error("Usage: pack-release.mjs --out <dir> --version <x.y.z>");
  const catalog = await buildRelease({ outDir: resolve(values.out), version: values.version, sign: updaterKeySigner() });
  console.log(`Built ${catalog.packs.length} packs into ${values.out}: ${catalog.packs.map(({ id }) => id).join(", ")}`);
}
