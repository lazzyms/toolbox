#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { access, mkdir, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const [inputArg, outputArg, ...rest] = process.argv.slice(2);
if (!inputArg || !outputArg || rest.length > 0) {
  throw new Error("Usage: flatten-screenshot.mjs <screenshot.png> <output.jpg>");
}

const inputPath = resolve(inputArg);
const outputPath = resolve(outputArg);
try {
  await access(outputPath);
  throw new Error(`Refusing to overwrite existing screenshot: ${outputPath}`);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
await mkdir(dirname(outputPath), { recursive: true });
execFileSync(
  "ffmpeg",
  [
    "-hide_banner",
    "-loglevel",
    "error",
    "-y",
    "-f",
    "lavfi",
    "-i",
    "color=c=0x171717:s=16x16",
    "-i",
    inputPath,
    "-filter_complex",
    "[0:v][1:v]scale2ref[bg][fg];[bg][fg]overlay=shortest=1:format=auto",
    "-frames:v",
    "1",
    "-q:v",
    "2",
    outputPath,
  ],
  { stdio: "inherit" },
);

if ((await stat(outputPath)).size < 1_000) {
  throw new Error(`Flattened screenshot is unexpectedly small: ${outputPath}`);
}
