// Compares the Convert to PDF snapshots each Release target made from the embedded-font
// fixtures with its own bundled fonts: every family's extracted text and page geometry must
// match the first target's unless convert-platform-differences.json documents the difference
// for that family, aspect and target. Installed system fonts are the stated platform
// difference, so the fixtures never use them and nothing about them is compared.
//   node scripts/compare-convert-outputs.mjs <target-name>=<outputs-folder> ...
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// One fixture per extension the release snapshot test converts (family_documents in convert.rs).
export const FAMILIES = ["txt", "md", "rtf", "csv", "tsv", "docx", "odt", "xlsx", "ods", "pptx", "odp", "html", "epub", "doc", "xls", "ppt"];
export const ASPECTS = ["text", "geometry"];
export const DIFFERENCES_PATH = new URL("../convert-platform-differences.json", import.meta.url);
export const CAPABILITIES_PATH = new URL("../shared/tool-capabilities.json", import.meta.url);

/** Convert to PDF families that are available but have no snapshot fixture. PDF input is copied, not rendered. */
export function uncoveredFamilies(capabilities = JSON.parse(readFileSync(CAPABILITIES_PATH, "utf8"))) {
  const convert = capabilities.find(({ command }) => command === "convert_to_pdf");
  return convert.formatFamilies
    .filter(({ id, nativeAvailability }) => nativeAvailability === "available" && id !== "pdf")
    .filter(({ extensions }) => !extensions.some((extension) => FAMILIES.includes(extension.replace(/^\./, ""))))
    .map(({ id }) => id);
}

/** Reads a target's snapshots.json and checks that it covers every family. */
export function readTargetOutputs(name, folder) {
  const path = join(folder, "snapshots.json");
  if (!existsSync(path)) throw new Error(`${name} saved no Convert to PDF snapshots at ${path}`);
  const { families = {} } = JSON.parse(readFileSync(path, "utf8"));
  for (const family of FAMILIES) {
    if (!Array.isArray(families[family]?.pages) || !families[family].pages.length) throw new Error(`${name} has no pages for ${family}`);
  }
  return { name, folder, families };
}

export function readDifferences(path = DIFFERENCES_PATH) {
  const { differences } = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(differences)) throw new Error("convert-platform-differences.json must list differences");
  for (const entry of differences) {
    if (!FAMILIES.includes(entry.family) || !ASPECTS.includes(entry.aspect) || !entry.target || !String(entry.reason ?? "").trim()) {
      throw new Error(`Each platform difference needs a family (${FAMILIES.join(", ")}), an aspect (${ASPECTS.join(" or ")}), a target and a reason: ${JSON.stringify(entry)}`);
    }
  }
  return differences;
}

const text = (pages) => JSON.stringify(pages.map((page) => page.text));
const geometry = (pages) => JSON.stringify(pages.map(({ width, height }) => [width, height]));

/**
 * Returns the differences between targets: each with the family, the aspect, the target, whether
 * it is documented, and the reason when it is. The first target is the reference.
 */
export function compareOutputs(targets, differences) {
  if (targets.length < 2) throw new Error("Compare at least two targets");
  const [reference, ...others] = targets;
  return others.flatMap((target) => FAMILIES.flatMap((family) => [["text", text], ["geometry", geometry]]
    .filter(([, read]) => read(target.families[family].pages) !== read(reference.families[family].pages))
    .map(([aspect]) => {
      const documented = differences.find((entry) => entry.family === family && entry.aspect === aspect && entry.target === target.name && (!entry.reference || entry.reference === reference.name));
      return { family, aspect, target: target.name, reference: reference.name, documented: Boolean(documented), reason: documented?.reason };
    })));
}

const main = () => {
  const uncovered = uncoveredFamilies();
  if (uncovered.length) throw new Error(`Convert to PDF families without a release snapshot fixture: ${uncovered.join(", ")}`);
  const targets = process.argv.slice(2).map((argument) => {
    const at = argument.indexOf("=");
    if (at < 1) throw new Error(`Expected <target-name>=<outputs-folder>, got ${argument}`);
    return readTargetOutputs(argument.slice(0, at), argument.slice(at + 1));
  });
  const found = compareOutputs(targets, readDifferences());
  for (const difference of found) {
    const line = `${difference.family} ${difference.aspect} on ${difference.target} differs from ${difference.reference}`;
    if (difference.documented) console.log(`Documented platform difference: ${line}. ${difference.reason}`);
    else console.error(`Undocumented Convert to PDF difference: ${line}. Compare the snapshots, then document it in convert-platform-differences.json or fix the release.`);
  }
  if (found.some((difference) => !difference.documented)) process.exit(1);
  console.log(`Convert to PDF snapshots agree across ${targets.map(({ name }) => name).join(", ")}${found.length ? " apart from the documented platform differences" : ""}.`);
};

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
