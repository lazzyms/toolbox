import { readFileSync } from "node:fs";

const registry = readFileSync("src/registry/index.ts", "utf8");
const registryIds = [...registry.matchAll(/(?:id: "([^"]+)"|(?:pdf|image)\("([^"]+)")/g)].map((match) => match[1] ?? match[2]);
const factoryEntries = (registry.match(/(?:pdf|image)\("/g) ?? []).length;
const requiredFields = ["command", "verification", "view", "acceptedExtensions", "inputCardinality", "supportsPageSelection", "supportsPreview", "nativeAvailability"];
const expectedToolCount = 32;

if (registryIds.length !== expectedToolCount) throw new Error(`Tauri registry has ${registryIds.length} tools, expected ${expectedToolCount}`);
if (new Set(registryIds).size !== registryIds.length) throw new Error("Tauri registry contains duplicate IDs");
if ((registry.match(/\.\.\.capability\(/g) ?? []).length !== expectedToolCount) throw new Error("Every tool must declare one capability contract");
for (const field of requiredFields) {
    const count = (registry.match(new RegExp(`${field}:`, "g")) ?? []).length + factoryEntries;
    if (["acceptedExtensions", "inputCardinality", "supportsPageSelection", "supportsPreview", "nativeAvailability"].includes(field)) {
        if (!registry.includes(`${field}:`)) throw new Error(`Capability helper is missing ${field}`);
    } else if (count < expectedToolCount) {
        throw new Error(`Registry is missing ${expectedToolCount - count} ${field} entries`);
    }
}

console.log(`Tool matrix passed: ${registryIds.length} tools, ${registryIds.filter((id) => registry.includes(`id: \"${id}\"`)).length} explicit entries`);
