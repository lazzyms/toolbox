import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const sourceFontsDir = fileURLToPath(new URL("../src-tauri/resources/fonts", import.meta.url));

export function readFontManifest(fontsDir = sourceFontsDir) {
  return JSON.parse(readFileSync(join(fontsDir, "manifest.json"), "utf8"));
}

/**
 * Checks the bundled fallback fonts in `fontsDir` against the font manifest: every face listed under
 * `fallbackFonts` (and the default) must exist and match its recorded SHA-256, and the fallback faces
 * together must stay within `fallbackFonts.budgetBytes`. Returns `{ cause, message }`; `cause` is null on success.
 */
export function inspectFallbackFonts({ fontsDir = sourceFontsDir, manifest = readFontManifest(fontsDir) } = {}) {
  const policy = manifest.fallbackFonts;
  if (!policy || !Number.isInteger(policy.budgetBytes) || !Array.isArray(policy.families) || !policy.default) {
    return { cause: "invalid", message: "The font manifest has no fallbackFonts policy with a budget, families and a default." };
  }
  const faceKeys = (family) => Object.values(family.faces ?? {});
  const fallbackKeys = [...new Set(policy.families.flatMap(faceKeys))];
  const allKeys = [...new Set([...fallbackKeys, ...faceKeys(policy.default)])];
  for (const key of allKeys) {
    const spec = manifest.resources?.[key];
    if (!spec) return { cause: "invalid", message: `${key} is listed as a fallback face but has no manifest entry.` };
    const path = join(fontsDir, spec.path);
    if (!existsSync(path)) return { cause: "missing", message: `${spec.path} (${key}) is missing from ${fontsDir}.` };
    const bytes = readFileSync(path);
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (digest !== spec.sha256) return { cause: "changed", message: `${spec.path} (${key}) has SHA-256 ${digest}; the manifest records ${spec.sha256}.` };
    if (bytes.length > spec.maxBytes) return { cause: "changed", message: `${spec.path} (${key}) is ${bytes.length} bytes, over its ${spec.maxBytes}-byte limit.` };
  }
  const totalBytes = fallbackKeys.reduce((sum, key) => sum + statSync(join(fontsDir, manifest.resources[key].path)).size, 0);
  if (totalBytes > policy.budgetBytes) {
    return { cause: "over-budget", message: `The fallback fonts take ${totalBytes} bytes, over the ${policy.budgetBytes}-byte budget.` };
  }
  return {
    cause: null,
    message: `${fallbackKeys.length} fallback faces in ${policy.families.length} families verified, ${totalBytes} of ${policy.budgetBytes} budget bytes`,
  };
}
