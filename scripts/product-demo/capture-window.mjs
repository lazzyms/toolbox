#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { access, mkdir, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";

function parseArgs(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (!["--app", "--output"].includes(name)) {
      throw new Error(`Unexpected argument: ${name}`);
    }
    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`Missing value for ${name}`);
    }
    options[name.slice(2)] = value;
    index += 1;
  }
  if (!options.app || !options.output) {
    throw new Error("Usage: capture-window.mjs --app <Toolbox.app> --output <screenshot.png>");
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));
const appPath = resolve(options.app);
const executablePath = `${appPath}/Contents/MacOS/toolbox`;
const outputPath = resolve(options.output);
try {
  await access(outputPath);
  throw new Error(`Refusing to overwrite existing screenshot: ${outputPath}`);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const processLines = execFileSync("ps", ["-axo", "pid=,command="], { encoding: "utf8" }).split("\n");
const matchingProcess = processLines
  .map((line) => line.trim().match(/^(\d+)\s+(.+)$/))
  .find((match) => match?.[2].startsWith(executablePath));
if (!matchingProcess) {
  throw new Error(`Toolbox is not running from ${appPath}`);
}

const pid = Number(matchingProcess[1]);
const windowScript = `
ObjC.import("CoreGraphics");
function run(argv) {
  const expectedPid = Number(argv[0]);
  const windows = $.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, $.kCGNullWindowID);
  for (let index = 0; index < Number($.CFArrayGetCount(windows)); index += 1) {
    const window = ObjC.castRefToObject($.CFArrayGetValueAtIndex(windows, index));
    const ownerPid = Number(window.objectForKey("kCGWindowOwnerPID")?.js);
    const title = window.objectForKey("kCGWindowName")?.js;
    const layer = Number(window.objectForKey("kCGWindowLayer")?.js);
    if (ownerPid === expectedPid && layer === 0 && title === "Toolbox") {
      console.log(String(window.objectForKey("kCGWindowNumber").js));
      return;
    }
  }
  throw new Error("Could not locate the visible Toolbox window");
}
`;

const windowId = execFileSync(
  "osascript",
  ["-l", "JavaScript", "-e", windowScript, String(pid)],
  { encoding: "utf8" },
).trim();
if (!/^\d+$/.test(windowId)) {
  throw new Error(`Could not read Toolbox window id: ${windowId}`);
}

await mkdir(dirname(outputPath), { recursive: true });
execFileSync("screencapture", ["-x", "-l", windowId, outputPath], { stdio: "inherit" });
if ((await stat(outputPath)).size < 1_000) {
  throw new Error(`Captured screenshot is unexpectedly small: ${outputPath}`);
}
console.log(`Captured Toolbox window ${windowId} to ${outputPath}`);
