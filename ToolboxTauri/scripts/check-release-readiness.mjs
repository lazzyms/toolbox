import { existsSync, readFileSync } from "node:fs";

const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const requiredIcons = config.bundle?.icon ?? [];
const missingIcons = requiredIcons.filter((icon) => !existsSync(`src-tauri/${icon}`));
if (missingIcons.length) throw new Error(`Missing bundle icons: ${missingIcons.join(", ")}`);
if (!config.bundle?.active || config.bundle?.targets !== "all") throw new Error("Tauri bundling must be active for all targets");
if (!config.plugins?.updater?.endpoints?.length) throw new Error("Updater endpoint is not configured");
if (!config.plugins?.updater?.pubkey) throw new Error("Updater public key is not configured");
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater configured`);
