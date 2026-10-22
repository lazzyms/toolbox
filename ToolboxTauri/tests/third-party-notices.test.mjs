import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { globMatches, licenseIds, unlistedBinaries } from "../scripts/third-party-notices.mjs";

const { components } = JSON.parse(readFileSync(new URL("../scripts/third-party/native-components.json", import.meta.url), "utf8"));
const licenseRoot = new URL("../scripts/third-party/licenses/", import.meta.url);

test("licenseIds splits SPDX and legacy slash expressions", () => {
    assert.deepEqual(licenseIds("(MIT OR Apache-2.0) AND Unicode-3.0"), ["MIT", "Apache-2.0", "Unicode-3.0"]);
    assert.deepEqual(licenseIds("Apache-2.0 WITH LLVM-exception"), ["Apache-2.0", "LLVM-exception"]);
    assert.deepEqual(licenseIds("MIT/Apache-2.0"), ["MIT", "Apache-2.0"]);
});

test("globMatches is case-insensitive and anchors the whole name", () => {
    assert.ok(globMatches("libheif*.dll", "LIBHEIF-1.dll"));
    assert.ok(globMatches("libstdc%2b%2b*", "libstdc%2b%2b-6.dll"));
    assert.ok(!globMatches("qpdf*.dll", "notqpdf30.dll"));
});

test("each macOS TLS library is credited to the project that ships it", () => {
    const owners = (file) => components.filter((component) => component.files?.some((pattern) => globMatches(pattern, file))).map((component) => component.name);
    assert.deepEqual(owners("libssl.3.dylib"), ["OpenSSL (libssl, libcrypto)"]);
    assert.deepEqual(owners("libssl3.dylib"), ["NSS (Network Security Services)"]);
    assert.deepEqual(owners("libssl-3-x64.dll"), ["OpenSSL (libssl, libcrypto)"]);
});

test("every native component names a license text that ships in the notices", () => {
    for (const component of components) {
        for (const file of component.notices ?? []) {
            assert.ok(existsSync(new URL(`../${file}`, import.meta.url)), `${component.name}: ${file}`);
        }
        if (component.licenseFile) {
            assert.ok(existsSync(new URL(`../${component.licenseFile}`, import.meta.url)), `${component.name}: ${component.licenseFile}`);
            continue;
        }
        for (const id of licenseIds(component.license)) {
            const found = [id, id.replace(/-or-later$/, "-only"), `${id}-only`]
                .some((candidate) => existsSync(new URL(`${candidate}.txt`, licenseRoot)));
            assert.ok(found, `${component.name}: no license text for ${id}`);
        }
    }
});

test("unlistedBinaries flags unknown executables and accepts listed ones", () => {
    const dir = mkdtempSync(join(tmpdir(), "toolbox-notices-"));
    writeFileSync(join(dir, "mystery.dll"), Buffer.from("MZ\x90\x00 fake"));
    writeFileSync(join(dir, "qpdf.exe"), Buffer.from("MZ\x90\x00 fake"));
    writeFileSync(join(dir, "readme.txt"), "MZ is not checked by extension, only by content");
    writeFileSync(join(dir, "notes.txt"), "plain text");
    const unlisted = unlistedBinaries(dir).map((path) => path.slice(dir.length + 1)).sort();
    assert.deepEqual(unlisted, ["mystery.dll", "readme.txt"]);
});
