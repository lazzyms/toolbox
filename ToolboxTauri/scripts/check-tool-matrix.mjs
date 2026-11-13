import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { ALL_TARGETS, SUPPORTED_TARGETS } from "./pack-release.mjs";

const projectFile = (path) => new URL(`../${path}`, import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(projectFile(path), "utf8"));

/** The packs tools need, from the plans in shared/: every OCR language pack for OCR PDF and the
 *  script pack for Edit PDF. */
export function requiredPacks() {
  const ocr = readJson("shared/ocr-language-packs.json");
  const script = readJson("shared/script-packs.json");
  return [
    ...ocr.languages.filter(({ packId }) => packId).map(({ packId, code }) => ({ id: packId, toolId: "pdf-ocr", ocrLanguage: code })),
    { id: script.pack.id, toolId: "pdf-edit" },
  ];
}

/** Whether `catalog` has every pack a tool uses, for that tool, on every Supported target.
 *  Returns `{ cause, pack, message }` failures; none means the catalog covers the matrix. */
export function packMatrixFailures(catalog, { registryIds, required = requiredPacks() }) {
  const failures = [];
  const fail = (cause, pack, message) => failures.push({ cause, pack, message: `${pack}: ${message}` });
  for (const need of required) {
    if (!registryIds.includes(need.toolId)) fail("unknown-tool", need.id, `${need.toolId} is not a registered tool`);
    const pack = catalog.packs?.find(({ id }) => id === need.id);
    if (!pack) {
      fail("missing-pack", need.id, `${need.toolId} uses this pack, but the catalog does not have it`);
      continue;
    }
    if (!pack.toolIds?.includes(need.toolId)) fail("wrong-tool", need.id, `the catalog entry does not name ${need.toolId}`);
    if (need.ocrLanguage && pack.ocrLanguage?.code !== need.ocrLanguage) fail("wrong-language", need.id, `the catalog entry is not the ${need.ocrLanguage} OCR language`);
    const targets = new Set((pack.payloads ?? []).map(({ target }) => target));
    const missing = targets.has(ALL_TARGETS) ? [] : SUPPORTED_TARGETS.filter((target) => !targets.has(target));
    if (missing.length) fail("missing-target", need.id, `no payload for ${missing.join(", ")}`);
  }
  for (const pack of catalog.packs ?? []) {
    for (const toolId of pack.toolIds ?? []) {
      if (!registryIds.includes(toolId)) fail("unknown-tool", pack.id, `${toolId} is not a registered tool`);
    }
  }
  return failures;
}

function registry() {
  const source = readFileSync(projectFile("src/registry/index.ts"), "utf8");
  const atomic = source.split("export const ToolWorkspaceRegistry")[0];
  const ids = [...atomic.matchAll(/(?:id: "([^"]+)"|(?:pdf|image)\("([^"]+)")/g)].map((match) => match[1] ?? match[2]);
  return { source, atomic, ids, factoryEntries: (atomic.match(/(?:pdf|image)\("/g) ?? []).length };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { source, atomic, ids: registryIds, factoryEntries } = registry();
  const requiredFields = ["command", "verification", "view"];
  const expectedToolCount = 39;

  if (registryIds.length !== expectedToolCount) throw new Error(`Tauri registry has ${registryIds.length} tools, expected ${expectedToolCount}`);
  if (new Set(registryIds).size !== registryIds.length) throw new Error("Tauri registry contains duplicate IDs");
  for (const field of requiredFields) {
    const count = (source.match(new RegExp(`${field}:`, "g")) ?? []).length + factoryEntries;
    if (count < expectedToolCount) throw new Error(`Registry is missing ${expectedToolCount - count} ${field} entries`);
  }
  const unregistered = requiredPacks().filter(({ toolId }) => !registryIds.includes(toolId));
  if (unregistered.length) throw new Error(`Packs name unregistered tools: ${unregistered.map(({ id, toolId }) => `${id} (${toolId})`).join(", ")}`);

  console.log(`Tool matrix passed: ${registryIds.length} tools, ${registryIds.filter((id) => atomic.includes(`id: \"${id}\"`)).length} explicit entries`);

  // `--catalog <file>`: the catalog a release builds must have every pack a tool uses for every
  // Supported target.
  const { values } = parseArgs({ options: { catalog: { type: "string" } } });
  if (values.catalog !== undefined) {
    const failures = packMatrixFailures(JSON.parse(readFileSync(values.catalog, "utf8")), { registryIds });
    if (failures.length) {
      for (const { cause, message } of failures) console.error(`Pack matrix failed (${cause}): ${message}`);
      process.exit(1);
    }
    console.log(`Pack matrix passed: ${requiredPacks().length} packs cover ${SUPPORTED_TARGETS.join(", ")}`);
  }
}
