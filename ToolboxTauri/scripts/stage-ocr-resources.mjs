import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const appRoot = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(appRoot, "..");
const nativeRoot = join(projectRoot, "src-tauri");
const resourceRoot = join(nativeRoot, "resources", "ocr");
const target = process.argv[2];
const executableName = process.platform === "win32" ? "tesseract.exe" : "tesseract";

const commandPath = (command) => {
    const override = process.env.TOOLBOX_TESSERACT_PATH;
    if (override && existsSync(override)) return resolve(override);
    const locator = process.platform === "win32" ? "where.exe" : "which";
    try { return resolve(execFileSync(locator, [command], { encoding: "utf8" }).split(/\r?\n/)[0].trim()); }
    catch { throw new Error(`Could not locate ${command}. Install Tesseract before staging OCR resources.`); }
};

const executable = commandPath("tesseract");
const executableDir = dirname(executable);
const tessdataOverride = process.env.TOOLBOX_TESSDATA_DIR;
const tessdataCandidates = [
    tessdataOverride,
    join(executableDir, "..", "share", "tessdata"),
    join(executableDir, "tessdata"),
    "/opt/homebrew/share/tessdata",
    "/usr/local/share/tessdata",
].filter(Boolean).map((candidate) => resolve(candidate));
const tessdata = tessdataCandidates.find((candidate) => existsSync(join(candidate, "eng.traineddata")));
if (!tessdata) throw new Error("Could not locate Tesseract's eng.traineddata.");

const stagedExecutable = join(resourceRoot, "bin", executableName);
const stagedData = join(resourceRoot, "tessdata", "eng.traineddata");
mkdirSync(dirname(stagedExecutable), { recursive: true });
mkdirSync(dirname(stagedData), { recursive: true });
copyFileSync(executable, stagedExecutable);
copyFileSync(join(tessdata, "eng.traineddata"), stagedData);
if (process.platform !== "win32") chmodSync(stagedExecutable, 0o755);

if (process.platform === "win32") {
    for (const file of readdirSync(executableDir)) {
        if (file.toLowerCase().endsWith(".dll")) copyFileSync(join(executableDir, file), join(resourceRoot, "bin", file));
    }
}

const architecture = (target ?? (execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: ([^\n]+)/m)?.[1] ?? "")).split("-")[0];
if (!architecture) throw new Error("Could not determine the Rust target architecture");
const digest = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const resource = (relativePath, license, maxBytes) => ({ path: relativePath, sha256: digest(join(resourceRoot, relativePath)), license, maxBytes });
const manifest = {
    version: 1,
    architecture,
    resources: {
        tesseract: resource(`bin/${executableName}`, "Apache-2.0", 50 * 1024 * 1024),
        engTraineddata: resource("tessdata/eng.traineddata", "Apache-2.0", 50 * 1024 * 1024),
    },
};
writeFileSync(join(resourceRoot, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Staged Tesseract OCR resources for ${architecture} at ${resourceRoot}`);
