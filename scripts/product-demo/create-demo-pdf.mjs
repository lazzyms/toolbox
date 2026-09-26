#!/usr/bin/env node

import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const args = process.argv.slice(2);
const outputPath = args[0];
let title = "Toolbox product demo";
for (let index = 1; index < args.length; index += 1) {
  if (args[index] !== "--title" || !args[index + 1] || args[index + 1].startsWith("--")) {
    throw new Error(`Unexpected or incomplete argument: ${args[index]}`);
  }
  title = args[index + 1];
  index += 1;
}
if (!outputPath) {
  throw new Error("Usage: node create-demo-pdf.mjs <output.pdf> [--title <fictional document title>]");
}

const pdfText = (value) => value.replaceAll("\\", "\\\\").replaceAll("(", "\\(").replaceAll(")", "\\)");

const content = [
  "BT",
  "/F1 24 Tf",
  "72 700 Td",
  `(${pdfText(title)}) Tj`,
  "/F1 15 Tf",
  "0 -38 Td",
  "(Fictional example PDF; no personal data.) Tj",
  "0 -26 Td",
  "(Files are edited locally and saved as a separate copy.) Tj",
  "ET",
].join("\n");

const objects = [
  "<< /Type /Catalog /Pages 2 0 R >>",
  "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
  "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
  "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  `<< /Length ${Buffer.byteLength(content, "ascii")} >>\nstream\n${content}\nendstream`,
];

let pdf = "%PDF-1.4\n";
const offsets = [0];
for (const [index, object] of objects.entries()) {
  offsets.push(Buffer.byteLength(pdf, "ascii"));
  pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
}

const xrefOffset = Buffer.byteLength(pdf, "ascii");
pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
for (const offset of offsets.slice(1)) {
  pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
}
pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;

await writeFile(resolve(outputPath), pdf, { flag: "wx" });
