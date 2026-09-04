import { readFileSync } from "node:fs";

const registry = readFileSync("src/registry/index.ts", "utf8");
const registryIds = [...registry.matchAll(/(?:id: "([^"]+)"|(?:pdf|image)\("([^"]+)")/g)].map((match) => match[1] ?? match[2]);
const factoryEntries = (registry.match(/(?:pdf|image)\("/g) ?? []).length;
const requiredFields = ["command", "verification", "view"];
const expectedToolCount = 31;

if (registryIds.length !== expectedToolCount) throw new Error(`Tauri registry has ${registryIds.length} tools, expected ${expectedToolCount}`);
if (new Set(registryIds).size !== registryIds.length) throw new Error("Tauri registry contains duplicate IDs");
for (const field of requiredFields) {
    const count = (registry.match(new RegExp(`${field}:`, "g")) ?? []).length + factoryEntries;
    if (count < expectedToolCount) throw new Error(`Registry is missing ${expectedToolCount - count} ${field} entries`);
}

console.log(`Tool matrix passed: ${registryIds.length} tools, ${registryIds.filter((id) => registry.includes(`id: \"${id}\"`)).length} explicit entries`);
