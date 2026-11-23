import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const tauriRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sourceRoot = join(tauriRoot, "src-tauri", "src");
const guardFile = join(sourceRoot, "kit", "test_env.rs");

function* rustFiles(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) yield* rustFiles(path);
    else if (entry.name.endsWith(".rs")) yield path;
  }
}

const offences = [];
for (const file of rustFiles(sourceRoot)) {
  if (file === guardFile) continue;
  readFileSync(file, "utf8").split("\n").forEach((line, index) => {
    if (/\b(set_var|remove_var)\b/.test(line)) {
      offences.push(`${relative(tauriRoot, file)}:${index + 1}: ${line.trim()}`);
    }
  });
}

if (offences.length > 0) {
  console.error(offences.join("\n"));
  console.error(
    "\nChange process environment in tests only through EnvOverride (src-tauri/src/kit/test_env.rs), which holds the env lock and restores values on drop.",
  );
  process.exit(1);
}
console.log("env overrides: ok");
