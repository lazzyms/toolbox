import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";

if (!process.argv[2]) throw new Error("OCR resource root is required");
const resourceRoot = resolve(process.argv[2]);
const manifestPath = join(resourceRoot, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const tesseract = manifest.resources?.tesseract;
if (!tesseract?.path) throw new Error("OCR manifest does not declare the Tesseract executable");
tesseract.sha256 = createHash("sha256").update(readFileSync(join(resourceRoot, tesseract.path))).digest("hex");
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
