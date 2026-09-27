#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { mkdir, readdir, rm, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PASSWORD = "batch-demo-2026";
const CREATE_PDF = new URL("./create-demo-pdf.mjs", import.meta.url);
const DOCUMENTS = [
  ["project-brief.pdf", "Fictional project brief"],
  ["meeting-notes.pdf", "Fictional meeting notes"],
  ["budget-summary.pdf", "Fictional budget summary"],
];

function parseArgs(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (!["--directory", "--qpdf"].includes(name)) {
      throw new Error(`Unexpected argument: ${name}`);
    }
    const value = args[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`Missing value for ${name}`);
    options[name.slice(2)] = value;
    index += 1;
  }
  if (!options.directory || !options.qpdf) {
    throw new Error("Usage: create-protected-demo-pdfs.mjs --directory <folder> --qpdf <qpdf executable>");
  }
  return { directory: resolve(options.directory), qpdf: resolve(options.qpdf) };
}

const { directory, qpdf } = parseArgs(process.argv.slice(2));
await mkdir(directory, { recursive: true });
const existing = await readdir(directory);
if (existing.length > 0) throw new Error(`Refusing to generate fixtures in a non-empty directory: ${directory}`);
if (!(await stat(qpdf)).isFile()) throw new Error(`Could not locate qpdf: ${qpdf}`);

const cleartextDir = resolve(directory, ".cleartext-source");
await mkdir(cleartextDir);
try {
  for (const [filename, title] of DOCUMENTS) {
    const source = resolve(cleartextDir, filename);
    const destination = resolve(directory, filename);
    execFileSync(process.execPath, [fileURLToPath(CREATE_PDF), source, "--title", title], { stdio: "inherit" });
    execFileSync(qpdf, ["--encrypt", PASSWORD, PASSWORD, "256", "--", source, destination], { stdio: "inherit" });

    const details = execFileSync(qpdf, ["--show-encryption", `--password=${PASSWORD}`, destination], { encoding: "utf8" });
    if (!details.includes("R = 6")) throw new Error(`${filename} was not protected with AES-256 encryption`);
  }
} finally {
  await rm(cleartextDir, { recursive: true, force: true });
}

console.log(`Created ${DOCUMENTS.length} fictional password-protected PDFs in ${directory}`);
