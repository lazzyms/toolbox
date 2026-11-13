// Prints, one per line, the DLLs in a directory that are native to the target, and names the skipped ones on stderr.
// bundle-qpdf.ps1 stages only these beside qpdf.exe.
import { parseArgs } from "node:util";
import { nativeLibraries } from "./qpdf-pin.mjs";

const { values, positionals } = parseArgs({ options: { target: { type: "string" } }, allowPositionals: true });
if (positionals.length !== 1 || !values.target) {
  console.error("usage: node scripts/qpdf-libraries.mjs <directory> --target <triple>");
  process.exit(2);
}
const { native, foreign } = nativeLibraries({ directory: positionals[0], target: values.target });
for (const { name, described } of foreign) console.error(`qpdf-libraries: skipping ${name}, which is ${described}, not native to ${values.target}`);
for (const name of native) console.log(name);
