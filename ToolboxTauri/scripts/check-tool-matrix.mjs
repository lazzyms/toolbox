import { readFileSync } from "node:fs";

const swift = readFileSync("../Sources/Toolbox/Registry/Utility+PDF.swift", "utf8") +
    readFileSync("../Sources/Toolbox/Registry/Utility+Images.swift", "utf8");
const registry = readFileSync("src/registry/index.ts", "utf8");
const swiftIds = [...swift.matchAll(/Utility\(id: "([^"]+)"/g)].map((match) => match[1]);
const registryIds = [...registry.matchAll(/(?:id: "([^"]+)"|(?:pdf|image)\("([^"]+)")/g)].map((match) => match[1] ?? match[2]);
const factoryEntries = (registry.match(/(?:pdf|image)\("/g) ?? []).length;
const requiredFields = ["command", "verification", "view"];

if (swiftIds.length !== 31) throw new Error(`Swift registry has ${swiftIds.length} tools, expected 31`);
if (registryIds.length !== swiftIds.length) throw new Error(`Tauri registry has ${registryIds.length} tools, expected ${swiftIds.length}`);
if (new Set(registryIds).size !== registryIds.length) throw new Error("Tauri registry contains duplicate IDs");
const missing = swiftIds.filter((id, index) => registryIds[index] !== id);
if (missing.length > 0) throw new Error(`Registry order or IDs differ: ${missing.join(", ")}`);
for (const field of requiredFields) {
    const count = (registry.match(new RegExp(`${field}:`, "g")) ?? []).length + factoryEntries;
    if (count < swiftIds.length) throw new Error(`Registry is missing ${swiftIds.length - count} ${field} entries`);
}

console.log(`Tool matrix passed: ${swiftIds.length} tools, ${registryIds.filter((id) => registry.includes(`id: \"${id}\"`)).length} explicit entries`);
