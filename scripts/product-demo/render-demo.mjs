#!/usr/bin/env node

import { access, mkdtemp, mkdir, rm, stat } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { prepareDemoProject } from "./prepare-demo.mjs";

function parseArgs(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (!["--home", "--edit", "--output", "--poster", "--project-dir"].includes(name)) {
      throw new Error(`Unexpected argument: ${name}`);
    }
    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`Missing value for ${name}`);
    }
    const key = name.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    options[key] = value;
    index += 1;
  }

  if (!options.output) {
    throw new Error("Usage: render-demo.mjs --home <image> --edit <image> --output <video.mp4> [--poster <image.jpg>] [--project-dir <prepared project>]");
  }
  if (!options.projectDir && (!options.home || !options.edit)) {
    throw new Error("--home and --edit are required when --project-dir is not provided");
  }
  if (extname(options.output).toLowerCase() !== ".mp4") {
    throw new Error("The video output must use the .mp4 extension");
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));
const outputPath = resolve(options.output);
const posterPath = resolve(options.poster ?? join(dirname(outputPath), `${basename(outputPath, ".mp4")}-poster.jpg`));
const environment = { ...process.env, HYPERFRAMES_NO_TELEMETRY: "1" };
const madeProject = !options.projectDir;

function run(command, args) {
  execFileSync(command, args, { env: environment, stdio: "inherit" });
}

async function assertDoesNotExist(path) {
  try {
    await access(path);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  throw new Error(`Refusing to overwrite existing output: ${path}`);
}

await mkdir(dirname(outputPath), { recursive: true });
await mkdir(dirname(posterPath), { recursive: true });
await assertDoesNotExist(outputPath);
await assertDoesNotExist(posterPath);

const temporaryRoot = madeProject ? await mkdtemp(join(tmpdir(), "toolbox-product-demo-")) : undefined;
const projectPath = madeProject ? join(temporaryRoot, "composition") : resolve(options.projectDir);

try {
  if (madeProject) {
    await prepareDemoProject({
      homeScreenshot: options.home,
      editScreenshot: options.edit,
      projectDir: projectPath,
    });
  }

  run("npx", ["--yes", "hyperframes@0.8.71", "check", projectPath, "--at-transitions"]);
  run("npx", [
    "--yes",
    "hyperframes@0.8.71",
    "snapshot",
    projectPath,
    "--at=1.1,5.6,10.1,14.6,19.1",
    "--no-end",
    `--output=${join(projectPath, "render-checks")}`,
  ]);
  run("npx", [
    "--yes",
    "hyperframes@0.8.71",
    "render",
    projectPath,
    "--quality",
    "delivery",
    "--workers",
    "4",
    "--video-frame-format",
    "png",
    "--output",
    outputPath,
  ]);
  run("ffmpeg", [
    "-hide_banner",
    "-loglevel",
    "error",
    "-y",
    "-ss",
    "5.6",
    "-i",
    outputPath,
    "-frames:v",
    "1",
    "-vf",
    "scale=1280:-2:flags=lanczos",
    "-q:v",
    "2",
    "-an",
    posterPath,
  ]);

  const video = JSON.parse(
    execFileSync(
      "ffprobe",
      ["-v", "error", "-show_entries", "format=duration,size:stream=codec_type,codec_name,width,height", "-of", "json", outputPath],
      { encoding: "utf8" },
    ),
  );
  const duration = Number(video.format?.duration);
  const videoStream = video.streams?.find((stream) => stream.codec_type === "video");
  const audioStream = video.streams?.find((stream) => stream.codec_type === "audio");
  const fileSize = (await stat(outputPath)).size;
  if (
    Math.abs(duration - 20.2) > 0.5 ||
    videoStream?.codec_name !== "h264" ||
    videoStream?.width !== 1920 ||
    videoStream?.height !== 1080 ||
    audioStream?.codec_name !== "aac"
  ) {
    throw new Error(
      `Unexpected render: duration=${duration}, video=${videoStream?.codec_name} ${videoStream?.width}x${videoStream?.height}, audio=${audioStream?.codec_name}`,
    );
  }
  if (fileSize < 100_000) {
    throw new Error(`Rendered video is unexpectedly small (${fileSize} bytes)`);
  }
  if ((await stat(posterPath)).size < 5_000) {
    throw new Error(`Rendered poster is unexpectedly small: ${posterPath}`);
  }

  console.log(`Rendered ${outputPath} (${duration.toFixed(2)}s, ${fileSize} bytes)`);
  console.log(`Poster: ${posterPath}`);
} finally {
  if (temporaryRoot) {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}
