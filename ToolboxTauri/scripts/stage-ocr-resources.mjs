// Stages the OCR engine and the pinned English data into src-tauri/resources/ocr and writes
// the manifest the app verifies them against. The engine comes from TOOLBOX_TESSERACT_PATH or
// PATH; the English data is the committed tessdata_fast file pinned in ocr-pin.json.
//   node scripts/stage-ocr-resources.mjs [rust-target]
import { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fileSha256, readOcrPin, verifyCommittedEnglishData } from "./ocr-release.mjs";

const appRoot = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(appRoot, "..");
const nativeRoot = join(projectRoot, "src-tauri");
const resourceRoot = join(nativeRoot, "resources", "ocr");
const target = process.argv[2];
const executableName = process.platform === "win32" ? "tesseract.exe" : "tesseract";
const pin = readOcrPin();

const commandPath = (command) => {
    const override = process.env.TOOLBOX_TESSERACT_PATH;
    if (override && existsSync(override)) return resolve(override);
    const locator = process.platform === "win32" ? "where.exe" : "which";
    try { return resolve(execFileSync(locator, [command], { encoding: "utf8" }).split(/\r?\n/)[0].trim()); }
    catch { throw new Error(`Could not locate ${command}. Install Tesseract before staging OCR resources.`); }
};

const executable = commandPath("tesseract");
const executableDir = dirname(executable);
const engineVersion = execFileSync(executable, ["--version"], { encoding: "utf8" }).match(/tesseract v?(\d+\.\d+\.\d+\S*)/i)?.[1];
if (!engineVersion) throw new Error(`Could not read the version of ${executable}`);

// The English data is committed, so every target ships the same pinned bytes.
const englishProblem = verifyCommittedEnglishData(pin);
if (englishProblem) throw new Error(englishProblem);

const stagedExecutable = join(resourceRoot, "bin", executableName);
mkdirSync(dirname(stagedExecutable), { recursive: true });
copyFileSync(executable, stagedExecutable);
if (process.platform !== "win32") chmodSync(stagedExecutable, 0o755);

if (process.platform === "win32") {
    for (const file of readdirSync(executableDir)) {
        if (file.toLowerCase().endsWith(".dll")) copyFileSync(join(executableDir, file), join(resourceRoot, "bin", file));
    }
}

const architecture = (target ?? (execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: ([^\n]+)/m)?.[1] ?? "")).split("-")[0];
if (!architecture) throw new Error("Could not determine the Rust target architecture");
const resource = (relativePath, version, license, maxBytes, sourceUrl) => ({
    path: relativePath, version, sha256: fileSha256(join(resourceRoot, relativePath)), license, ...(sourceUrl ? { sourceUrl } : {}), maxBytes,
});
const english = pin.englishData;
const manifest = {
    version: 1,
    architecture,
    resources: {
        tesseract: resource(`bin/${executableName}`, engineVersion, "Apache-2.0", 50 * 1024 * 1024),
        engTraineddata: resource(english.path, english.version, english.license, english.maxBytes, english.source),
    },
};
writeFileSync(join(resourceRoot, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Staged Tesseract OCR ${engineVersion} and ${english.version} English data for ${architecture} at ${resourceRoot}`);
