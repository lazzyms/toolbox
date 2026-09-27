import { existsSync, readFileSync } from "node:fs";

const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const canonicalTypes = JSON.parse(readFileSync("shared/shell-file-types.json", "utf8"));
const requiredIcons = config.bundle?.icon ?? [];
const missingIcons = requiredIcons.filter((icon) => !existsSync(`src-tauri/${icon}`));
if (missingIcons.length) throw new Error(`Missing bundle icons: ${missingIcons.join(", ")}`);
if (!config.bundle?.active || config.bundle?.targets !== "all") throw new Error("Tauri bundling must be active for all targets");
if (!config.plugins?.updater?.endpoints?.length) throw new Error("Updater endpoint is not configured");
if (!config.plugins?.updater?.pubkey) throw new Error("Updater public key is not configured");
const canonicalExtensions = Object.values(canonicalTypes).flat().map((extension) => extension.replace(/^\./, "")).sort();
const tauriExtensions = (config.bundle?.fileAssociations ?? []).flatMap(({ ext }) => ext ?? []).map((extension) => extension.replace(/^\./, "")).sort();
if (JSON.stringify(tauriExtensions) !== JSON.stringify(canonicalExtensions)) {
  throw new Error(`Tauri file associations differ from shared/shell-file-types.json. Expected ${canonicalExtensions.join(", ")}; received ${tauriExtensions.join(", ")}.`);
}
const msixManifest = readFileSync("packaging/windows-msix/AppxManifest.xml.template", "utf8");
const msixExtensions = [...msixManifest.matchAll(/<uap:FileType>([^<]+)<\/uap:FileType>/g)].map(([, extension]) => extension.replace(/^\./, "")).sort();
if (JSON.stringify(msixExtensions) !== JSON.stringify(canonicalExtensions)) {
  throw new Error(`MSIX file associations differ from shared/shell-file-types.json. Expected ${canonicalExtensions.join(", ")}; received ${msixExtensions.join(", ")}.`);
}
console.log(`Release configuration passed: ${requiredIcons.length} icons, all Tauri targets, updater and ${canonicalExtensions.length} file associations configured`);
