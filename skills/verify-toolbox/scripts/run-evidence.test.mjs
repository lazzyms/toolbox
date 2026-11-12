import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runEvidence } from "./run-evidence.mjs";

test("records deterministic and absent-adapter cases without mutating fixtures", async () => {
    const root = await mkdtemp(join(tmpdir(), "toolbox-evidence-"));
    const fixture = join(root, "input.bin");
    const golden = join(root, "golden.bin");
    const evidence = join(root, "evidence");
    await writeFile(fixture, "fixture");
    await writeFile(golden, "golden");
    await mkdir(join(root, "outputs"));
    await writeFile(join(root, "outputs", "deterministic.bin"), "golden");

    const manifest = join(root, "manifest.json");
    await writeFile(manifest, JSON.stringify({
        version: 1,
        cases: [
            { id: "deterministic", fixture: "input.bin", output: "outputs/deterministic.bin", golden: "golden.bin", comparison: "deterministic" },
            { id: "adapter-absent", fixture: "input.bin", adapter: "absent", comparison: "modelDependent", expected: "unavailable" },
        ],
    }));

    const report = await runEvidence({ manifestPath: manifest, evidenceDir: evidence, root });
    assert.equal(report.cases[0].status, "passed");
    assert.equal(report.cases[1].status, "passed");
    assert.equal(report.cases[1].adapter, "absent");
    assert.equal(await readFile(fixture, "utf8"), "fixture");
    assert.deepEqual(Object.keys(report), ["manifest", "cases"]);
    assert.match(await readFile(join(evidence, "manifest.json"), "utf8"), /deterministic/);
});
