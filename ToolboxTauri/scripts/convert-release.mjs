// Release checks for Convert to PDF. Conversion is the app's own renderer, so a Release
// artifact must carry the app binary and the bundled fallback fonts. It must not need a
// Bundled helper or an optional pack: the conversion sources never resolve or start a
// helper, and the release snapshot test converts every family with no packs.
import { existsSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { inspectFallbackFonts, readFontManifest } from "./fallback-fonts.mjs";
import { nativeProblem } from "./ocr-release.mjs";
import { TARGETS } from "./qpdf-pin.mjs";

export const RUST_SOURCES = fileURLToPath(new URL("../src-tauri/src/", import.meta.url));
const CONVERT_ENTRY = "kit/pdf/convert.rs";
// A source that resolves or starts a helper process: a Bundled helper, or one found on PATH.
const HELPER_USE = /\bresolve_helper\b|\bCommand::new\b|\bprocess::Command\b|\bresources::(?:PDF_RENDERER|PDF_TEXT|QPDF)\b|\bresources::resolve\b/;

/** The part of a Rust source that ships: everything before its first test module. */
const production = (text) => text.split(/^\s*#\[cfg\(test\)\]/m)[0];

/** The modules a `use crate::...;` line names, as path segments under src-tauri/src. */
function importedPaths(text) {
  const paths = [];
  for (const [, body] of production(text).matchAll(/^use crate::([\s\S]*?);/gm)) {
    const flat = body.replace(/\s+/g, "");
    const brace = flat.indexOf("{");
    if (brace < 0) { paths.push(flat.split("::")); continue; }
    const prefix = flat.slice(0, brace).split("::").filter(Boolean);
    for (const item of flat.slice(brace + 1, -1).split(",")) paths.push([...prefix, ...item.split("::").filter((part) => part && part !== "self")]);
  }
  return paths;
}

function moduleFile(root, segments) {
  for (let length = segments.length; length > 0; length -= 1) {
    const base = posix.join(...segments.slice(0, length));
    if (existsSync(join(root, `${base}.rs`))) return `${base}.rs`;
    if (existsSync(join(root, base, "mod.rs"))) return posix.join(base, "mod.rs");
  }
  return null;
}

/**
 * The conversion's own sources: convert.rs and every PDF module it reaches through its imports.
 * kit/pdf/mod.rs and modules outside kit/pdf are shared with other tools and are left out.
 */
export function conversionSources(root = RUST_SOURCES) {
  const found = new Set();
  const queue = [CONVERT_ENTRY];
  while (queue.length) {
    const file = queue.shift();
    if (found.has(file)) continue;
    found.add(file);
    for (const segments of importedPaths(readFileSync(join(root, file), "utf8"))) {
      const next = moduleFile(root, segments);
      if (next && next.startsWith("kit/pdf/") && !next.endsWith("mod.rs")) queue.push(next);
    }
  }
  return [...found].sort();
}

/** Conversion sources that resolve or start a helper. Convert to PDF must not add a Bundled helper. */
export function conversionHelperUses(root = RUST_SOURCES) {
  return conversionSources(root)
    .filter((file) => HELPER_USE.test(production(readFileSync(join(root, file), "utf8"))))
    .map((file) => ({ cause: "bundled-helper", message: `${file} resolves or starts a helper process; Convert to PDF renders in the app and must not need a Bundled helper` }));
}

/** Where a Release artifact keeps the app binary that renders conversions and the fallback fonts. */
export function convertArtifactLayout(root, target) {
  const resources = existsSync(join(root, "Contents", "Resources")) ? join(root, "Contents", "Resources") : root;
  const renderer = TARGETS[target]?.format === "pe" ? join(resources, "toolbox.exe") : join(resources, "..", "MacOS", "toolbox");
  const fonts = [join(resources, "resources", "fonts"), join(resources, "fonts")].find((path) => existsSync(join(path, "manifest.json")))
    ?? join(resources, "resources", "fonts");
  return { resources, renderer, fonts };
}

/**
 * Checks one Release artifact for what Convert to PDF needs from its base installer. Returns every
 * failure, each with a cause: missing, wrong-architecture, fonts-missing, fonts-invalid, fonts-changed,
 * fonts-over-budget or bundled-helper. An empty list means the artifact passes.
 */
export function verifyConvertArtifact({ root, target, sources = RUST_SOURCES }) {
  if (!TARGETS[target]) throw new Error(`Unsupported release target ${target}; expected one of ${Object.keys(TARGETS).join(", ")}`);
  if (!root || !existsSync(root)) return [{ cause: "missing", message: `The Release artifact is missing at ${root}` }];
  const failures = [];
  const layout = convertArtifactLayout(root, target);
  if (!existsSync(layout.renderer)) failures.push({ cause: "missing", message: `The app binary that renders Convert to PDF is missing at ${layout.renderer}` });
  else {
    const problem = nativeProblem(layout.renderer, target, "The app binary that renders Convert to PDF");
    if (problem) failures.push(problem);
  }
  if (!existsSync(join(layout.fonts, "manifest.json"))) failures.push({ cause: "fonts-missing", message: `The fallback font manifest is missing from ${layout.fonts}` });
  else {
    let fonts;
    try { fonts = inspectFallbackFonts({ fontsDir: layout.fonts, manifest: readFontManifest(layout.fonts) }); }
    catch (error) { fonts = { cause: "invalid", message: `The fallback font manifest in ${layout.fonts} cannot be read: ${error.message}` }; }
    if (fonts.cause) failures.push({ cause: `fonts-${fonts.cause}`, message: fonts.message });
  }
  failures.push(...conversionHelperUses(sources));
  return failures;
}
