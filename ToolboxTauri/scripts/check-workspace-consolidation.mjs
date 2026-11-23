import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { duplicates, loadRegistry } from "./registry-data.mjs";

const root = process.cwd();
const read = (relativePath) => fs.readFileSync(path.join(root, relativePath), "utf8");
const { tools, workspaces } = await loadRegistry();
const atomicIds = tools.map(({ id }) => id);
const workspaceIds = workspaces.flatMap(({ toolIds }) => toolIds);

const failures = [];
const list = (ids) => ids.join(", ");
const unplaced = atomicIds.filter((id) => !workspaceIds.includes(id));
const unknown = workspaceIds.filter((id) => !atomicIds.includes(id));
const repeated = duplicates(workspaceIds);
const repeatedTools = duplicates(atomicIds);
if (unplaced.length) failures.push(`${list(unplaced)} sit in no workspace; add each to one toolIds list in src/registry/index.ts`);
if (unknown.length) failures.push(`workspaces list ${list(unknown)}, which the registry does not register; fix the toolIds in src/registry/index.ts`);
if (repeated.length) failures.push(`${list(repeated)} sit in more than one workspace; keep each in exactly one`);
if (repeatedTools.length) failures.push(`${list(repeatedTools)} are registered more than once`);

const expectedWorkspaces = ["file-security", "pdf-editor", "pdf-convert", "image-editor", "compress-files", "media-tools"];
const declaredWorkspaces = workspaces.map(({ id }) => id);
if (declaredWorkspaces.join() !== expectedWorkspaces.join()) failures.push(`expected workspaces ${expectedWorkspaces.join(", ")}, found ${declaredWorkspaces.join(", ")}`);
const membersOf = (id) => workspaces.find((workspace) => workspace.id === id)?.toolIds ?? [];
if (membersOf("file-security").join() !== "lock-unlock") failures.push("file-security must hold exactly lock-unlock");
for (const retired of ["pdf-unlock", "pdf-protect", "office-protect", "pdf-compress", "compress"]) {
  if (atomicIds.includes(retired) || workspaceIds.includes(retired)) failures.push(`${retired} is retired and must not be registered`);
}
if (membersOf("compress-files").join() !== "compress-files") failures.push("compress-files must hold exactly compress-files");
if (!membersOf("pdf-convert").includes("pdf-ocr")) failures.push("pdf-ocr must belong to the PDF conversion workspace");
if (!membersOf("pdf-convert").includes("convert-to-pdf")) failures.push("convert-to-pdf must belong to the PDF conversion workspace");
if (membersOf("media-tools").includes("pdf-ocr")) failures.push("pdf-ocr must not belong to the media workspace");
for (const id of ["pdf-merge", "pdf-split", "pdf-crop", "pdf-page-numbers", "pdf-rotate", "pdf-repair", "pdf-compare", "pdf-redact", "pdf-privacy", "pdf-properties", "pdf-bates"]) {
  if (!membersOf("pdf-editor").includes(id)) failures.push(`${id} must belong to the PDF Editor`);
}
for (const id of ["pdf-merge", "pdf-split"]) {
  if (membersOf("pdf-convert").includes(id)) failures.push(`${id} belongs to the PDF Editor, not PDF Conversion`);
}
if (!membersOf("image-editor").includes("image-metadata")) failures.push("image-metadata must belong to the Image Editor");

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
  console.log(`Workspace consolidation passed: ${atomicIds.length} atomic IDs, ${expectedWorkspaces.length} workspaces, no legacy workspace views`);
}
