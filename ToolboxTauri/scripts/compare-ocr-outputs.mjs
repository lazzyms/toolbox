// Compares the English fixture outputs each Release target recognized with its own OCR
// resources. Every output must match the first target's byte for byte unless
// ocr-platform-differences.json documents the difference for that output and target.
//   node scripts/compare-ocr-outputs.mjs <target-name>=<outputs-folder> ...
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const OUTPUTS = ["text.txt", "searchable.pdf"];
export const DIFFERENCES_PATH = new URL("../ocr-platform-differences.json", import.meta.url);

/** Reads each target's fingerprints.json and checks it against the saved outputs. */
export function readTargetOutputs(name, folder) {
  const summaryPath = join(folder, "fingerprints.json");
  if (!existsSync(summaryPath)) throw new Error(`${name} saved no OCR fingerprints at ${summaryPath}`);
  const { files = {} } = JSON.parse(readFileSync(summaryPath, "utf8"));
  for (const output of OUTPUTS) {
    if (!files[output]) throw new Error(`${name} has no fingerprint for ${output}`);
    if (!existsSync(join(folder, output))) throw new Error(`${name} did not save ${output}`);
    const actual = createHash("sha256").update(readFileSync(join(folder, output))).digest("hex");
    if (actual !== files[output]) throw new Error(`${name}'s saved ${output} has SHA-256 ${actual}, not the recorded ${files[output]}`);
  }
  return { name, folder, files };
}

export function readDifferences(path = DIFFERENCES_PATH) {
  const { differences } = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(differences)) throw new Error("ocr-platform-differences.json must list differences");
  for (const entry of differences) {
    if (!OUTPUTS.includes(entry.output) || !entry.target || !String(entry.reason ?? "").trim()) {
      throw new Error(`Each platform difference needs an output (${OUTPUTS.join(" or ")}), a target and a reason: ${JSON.stringify(entry)}`);
    }
  }
  return differences;
}

/**
 * Returns the differences between targets: each with the output, the target, whether it is
 * documented, and the reason when it is. The first target is the reference.
 */
export function compareOutputs(targets, differences) {
  if (targets.length < 2) throw new Error("Compare at least two targets");
  const [reference, ...others] = targets;
  return others.flatMap((target) => OUTPUTS
    .filter((output) => target.files[output] !== reference.files[output])
    .map((output) => {
      const documented = differences.find((entry) => entry.output === output && entry.target === target.name && (!entry.reference || entry.reference === reference.name));
      return { output, target: target.name, reference: reference.name, documented: Boolean(documented), reason: documented?.reason };
    }));
}

const main = () => {
  const targets = process.argv.slice(2).map((argument) => {
    const at = argument.indexOf("=");
    if (at < 1) throw new Error(`Expected <target-name>=<outputs-folder>, got ${argument}`);
    return readTargetOutputs(argument.slice(0, at), argument.slice(at + 1));
  });
  const found = compareOutputs(targets, readDifferences());
  for (const difference of found) {
    const line = `${difference.output} on ${difference.target} differs from ${difference.reference}`;
    if (difference.documented) console.log(`Documented platform difference: ${line}. ${difference.reason}`);
    else console.error(`Undocumented OCR difference: ${line}. Compare the saved outputs, then document it in ocr-platform-differences.json or fix the release.`);
  }
  const undocumented = found.filter((difference) => !difference.documented);
  if (undocumented.length) process.exit(1);
  console.log(`OCR outputs agree across ${targets.map(({ name }) => name).join(", ")}${found.length ? " apart from the documented platform differences" : ""}.`);
};

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
