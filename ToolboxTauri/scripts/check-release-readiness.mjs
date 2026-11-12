import { existsSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { inspectFallbackFonts, sourceFontsDir } from "./fallback-fonts.mjs";
import { inspectPackRelease, inspectUninstallers, tauriNsisTemplate } from "./pack-release.mjs";
import { inspectPdftotext, inspectQpdf, inspectRenderer, readPin } from "./qpdf-pin.mjs";
import { describe, readOcrPin, verifyCommittedEnglishData, verifyOcrArtifact } from "./ocr-release.mjs";
import { verifyConvertArtifact } from "./convert-release.mjs";

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
if (!/TOOLBOX_REQUIRE_OCR_RESOURCES: '1'/.test(workflow) || !workflow.includes("--ocr-artifact")) {
  throw new Error("Release builds must require OCR resources and check every artifact with --ocr-artifact");
}
if (!workflow.includes("--convert-artifact") || !workflow.includes("compare-convert-outputs.mjs")) {
  throw new Error("Release builds must check every artifact with --convert-artifact and compare Convert to PDF snapshots across targets");
}
if (!/check-release-readiness\.mjs --packs pack-release/.test(workflow) || !/check-tool-matrix\.mjs --catalog pack-release\/pack-catalog\.json/.test(workflow)) {
  throw new Error("Releases must build their packs, gate them with --packs and check the catalog against the tool matrix");
}
if (!/gh release create[^\n]*pack-release\/\*\.toolboxpack/.test(workflow)) throw new Error("Releases must publish the pack files they build");
if (!/check-release-readiness\.mjs --renderer \S*pdftoppm\.exe --target \$\{\{ matrix\.target \}\}/.test(workflow)) throw new Error("Windows releases must refuse a page renderer that is missing or not native to the target");
const uninstallers = inspectUninstallers({
  config,
  packsSource: readFileSync(projectFile("src-tauri/src/packs.rs"), "utf8"),
  nsisTemplate: tauriNsisTemplate(),
});
const keepsAppData = uninstallers.filter(({ cause }) => cause);
if (keepsAppData.length) throw new Error(`Uninstaller check failed: ${keepsAppData.map(({ cause, message }) => `${cause}: ${message}`).join("; ")}`);
const pin = readPin();
const ocrPin = readOcrPin();
const englishProblem = verifyCommittedEnglishData(ocrPin);
if (englishProblem) throw new Error(englishProblem);
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater and ${canonicalExtensions.length} file associations configured, qpdf minimum ${pin.minimumVersion}, English OCR data ${ocrPin.englishData.version}`);

for (const { message } of uninstallers) console.log(`Uninstall: ${message}`);

const { values } = parseArgs({ options: { qpdf: { type: "string" }, pdftotext: { type: "string" }, renderer: { type: "string" }, target: { type: "string" }, fonts: { type: "string" }, packs: { type: "string" }, "public-key": { type: "string" }, "ocr-artifact": { type: "string" }, "convert-artifact": { type: "string" } } });
const fonts = inspectFallbackFonts({ fontsDir: values.fonts ?? sourceFontsDir });
if (fonts.cause) {
  console.error(`Fallback font release check failed (${fonts.cause}): ${fonts.message}`);
  process.exit(1);
}
console.log(`Fallback font release check passed: ${fonts.message}`);

const ocrArtifact = values["ocr-artifact"];
if (ocrArtifact !== undefined) {
  if (!values.target) {
    console.error("OCR release check failed (usage): --ocr-artifact needs --target");
    process.exit(1);
  }
  const failures = verifyOcrArtifact({ root: ocrArtifact, target: values.target, pin: ocrPin });
  if (failures.length) {
    console.error(`OCR release check failed (${[...new Set(failures.map(({ cause }) => cause))].join(", ")}) for ${values.target} at ${ocrArtifact}:\n${describe(failures)}`);
    process.exit(1);
  }
  console.log(`OCR release check passed: ${ocrArtifact} carries a native ${values.target} OCR engine and PDF rasterizer and the pinned English data`);
}
const convertArtifact = values["convert-artifact"];
if (convertArtifact !== undefined) {
  if (!values.target) {
    console.error("Convert to PDF release check failed (usage): --convert-artifact needs --target");
    process.exit(1);
  }
  const failures = verifyConvertArtifact({ root: convertArtifact, target: values.target });
  if (failures.length) {
    console.error(`Convert to PDF release check failed (${[...new Set(failures.map(({ cause }) => cause))].join(", ")}) for ${values.target} at ${convertArtifact}:\n${describe(failures)}`);
    process.exit(1);
  }
  console.log(`Convert to PDF release check passed: ${convertArtifact} carries the native ${values.target} renderer and the fallback fonts, and conversion uses no Bundled helper`);
}

const helperChecks = [];
if (values.renderer !== undefined) {
  helperChecks.push(["page renderer", () => inspectRenderer({ path: values.renderer, target: values.target })]);
}
if (values.qpdf !== undefined || (values.target !== undefined && values.pdftotext === undefined && values.renderer === undefined && ocrArtifact === undefined && convertArtifact === undefined)) {
  helperChecks.push(["qpdf", () => inspectQpdf({ path: values.qpdf, target: values.target, pin })]);
}
if (values.pdftotext !== undefined) {
  helperChecks.push(["pdftotext", () => inspectPdftotext({ path: values.pdftotext, target: values.target })]);
}
let helperFailed = false;
for (const [name, inspect] of helperChecks) {
  const result = inspect();
  if (result.cause) {
    console.error(`${name} release check failed (${result.cause}): ${result.message}`);
    helperFailed = true;
  } else {
    console.log(`${name} release check passed: ${result.message}`);
  }
}
if (helperFailed) process.exit(1);

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
