import { existsSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { inspectPackRelease, inspectUninstallers, tauriNsisTemplate } from "./pack-release.mjs";
import { inspectQpdf, readPin } from "./qpdf-pin.mjs";

const projectFile = (path) => new URL(`../${path}`, import.meta.url);
const config = JSON.parse(readFileSync(projectFile("src-tauri/tauri.conf.json"), "utf8"));
const canonicalTypes = JSON.parse(readFileSync(projectFile("shared/shell-file-types.json"), "utf8"));
const requiredIcons = config.bundle?.icon ?? [];
const missingIcons = requiredIcons.filter((icon) => !existsSync(projectFile(`src-tauri/${icon}`)));
if (missingIcons.length) throw new Error(`Missing bundle icons: ${missingIcons.join(", ")}`);
if (!config.bundle?.active || config.bundle?.targets !== "all") throw new Error("Tauri bundling must be active for all targets");
if (!config.plugins?.updater?.endpoints?.length) throw new Error("Updater endpoint is not configured");
if (!config.plugins?.updater?.pubkey) throw new Error("Updater public key is not configured");
const canonicalExtensions = Object.values(canonicalTypes).flat().map((extension) => extension.replace(/^\./, "")).sort();
const tauriExtensions = (config.bundle?.fileAssociations ?? []).flatMap(({ ext }) => ext ?? []).map((extension) => extension.replace(/^\./, "")).sort();
if (JSON.stringify(tauriExtensions) !== JSON.stringify(canonicalExtensions)) {
  throw new Error(`Tauri file associations differ from shared/shell-file-types.json. Expected ${canonicalExtensions.join(", ")}; received ${tauriExtensions.join(", ")}.`);
}
const msixManifest = readFileSync(projectFile("packaging/windows-msix/AppxManifest.xml.template"), "utf8");
const msixExtensions = [...msixManifest.matchAll(/<uap:FileType>([^<]+)<\/uap:FileType>/g)].map(([, extension]) => extension.replace(/^\./, "")).sort();
if (JSON.stringify(msixExtensions) !== JSON.stringify(canonicalExtensions)) {
  throw new Error(`MSIX file associations differ from shared/shell-file-types.json. Expected ${canonicalExtensions.join(", ")}; received ${msixExtensions.join(", ")}.`);
}
const workflow = readFileSync(projectFile("../.github/workflows/tauri-release.yml"), "utf8");
const notesHeader = readFileSync(projectFile("../.github/release-notes-header.md"), "utf8");
if (!workflow.includes("node ToolboxTauri/scripts/release-notes.mjs") || !workflow.includes("--notes-file release-notes.md")) throw new Error("Releases must use the first-launch header and the PR release-notes changelog");
if (!/Privacy & Security[\s\S]*Open Anyway/.test(notesHeader)) throw new Error("Release notes header must explain the macOS Open Anyway step");
if (!/check-release-readiness\.mjs --packs pack-release/.test(workflow) || !/check-tool-matrix\.mjs --catalog pack-release\/pack-catalog\.json/.test(workflow)) {
  throw new Error("Releases must build their packs, gate them with --packs and check the catalog against the tool matrix");
}
if (!/gh release create[^\n]*pack-release\/\*\.toolboxpack/.test(workflow)) throw new Error("Releases must publish the pack files they build");
const uninstallers = inspectUninstallers({
  config,
  packsSource: readFileSync(projectFile("src-tauri/src/packs.rs"), "utf8"),
  nsisTemplate: tauriNsisTemplate(),
});
const keepsAppData = uninstallers.filter(({ cause }) => cause);
if (keepsAppData.length) throw new Error(`Uninstaller check failed: ${keepsAppData.map(({ cause, message }) => `${cause}: ${message}`).join("; ")}`);
const pin = readPin();
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater and ${canonicalExtensions.length} file associations configured, qpdf minimum ${pin.minimumVersion}`);

for (const { message } of uninstallers) console.log(`Uninstall: ${message}`);

const { values } = parseArgs({ options: { qpdf: { type: "string" }, target: { type: "string" }, packs: { type: "string" }, "public-key": { type: "string" } } });
if (values.qpdf !== undefined || values.target !== undefined) {
  const result = inspectQpdf({ path: values.qpdf, target: values.target, pin });
  if (result.cause) {
    console.error(`qpdf release check failed (${result.cause}): ${result.message}`);
    process.exit(1);
  }
  console.log(`qpdf release check passed: ${result.message}`);
}


// `--packs <dir>`: the release's pack files and catalog, as scripts/pack-release.mjs builds them.
// They must verify against the updater key the app trusts; `--public-key <file>` substitutes
// another key in the updater configuration's format, for checking a locally signed build.
if (values.packs !== undefined) {
  const publicKey = values["public-key"] ? readFileSync(values["public-key"], "utf8").trim() : config.plugins.updater.pubkey;
  const failures = inspectPackRelease({ dir: values.packs, publicKey });
  if (failures.length) {
    for (const { cause, message } of failures) console.error(`Pack release check failed (${cause}): ${message}`);
    process.exit(1);
  }
  const catalog = JSON.parse(readFileSync(`${values.packs}/pack-catalog.json`, "utf8"));
  console.log(`Pack release check passed: ${catalog.packs.length} packs, ${catalog.packs.reduce((total, { payloads }) => total + payloads.length, 0)} payloads signed with the updater key`);
}
