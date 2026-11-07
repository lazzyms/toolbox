// Installs the native OCR engine and PDF rasterizer pinned in ocr-pin.json for a Windows target
// whose usual installers are not native (Windows arm64), and gathers each helper with the
// DLLs it loads into its own folder:
//   <out>/ocr-engine      tesseract.exe and its DLLs (point TOOLBOX_TESSERACT_PATH here)
//   <out>/pdf-rasterizer  pdftoppm.exe, pdftotext.exe and their DLLs
// Run on the target machine with MSYS2 set up for the pinned msystem:
//   node scripts/stage-windows-ocr-helpers.mjs <rust-target> --msys-root <dir> --out <dir>
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { binaryArchitectures, TARGETS } from "./qpdf-pin.mjs";
import { dllClosure, readOcrPin } from "./ocr-release.mjs";

const { values, positionals } = parseArgs({ allowPositionals: true, options: { "msys-root": { type: "string" }, out: { type: "string" } } });
const target = positionals[0];
const pinned = readOcrPin().windows?.[target];
if (!pinned) throw new Error(`ocr-pin.json pins no Windows OCR helpers for ${target}`);
if (!values["msys-root"] || !values.out) throw new Error("--msys-root and --out are required");
const msysRoot = resolve(values["msys-root"]);
const out = resolve(values.out);
const prefix = join(msysRoot, pinned.msystem.toLowerCase());
const bash = join(msysRoot, "usr", "bin", "bash.exe");
if (!existsSync(bash)) throw new Error(`MSYS2 is not set up at ${msysRoot}`);

const downloads = join(out, "downloads");
mkdirSync(downloads, { recursive: true });
const archives = [];
for (const item of pinned.packages) {
  const archive = join(downloads, basename(new URL(item.url).pathname));
  const response = await fetch(item.url);
  if (!response.ok) throw new Error(`Could not download ${item.url}: HTTP ${response.status}`);
  writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
  const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");
  if (actual !== item.sha256) throw new Error(`${item.name} ${item.version} has SHA-256 ${actual}, but ocr-pin.json pins ${item.sha256}`);
  archives.push(archive);
}

// pacman installs the pinned packages and resolves their libraries from the MSYS2 repository.
const unixPaths = archives.map((archive) => execFileSync(join(msysRoot, "usr", "bin", "cygpath.exe"), ["-u", archive], { encoding: "utf8" }).trim());
execFileSync(bash, ["-lc", `pacman -U --noconfirm ${unixPaths.map((path) => `'${path}'`).join(" ")}`], {
  stdio: "inherit", env: { ...process.env, MSYSTEM: pinned.msystem, CHERE_INVOKING: "1" },
});

const bin = join(prefix, "bin");
const expected = TARGETS[target];
const systemPath = join(process.env.SystemRoot ?? "C:\\Windows", "System32");
for (const item of pinned.packages) {
  const destination = join(out, item.destination);
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });
  const executables = item.executables.map((name) => join(bin, name));
  for (const path of [...executables, ...dllClosure(executables, bin)]) {
    const found = binaryArchitectures(readFileSync(path));
    if (found.format !== expected.format || !found.architectures.includes(expected.architecture)) {
      throw new Error(`${path} is ${found.format} ${found.architectures.join("+")}, not native ${expected.architecture}`);
    }
    copyFileSync(path, join(destination, basename(path)));
  }
  // Each helper must start from its own folder alone, so no DLL is missing from the closure.
  for (const name of item.executables) {
    const flag = name.startsWith("tesseract") ? "--version" : "-v";
    execFileSync(join(destination, name), [flag], { stdio: "pipe", env: { SystemRoot: process.env.SystemRoot, PATH: systemPath } });
  }
  console.log(`Staged ${item.name} ${item.version} with its DLLs at ${destination}`);
}
