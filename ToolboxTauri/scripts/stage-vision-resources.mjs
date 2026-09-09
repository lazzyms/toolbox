import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const appRoot = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(appRoot, "..");
const nativeRoot = join(projectRoot, "src-tauri");
const resourceRoot = join(nativeRoot, "resources", "vision");
const target = process.argv[2];
const executableName = process.platform === "win32" ? "toolbox-vision-adapter.exe" : "toolbox-vision-adapter";
const targetRoot = target ? join(nativeRoot, "target", target) : join(nativeRoot, "target");
const builtAdapter = join(targetRoot, "release", executableName);
const stagedAdapter = join(resourceRoot, "bin", executableName);

const modelFiles = {
    faceBlurModel: ["models/face_detection_yunet_2023mar.onnx", "MIT", 1024 * 1024],
    backgroundRemovalModel: ["models/u2netp.onnx", "Apache-2.0", 10 * 1024 * 1024],
};

const architecture = (target ?? (execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: ([^\n]+)/m)?.[1] ?? "")).split("-")[0];
if (!architecture) throw new Error("Could not determine the Rust target architecture");

const cargoArgs = ["build", "--release", "--features", "vision-adapter", "--bin", "toolbox-vision-adapter"];
if (target) cargoArgs.push("--target", target);
execFileSync("cargo", cargoArgs, { cwd: nativeRoot, stdio: "inherit" });

for (const path of Object.values(modelFiles)) {
    const fullPath = join(resourceRoot, path[0]);
    if (!existsSync(fullPath)) throw new Error(`Missing vision model ${fullPath}`);
}
if (!existsSync(builtAdapter)) throw new Error(`Missing built vision adapter ${builtAdapter}`);

mkdirSync(join(resourceRoot, "bin"), { recursive: true });
copyFileSync(builtAdapter, stagedAdapter);
if (process.platform !== "win32") chmodSync(stagedAdapter, 0o755);

const digest = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const resource = (relativePath, license, maxBytes) => ({ path: relativePath, sha256: digest(join(resourceRoot, relativePath)), license, maxBytes });
const adapterSpec = resource(`bin/${executableName}`, "MIT AND Apache-2.0", 50 * 1024 * 1024);
const manifest = {
    version: 1,
    architecture,
    resources: {
        faceBlur: adapterSpec,
        faceBlurModel: resource(...modelFiles.faceBlurModel),
        backgroundRemoval: adapterSpec,
        backgroundRemovalModel: resource(...modelFiles.backgroundRemovalModel),
    },
};
writeFileSync(join(resourceRoot, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Staged vision adapter for ${architecture} at ${resourceRoot}`);
