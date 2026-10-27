// Builds src-tauri/resources/licenses/THIRD-PARTY-NOTICES.txt from the native
// component list, the Rust crates compiled into the shipped binaries, and the
// npm packages bundled into the frontend.
//   node scripts/third-party-notices.mjs             write the notices
//   node scripts/third-party-notices.mjs --check     fail if the committed file is stale
//   node scripts/third-party-notices.mjs --binaries <dir>
//                                                   fail if a shipped executable or library
//                                                   in <dir> is not listed in native-components.json
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync, openSync, readSync, closeSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const dataRoot = join(projectRoot, "scripts", "third-party");
const licenseTextRoot = join(dataRoot, "licenses");
const outputPath = join(projectRoot, "src-tauri", "resources", "licenses", "THIRD-PARTY-NOTICES.txt");
const shippingTargets = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"];
const shippingFeatures = "updater,vision-adapter";
const { components } = JSON.parse(readFileSync(join(dataRoot, "native-components.json"), "utf8"));

const normalize = (text) => text.replace(/\r\n?/g, "\n").split("\n").map((line) => line.trimEnd()).join("\n").trim();
const rule = (title) => `${title}\n${"=".repeat(title.length)}`;

// SPDX expressions name licenses joined by AND/OR/WITH; legacy crates use "/" for OR.
export const licenseIds = (expression) => expression
    .split(/\s+(?:AND|OR|WITH)\s+|\s*\/\s*|[()]/)
    .map((id) => id.trim())
    .filter(Boolean);

const licenseTextFile = (id) => {
    for (const candidate of [id, id.replace(/-or-later$/, "-only"), `${id}-only`]) {
        const path = join(licenseTextRoot, `${candidate}.txt`);
        if (existsSync(path)) return { key: candidate, path };
    }
    throw new Error(`No license text for ${id}. Add scripts/third-party/licenses/${id}.txt from https://github.com/spdx/license-list-data/tree/main/text.`);
};

export const globMatches = (pattern, name) =>
    new RegExp(`^${pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*")}$`, "i").test(name);

const crateLicenseFiles = (directory) => readdirSync(directory)
    .filter((file) => /^(licen[cs]e|copying|notice|copyright|unlicense)/i.test(file) && statSync(join(directory, file)).isFile())
    .sort();

const shippedCrates = () => {
    const crates = new Map();
    for (const target of shippingTargets) {
        const metadata = JSON.parse(execFileSync("cargo", [
            "metadata", "--format-version", "1", "--locked", "--features", shippingFeatures, "--filter-platform", target,
        ], { cwd: join(projectRoot, "src-tauri"), encoding: "utf8", maxBuffer: 256 * 1024 * 1024 }));
        const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
        const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
        const pending = [metadata.resolve.root];
        const seen = new Set();
        while (pending.length) {
            const id = pending.pop();
            if (seen.has(id)) continue;
            seen.add(id);
            for (const dep of nodes.get(id).deps) {
                if (dep.dep_kinds.some((kind) => kind.kind === null)) pending.push(dep.pkg);
            }
        }
        for (const id of seen) {
            const pkg = packages.get(id);
            if (pkg.source) crates.set(`${pkg.name} ${pkg.version}`, pkg);
        }
    }
    return [...crates.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
};

const bundledPackages = () => {
    const lock = JSON.parse(readFileSync(join(projectRoot, "package-lock.json"), "utf8"));
    return Object.entries(lock.packages)
        .filter(([path, entry]) => path && !entry.dev && !entry.devOptional)
        .map(([path, entry]) => ({ name: path.replace(/^.*node_modules\//, ""), version: entry.version, license: entry.license, directory: join(projectRoot, path) }))
        .sort((a, b) => a.name.localeCompare(b.name));
};

// One copy of each distinct license file, listing every package that ships it.
const groupTexts = (packages, readFiles) => {
    const groups = new Map();
    for (const pkg of packages) {
        for (const file of readFiles(pkg)) {
            const text = normalize(readFileSync(join(pkg.directory, file), "utf8"));
            if (!text) continue;
            if (!groups.has(text)) groups.set(text, []);
            groups.get(text).push(`${pkg.name} ${pkg.version} (${file})`);
        }
    }
    return [...groups.entries()]
        .map(([text, owners]) => ({ text, owners: [...new Set(owners)].sort() }))
        .sort((a, b) => a.owners[0].localeCompare(b.owners[0]));
};

export const buildNotices = () => {
    const referenced = new Map();
    const reference = (expression) => {
        for (const id of licenseIds(expression)) {
            const file = licenseTextFile(id);
            referenced.set(file.key, file.path);
        }
    };

    const native = components.map((component) => {
        if (!component.licenseFile) reference(component.license);
        return [
            `${component.name}${component.version ? ` ${component.version}` : ""}`,
            `  License: ${component.license}`,
            `  ${component.copyright}`,
            `  Source: ${component.source}`,
            component.note && `  ${component.note}`,
            component.files && `  Files: ${component.files.join(", ")}`,
            component.licenseFile && `\n${normalize(readFileSync(join(projectRoot, component.licenseFile), "utf8"))}`,
            // Upstream NOTICE, PATENTS and COPYING files that must be reproduced alongside the license text.
            ...(component.notices ?? []).map((file) => `\n  ${basename(file)}:\n\n${normalize(readFileSync(join(projectRoot, file), "utf8"))}`),
        ].filter(Boolean).join("\n");
    });

    const crates = shippedCrates().map((pkg) => ({ ...pkg, directory: dirname(pkg.manifest_path) }));
    const crateLines = crates.map((pkg) => {
        if (pkg.license) reference(pkg.license);
        return `${pkg.name} ${pkg.version} - ${pkg.license ?? `see ${pkg.license_file}`}${pkg.repository ? ` - ${pkg.repository}` : ""}`;
    });
    const crateTexts = groupTexts(crates, (pkg) => crateLicenseFiles(pkg.directory));

    const packages = bundledPackages();
    const packageLines = packages.map((pkg) => {
        reference(pkg.license);
        return `${pkg.name} ${pkg.version} - ${pkg.license}`;
    });
    const packageTexts = groupTexts(packages, (pkg) => existsSync(pkg.directory) ? crateLicenseFiles(pkg.directory) : []);
    const missingPackages = packages.filter((pkg) => !existsSync(pkg.directory));
    if (missingPackages.length) throw new Error(`Run npm ci first; missing ${missingPackages.map((pkg) => pkg.name).join(", ")}`);

    const groupedText = ({ owners, text }) => `Used by: ${owners.join(", ")}\n\n${text}`;
    const standardTexts = [...referenced.entries()].sort(([a], [b]) => a.localeCompare(b))
        .map(([key, path]) => `${rule(key)}\n\n${normalize(readFileSync(path, "utf8"))}`);

    return [
        rule("Toolbox third-party notices"),
        "",
        "Toolbox is open-source software by Maulik Sompura, released under the MIT license",
        "(https://github.com/lazzyms/toolbox). It includes the third-party software listed below.",
        "Each component remains under its own license. Where a component is licensed under the",
        "GPL or LGPL, you can get its complete source code from the links below, and the full",
        "Toolbox source from https://github.com/lazzyms/toolbox. Report a missing or wrong",
        "notice at https://github.com/lazzyms/toolbox/issues.",
        "",
        "This file is generated by scripts/third-party-notices.mjs. Do not edit it by hand.",
        "",
        rule("Part 1. Native helpers, libraries, fonts and models"),
        "",
        native.join("\n\n"),
        "",
        rule("Part 2. Rust crates compiled into Toolbox"),
        "",
        crateLines.join("\n"),
        "",
        rule("Part 2a. License and notice files shipped by those crates"),
        "",
        crateTexts.map(groupedText).join(`\n\n${"-".repeat(72)}\n\n`),
        "",
        rule("Part 3. JavaScript packages bundled into the Toolbox interface"),
        "",
        packageLines.join("\n"),
        "",
        rule("Part 3a. License files shipped by those packages"),
        "",
        packageTexts.map(groupedText).join(`\n\n${"-".repeat(72)}\n\n`),
        "",
        rule("Part 4. License texts"),
        "",
        standardTexts.join("\n\n\n"),
        "",
    ].join("\n");
};

const isNativeBinary = (path) => {
    const header = Buffer.alloc(4);
    const handle = openSync(path, "r");
    try { readSync(handle, header, 0, 4, 0); } finally { closeSync(handle); }
    const magic = header.readUInt32BE(0);
    return header.subarray(0, 2).toString("latin1") === "MZ"
        || [0xfeedface, 0xfeedfacf, 0xcefaedfe, 0xcffaedfe, 0xcafebabe].includes(magic);
};

const walk = (directory) => readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return walk(path);
    return entry.isFile() && statSync(path).size >= 4 ? [path] : [];
});

export const unlistedBinaries = (directory) => walk(directory)
    .filter(isNativeBinary)
    .filter((path) => !components.some((component) => component.files?.some((pattern) => globMatches(pattern, basename(path)))));

const main = () => {
    const [mode, argument] = process.argv.slice(2);
    if (mode === "--binaries") {
        if (!argument || !existsSync(argument)) throw new Error("--binaries needs the directory that will be shipped");
        const unlisted = unlistedBinaries(argument);
        if (unlisted.length) {
            console.error("These shipped binaries have no entry in scripts/third-party/native-components.json:");
            for (const path of unlisted) console.error(`  ${path}`);
            process.exit(1);
        }
        console.log(`Every native binary under ${argument} has a third-party notice.`);
        return;
    }
    const notices = buildNotices();
    if (mode === "--check") {
        // Windows checkouts may convert the committed file to CRLF.
        const current = existsSync(outputPath) ? readFileSync(outputPath, "utf8").replace(/\r\n/g, "\n") : "";
        if (current !== notices) {
            console.error("THIRD-PARTY-NOTICES.txt is out of date. Run: node scripts/third-party-notices.mjs");
            process.exit(1);
        }
        console.log("THIRD-PARTY-NOTICES.txt is up to date.");
        return;
    }
    mkdirSync(dirname(outputPath), { recursive: true });
    writeFileSync(outputPath, notices);
    console.log(`Wrote ${outputPath} (${Math.round(notices.length / 1024)} KB)`);
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
