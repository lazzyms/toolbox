import { existsSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { inspectQpdf, readPin } from "./qpdf-pin.mjs";
import { describe, readOcrPin, verifyCommittedEnglishData, verifyOcrArtifact } from "./ocr-release.mjs";

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
const pin = readPin();
const ocrPin = readOcrPin();
const englishProblem = verifyCommittedEnglishData(ocrPin);
if (englishProblem) throw new Error(englishProblem);
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater and ${canonicalExtensions.length} file associations configured, qpdf minimum ${pin.minimumVersion}, English OCR data ${ocrPin.englishData.version}`);

const { values } = parseArgs({ options: { qpdf: { type: "string" }, target: { type: "string" }, "ocr-artifact": { type: "string" } } });
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
if (values.qpdf !== undefined || (values.target !== undefined && ocrArtifact === undefined)) {
  const result = inspectQpdf({ path: values.qpdf, target: values.target, pin });
  if (result.cause) {
    console.error(`qpdf release check failed (${result.cause}): ${result.message}`);
    process.exit(1);
  }
  console.log(`qpdf release check passed: ${result.message}`);
}

