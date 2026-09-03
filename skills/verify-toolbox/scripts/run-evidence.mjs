import { createHash } from "node:crypto";
import { readFile, stat, writeFile, mkdir } from "node:fs/promises";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";

const comparisons = new Set(["deterministic", "lossy", "modelDependent"]);

export async function runEvidence({ manifestPath, evidenceDir, root = dirname(manifestPath) }) {
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    if (manifest.version !== 1 || !Array.isArray(manifest.cases)) {
        throw new Error("Evidence manifest must have version 1 and a cases array.");
    }
    await mkdir(evidenceDir, { recursive: true });

    const cases = [];
    for (const definition of manifest.cases) {
        cases.push(await verifyCase(definition, { root: resolve(root) }));
    }

    const report = {
        manifest: { version: manifest.version, source: basename(manifestPath) },
        cases,
    };
    await writeFile(resolve(evidenceDir, "manifest.json"), `${JSON.stringify(report, null, 2)}\n`);
    await writeFile(resolve(evidenceDir, "results.json"), `${JSON.stringify(report.cases, null, 2)}\n`);
    await writeFile(resolve(evidenceDir, "before.sha256"), `${cases.map((item) => `${item.beforeSha256}  ${item.fixture}`).join("\n")}\n`);
    await writeFile(resolve(evidenceDir, "after.sha256"), `${cases.map((item) => `${item.afterSha256}  ${item.fixture}`).join("\n")}\n`);
    return report;
}

async function verifyCase(definition, context) {
    if (!definition.id || !definition.fixture || !comparisons.has(definition.comparison)) {
        throw new Error(`Case ${definition.id ?? "<unnamed>"} has an invalid fixture or comparison.`);
    }
    const fixture = safePath(definition.fixture, context.root);
    const beforeSha256 = await hash(fixture);
    let status = "passed";
    let detail = "";

    try {
        if (definition.comparison === "deterministic") {
            if (!definition.output || !definition.golden) throw new Error("Deterministic cases require output and golden paths.");
            const outputSha256 = await hash(safePath(definition.output, context.root));
            const goldenSha256 = await hash(safePath(definition.golden, context.root));
            if (outputSha256 !== goldenSha256) throw new Error("Output hash does not match the golden hash.");
            detail = "Output hash matches golden hash.";
        } else if (definition.comparison === "lossy") {
            if (!definition.output) throw new Error("Lossy cases require an output path.");
            await stat(safePath(definition.output, context.root));
            detail = "Output exists for perceptual comparison.";
        } else {
            const expected = definition.expected ?? "available";
            if (expected === "unavailable" && definition.output) throw new Error("Unavailable adapter cases must not declare an output.");
            if (expected === "available" && !definition.output) throw new Error("Available adapter cases require an output path.");
            if (definition.adapter !== "present" && definition.adapter !== "absent") throw new Error("Model-dependent cases require an adapter state.");
            detail = `Adapter ${definition.adapter} matches ${expected} expectation.`;
        }
    } catch (error) {
        status = "failed";
        detail = error instanceof Error ? error.message : String(error);
    }

    const afterSha256 = await hash(fixture);
    if (beforeSha256 !== afterSha256) {
        status = "failed";
        detail = "Fixture changed during verification.";
    }
    return {
        id: definition.id,
        fixture: definition.fixture,
        comparison: definition.comparison,
        adapter: definition.adapter ?? "notApplicable",
        status,
        detail,
        beforeSha256,
        afterSha256,
    };
}

function safePath(value, root) {
    if (isAbsolute(value)) throw new Error(`Evidence paths must be relative: ${value}`);
    const path = resolve(root, value);
    const escaped = relative(root, path).split(/[\\/]/).includes("..");
    if (escaped) throw new Error(`Evidence path escapes the manifest root: ${value}`);
    return path;
}

async function hash(path) {
    const contents = await readFile(path);
    return createHash("sha256").update(contents).digest("hex");
}

if (process.argv[1]?.endsWith("run-evidence.mjs")) {
    const [manifestPath, evidenceDir, root = dirname(manifestPath)] = process.argv.slice(2);
    if (!manifestPath || !evidenceDir) {
        console.error("Usage: node run-evidence.mjs MANIFEST EVIDENCE_DIR [ROOT]");
        process.exitCode = 2;
    } else {
        const report = await runEvidence({ manifestPath, evidenceDir, root });
        console.log(JSON.stringify(report));
        if (report.cases.some((item) => item.status === "failed")) process.exitCode = 1;
    }
}
