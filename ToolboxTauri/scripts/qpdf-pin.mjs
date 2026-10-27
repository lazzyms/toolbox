import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";

export const PIN_PATH = new URL("../qpdf-pin.json", import.meta.url);

export const TARGETS = {
  "aarch64-apple-darwin": { format: "mach-o", architecture: "aarch64" },
  "x86_64-pc-windows-msvc": { format: "pe", architecture: "x86_64" },
  "aarch64-pc-windows-msvc": { format: "pe", architecture: "aarch64" },
};

const MACH_O_CPU = { 0x01000007: "x86_64", 0x0100000c: "aarch64" };
const PE_MACHINE = { 0x014c: "i686", 0x8664: "x86_64", 0xaa64: "aarch64" };

export function readPin(path = PIN_PATH) {
  const pin = JSON.parse(readFileSync(path, "utf8"));
  const minimum = parseVersion(pin.minimumVersion);
  if (!minimum) throw new Error(`qpdf pin has no valid minimumVersion: ${pin.minimumVersion}`);
  const windowsVersion = parseVersion(pin.windows?.version);
  if (!windowsVersion) throw new Error(`qpdf pin has no exact Windows version: ${pin.windows?.version}`);
  if (compareVersions(windowsVersion, minimum) < 0) throw new Error(`Pinned Windows qpdf ${pin.windows.version} is below the minimum ${pin.minimumVersion}`);
  for (const target of Object.keys(TARGETS).filter((name) => TARGETS[name].format === "pe")) {
    const archive = pin.windows.archives?.[target];
    if (!archive?.url?.includes(pin.windows.version)) throw new Error(`qpdf pin has no ${pin.windows.version} archive for ${target}`);
    if (!/^[0-9a-f]{64}$/.test(archive.sha256 ?? "")) throw new Error(`qpdf pin has no SHA-256 for ${target}`);
  }
  return pin;
}

export function parseVersion(text) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(String(text ?? "").trim());
  return match ? match.slice(1).map(Number) : null;
}

export function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}

export function binaryArchitectures(bytes) {
  if (bytes.length >= 8 && bytes.readUInt32LE(0) === 0xfeedfacf) {
    return { format: "mach-o", architectures: [MACH_O_CPU[bytes.readUInt32LE(4)] ?? "unknown"] };
  }
  const fat = bytes.length >= 8 ? bytes.readUInt32BE(0) : 0;
  if (fat === 0xcafebabe || fat === 0xcafebabf) {
    const stride = fat === 0xcafebabe ? 20 : 32;
    const count = bytes.readUInt32BE(4);
    const architectures = [];
    for (let index = 0; index < count && 8 + (index + 1) * stride <= bytes.length; index += 1) {
      architectures.push(MACH_O_CPU[bytes.readUInt32BE(8 + index * stride)] ?? "unknown");
    }
    return { format: "mach-o", architectures };
  }
  if (bytes.length >= 0x40 && bytes.toString("latin1", 0, 2) === "MZ") {
    const header = bytes.readUInt32LE(0x3c);
    if (header + 6 <= bytes.length && bytes.toString("latin1", header, header + 4) === "PE\0\0") {
      return { format: "pe", architectures: [PE_MACHINE[bytes.readUInt16LE(header + 4)] ?? "unknown"] };
    }
  }
  return { format: "unknown", architectures: [] };
}

export function runVersion(path) {
  const result = spawnSync(path, ["--version"], { encoding: "utf8", timeout: 30_000 });
  if (result.error || result.status !== 0) return null;
  const match = /^qpdf version (\d+\.\d+\.\d+)/m.exec(result.stdout);
  return match ? match[1] : null;
}

export function inspectQpdf({ path, target, pin = readPin(), readVersion = runVersion }) {
  const expected = TARGETS[target];
  if (!expected) throw new Error(`Unsupported release target ${target}; expected one of ${Object.keys(TARGETS).join(", ")}`);
  if (!path || !existsSync(path) || !statSync(path).isFile()) {
    return { cause: "missing", message: `qpdf is missing at ${path}` };
  }
  const found = binaryArchitectures(readFileSync(path));
  if (found.format !== expected.format || !found.architectures.includes(expected.architecture)) {
    const described = found.format === "unknown" ? "not a native executable" : `${found.format} ${found.architectures.join("+")}`;
    return {
      cause: "wrong-architecture",
      message: `qpdf at ${path} is ${described}; ${target} needs a native ${expected.format} ${expected.architecture} binary and would otherwise run emulated or not at all`,
    };
  }
  const version = readVersion(path);
  const parsed = parseVersion(version);
  if (!parsed) return { cause: "unrunnable", message: `qpdf at ${path} did not report a version` };
  if (compareVersions(parsed, parseVersion(pin.minimumVersion)) < 0) {
    return { cause: "too-old", message: `qpdf ${version} at ${path} is below the pinned minimum ${pin.minimumVersion}` };
  }
  return { cause: null, version, message: `qpdf ${version} is native ${expected.architecture} for ${target}` };
}
