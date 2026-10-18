import { existsSync, readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { inspectQpdf, readPin } from "./qpdf-pin.mjs";

const projectFile = (path) => new URL(`../${path}`, import.meta.url);
const config = JSON.parse(readFileSync(projectFile("src-tauri/tauri.conf.json"), "utf8"));
const requiredIcons = config.bundle?.icon ?? [];
const missingIcons = requiredIcons.filter((icon) => !existsSync(projectFile(`src-tauri/${icon}`)));
if (missingIcons.length) throw new Error(`Missing bundle icons: ${missingIcons.join(", ")}`);
if (!config.bundle?.active || config.bundle?.targets !== "all") throw new Error("Tauri bundling must be active for all targets");
if (!config.plugins?.updater?.endpoints?.length) throw new Error("Updater endpoint is not configured");
if (!config.plugins?.updater?.pubkey) throw new Error("Updater public key is not configured");
const pin = readPin();
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater configured, qpdf minimum ${pin.minimumVersion}`);

const { values } = parseArgs({ options: { qpdf: { type: "string" }, target: { type: "string" } } });
if (values.qpdf !== undefined || values.target !== undefined) {
  const result = inspectQpdf({ path: values.qpdf, target: values.target, pin });
  if (result.cause) {
    console.error(`qpdf release check failed (${result.cause}): ${result.message}`);
    process.exit(1);
  }
  console.log(`qpdf release check passed: ${result.message}`);
}
