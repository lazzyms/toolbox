import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { inspectQpdf, readPin } from "../scripts/qpdf-pin.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtures = mkdtempSync(join(tmpdir(), "toolbox-qpdf-release-"));
const pin = readPin();

function machO(cpu) {
  const bytes = Buffer.alloc(64);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(cpu, 4);
  return bytes;
}

function universal(...cpus) {
  const bytes = Buffer.alloc(8 + cpus.length * 20);
  bytes.writeUInt32BE(0xcafebabe, 0);
  bytes.writeUInt32BE(cpus.length, 4);
  cpus.forEach((cpu, index) => bytes.writeUInt32BE(cpu, 8 + index * 20));
  return bytes;
}

function pe(machine) {
  const bytes = Buffer.alloc(0x100);
  bytes.write("MZ", 0, "latin1");
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.write("PE\0\0", 0x80, "latin1");
  bytes.writeUInt16LE(machine, 0x84);
  return bytes;
}

const ARM64_MACH_O = 0x0100000c;
const X86_64_MACH_O = 0x01000007;
const ARM64_PE = 0xaa64;
const X86_64_PE = 0x8664;

function fixture(name, bytes) {
  const path = join(fixtures, name);
  writeFileSync(path, bytes);
  return path;
}

const reportsVersion = (version) => () => version;

const cases = [
  { name: "passes a native arm64 qpdf for macOS", target: "aarch64-apple-darwin", bytes: machO(ARM64_MACH_O), version: "12.4.1", cause: null },
  { name: "passes a universal qpdf that carries an arm64 slice for macOS", target: "aarch64-apple-darwin", bytes: universal(X86_64_MACH_O, ARM64_MACH_O), version: "12.4.1", cause: null },
  { name: "passes a native x64 qpdf for Windows x86_64", target: "x86_64-pc-windows-msvc", bytes: pe(X86_64_PE), version: pin.windows.version, cause: null },
  { name: "passes a native arm64 qpdf for Windows arm64", target: "aarch64-pc-windows-msvc", bytes: pe(ARM64_PE), version: pin.windows.version, cause: null },
  { name: "passes a qpdf exactly at the minimum", target: "aarch64-apple-darwin", bytes: machO(ARM64_MACH_O), version: pin.minimumVersion, cause: null },
  { name: "fails an emulated x64 qpdf on Windows arm64", target: "aarch64-pc-windows-msvc", bytes: pe(X86_64_PE), version: pin.windows.version, cause: "wrong-architecture" },
  { name: "fails an arm64 qpdf on Windows x86_64", target: "x86_64-pc-windows-msvc", bytes: pe(ARM64_PE), version: pin.windows.version, cause: "wrong-architecture" },
  { name: "fails an x86_64-only qpdf on macOS arm64", target: "aarch64-apple-darwin", bytes: machO(X86_64_MACH_O), version: "12.4.1", cause: "wrong-architecture" },
  { name: "fails a Windows qpdf staged for macOS", target: "aarch64-apple-darwin", bytes: pe(ARM64_PE), version: "12.4.1", cause: "wrong-architecture" },
  { name: "fails a script that is not a native executable", target: "aarch64-apple-darwin", bytes: Buffer.from("#!/bin/sh\necho 'qpdf version 12.4.1'\n"), version: "12.4.1", cause: "wrong-architecture" },
  { name: "fails a qpdf below the minimum", target: "aarch64-pc-windows-msvc", bytes: pe(ARM64_PE), version: "6.0.0", cause: "too-old" },
  { name: "fails a qpdf that reports no version", target: "x86_64-pc-windows-msvc", bytes: pe(X86_64_PE), version: null, cause: "unrunnable" },
];

for (const [index, entry] of cases.entries()) {
  test(entry.name, () => {
    const path = fixture(`case-${index}`, entry.bytes);
    const result = inspectQpdf({ path, target: entry.target, pin, readVersion: reportsVersion(entry.version) });
    assert.equal(result.cause, entry.cause, result.message);
  });
}

test("fails a missing qpdf without running anything", () => {
  const result = inspectQpdf({
    path: join(fixtures, "absent", "qpdf.exe"),
    target: "aarch64-pc-windows-msvc",
    pin,
    readVersion: () => assert.fail("a missing qpdf must not run"),
  });
  assert.equal(result.cause, "missing");
});

test("does not run a qpdf built for another architecture", () => {
  const path = fixture("emulated.exe", pe(X86_64_PE));
  const result = inspectQpdf({ path, target: "aarch64-pc-windows-msvc", pin, readVersion: () => assert.fail("a foreign qpdf must not run") });
  assert.equal(result.cause, "wrong-architecture");
  assert.match(result.message, /emulated/);
});

test("pins one minimum and exact, checksummed Windows archives for each Windows target", () => {
  assert.match(pin.minimumVersion, /^\d+\.\d+\.\d+$/);
  for (const target of ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]) {
    const archive = pin.windows.archives[target];
    assert.ok(archive.url.endsWith(`qpdf-${pin.windows.version}-${target.startsWith("aarch64") ? "msvc-arm64" : "msvc64"}.zip`), archive.url);
    assert.match(archive.sha256, /^[0-9a-f]{64}$/);
  }
});

function readiness(args) {
  return spawnSync(process.execPath, ["scripts/check-release-readiness.mjs", ...args], { cwd: projectRoot, encoding: "utf8" });
}

test("release readiness names the cause when a release qpdf is missing", () => {
  const result = readiness(["--qpdf", join(fixtures, "absent", "qpdf"), "--target", "aarch64-apple-darwin"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /qpdf release check failed \(missing\)/);
});

test("release readiness names the cause when a release qpdf is emulated", () => {
  const result = readiness(["--qpdf", fixture("readiness-emulated.exe", pe(X86_64_PE)), "--target", "aarch64-pc-windows-msvc"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /qpdf release check failed \(wrong-architecture\)/);
});

const compiler = process.platform === "darwin" && process.arch === "arm64" && spawnSync("cc", ["--version"]).status === 0;

function nativeFakeQpdf(name, version) {
  const directory = join(fixtures, name);
  mkdirSync(directory, { recursive: true });
  const source = join(directory, "qpdf.c");
  writeFileSync(source, `#include <stdio.h>\nint main(void) { puts("qpdf version ${version}"); return 0; }\n`);
  const path = join(directory, "qpdf");
  const build = spawnSync("cc", ["-arch", "arm64", source, "-o", path], { encoding: "utf8" });
  assert.equal(build.status, 0, build.stderr);
  return path;
}

test("release readiness names the cause when a native release qpdf is too old", { skip: !compiler }, () => {
  const result = readiness(["--qpdf", nativeFakeQpdf("readiness-old", "6.0.0"), "--target", "aarch64-apple-darwin"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /qpdf release check failed \(too-old\)/);
});

test("release readiness passes a native release qpdf at the pinned minimum", { skip: !compiler }, () => {
  const result = readiness(["--qpdf", nativeFakeQpdf("readiness-valid", pin.minimumVersion), "--target", "aarch64-apple-darwin"]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /qpdf release check passed/);
});

function bundleMacOS(qpdf) {
  const app = join(fixtures, `app-${Math.random().toString(36).slice(2)}`, "Toolbox.app");
  mkdirSync(join(app, "Contents", "Resources"), { recursive: true });
  const helper = fixture("helper", Buffer.from("#!/bin/sh\nexit 0\n"));
  const result = spawnSync("bash", ["scripts/bundle-qpdf.sh", app], {
    cwd: projectRoot,
    encoding: "utf8",
    env: { ...process.env, QPDF: qpdf, PDFTOPPM: helper, PDFTOTEXT: helper, TESSERACT: helper, TAURI_SIGNING_PRIVATE_KEY: "" },
  });
  return { result, resources: readdirSync(join(app, "Contents", "Resources")) };
}

test("the macOS bundling script refuses a qpdf below the minimum before bundling anything", { skip: !compiler }, () => {
  const { result, resources } = bundleMacOS(nativeFakeQpdf("bundle-old", "6.0.0"));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /\(too-old\)/);
  assert.match(result.stderr, /bundle-qpdf: refusing qpdf/);
  assert.deepEqual(resources, []);
});

test("the macOS bundling script refuses a qpdf that is not a native arm64 binary", { skip: process.platform === "win32" }, () => {
  const { result, resources } = bundleMacOS(fixture("bundle-intel-qpdf", machO(X86_64_MACH_O)));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /\(wrong-architecture\)/);
  assert.deepEqual(resources, []);
});

test("the macOS bundling script refuses a qpdf path that does not exist", { skip: process.platform === "win32" }, () => {
  const { result, resources } = bundleMacOS(join(fixtures, "absent", "qpdf"));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /\(missing\)/);
  assert.deepEqual(resources, []);
});
