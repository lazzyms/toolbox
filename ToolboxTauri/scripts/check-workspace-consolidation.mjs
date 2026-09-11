import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const read = (relativePath) => fs.readFileSync(path.join(root, relativePath), "utf8");
const registry = read("src/registry/index.ts");
const atomicSource = registry.split("export const UtilityRegistry")[0];
const atomicIds = [...atomicSource.matchAll(/\{ id: "([^"]+)"/g)].map((match) => match[1]);
const workspaceSource = registry.split("export const ToolWorkspaceRegistry")[1] ?? "";
const workspaceIds = [...workspaceSource.matchAll(/toolIds:\s*\[([\s\S]*?)\]/g)]
  .flatMap((match) => [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]));

const failures = [];
const unique = (values) => new Set(values).size === values.length;
if (atomicIds.length !== 32 || !unique(atomicIds)) failures.push(`expected 32 unique atomic IDs, found ${atomicIds.length}`);
if (workspaceIds.length !== atomicIds.length || !unique(workspaceIds)) failures.push("workspace membership must contain each atomic ID exactly once");
if (atomicIds.some((id) => !workspaceIds.includes(id)) || workspaceIds.some((id) => !atomicIds.includes(id))) failures.push("workspace membership and atomic registry differ");

const conversion = workspaceSource.match(/id: "pdf-convert"[\s\S]*?toolIds:\s*\[([\s\S]*?)\]/)?.[1] ?? "";
const media = workspaceSource.match(/id: "media-tools"[\s\S]*?toolIds:\s*\[([\s\S]*?)\]/)?.[1] ?? "";
if (!conversion.includes('"pdf-ocr"')) failures.push("pdf-ocr must belong to the PDF conversion workspace");
if (media.includes('"pdf-ocr"')) failures.push("pdf-ocr must not belong to the media workspace");

const mainPage = read("src/views/MainPage.tsx");
const legacyViews = [
  "PDFConversionView",
  "PDFEditView",
  "PDFPageToolView",
  "PDFPathsView",
  "PDFProtectView",
  "PDFSelectionView",
  "PDFSimpleToolView",
  "PDFUnlockView",
  "ImageCompressView",
  "ImageConvertView",
  "ImageEffectView",
  "ImageFormatView",
  "ImageGeometryView",
  "ImageMetadataView",
  "VisionView",
];
for (const view of legacyViews) {
  if (new RegExp(`from ["']\\./${view}["']`).test(mainPage)) failures.push(`${view} is still imported by MainPage`);
  if (fs.existsSync(path.join(root, "src/views", `${view}.tsx`))) failures.push(`${view}.tsx should be removed after workspace migration`);
}

if (failures.length) {
  console.error(failures.map((failure) => `Workspace consolidation failed: ${failure}`).join("\n"));
  process.exitCode = 1;
} else {
  console.log("Workspace consolidation passed: 32 atomic IDs, 5 workspaces, no legacy workspace views");
}
