#!/usr/bin/env node

import { createHash } from "node:crypto";
import { copyFile, cp, mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, join, resolve } from "node:path";

const MUSIC_URL = "https://assets.mixkit.co/music/1167/1167.mp3";
const MUSIC_SHA256 = "a7f05a29d07a84d38072ccd2b35204bca812db86e75b2a837e71cc144d3e739b";
const TEMPLATE_DIR = join(dirname(fileURLToPath(import.meta.url)), "composition");

export async function prepareDemoProject({ homeScreenshot, editScreenshot, projectDir }) {
  if (!homeScreenshot || !editScreenshot || !projectDir) {
    throw new Error("homeScreenshot, editScreenshot, and projectDir are required");
  }

  const homePath = resolve(homeScreenshot);
  const editPath = resolve(editScreenshot);
  const projectPath = resolve(projectDir);
  await mkdir(projectPath, { recursive: false });
  await cp(TEMPLATE_DIR, projectPath, { recursive: true });
  await mkdir(join(projectPath, "assets", "music"), { recursive: true });
  await copyFile(homePath, join(projectPath, "assets", "toolbox-home.jpg"));
  await copyFile(editPath, join(projectPath, "assets", "toolbox-edit-success.jpg"));

  const response = await fetch(MUSIC_URL, {
    headers: { Accept: "audio/mpeg", "User-Agent": "toolbox-product-demo/1.0" },
    signal: AbortSignal.timeout(60_000),
  });
  if (!response.ok) {
    throw new Error(`Mixkit download failed with HTTP ${response.status}`);
  }

  const sourceAudio = Buffer.from(await response.arrayBuffer());
  const actualHash = createHash("sha256").update(sourceAudio).digest("hex");
  if (actualHash !== MUSIC_SHA256) {
    throw new Error(`Mixkit source checksum mismatch: ${actualHash}`);
  }

  const sourcePath = join(projectPath, "source-music.mp3");
  const musicPath = join(projectPath, "assets", "music", "music-bed.mp3");
  await writeFile(sourcePath, sourceAudio, { flag: "wx" });
  try {
    execFileSync(
      "ffmpeg",
      [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        sourcePath,
        "-t",
        "20.2",
        "-af",
        "afade=t=in:st=0:d=0.6,afade=t=out:st=18.8:d=1.4",
        "-codec:a",
        "libmp3lame",
        "-b:a",
        "160k",
        "-ar",
        "44100",
        "-ac",
        "2",
        musicPath,
      ],
      { stdio: "inherit" },
    );
  } finally {
    await rm(sourcePath, { force: true });
  }

  await writeFile(
    join(projectPath, "music-source.txt"),
    `Mixkit source: ${MUSIC_URL}\nSHA-256: ${MUSIC_SHA256}\n` +
      "Track: Close Up by Michael Ramir C.\nLicense: Mixkit Stock Music Free License.\n",
    { flag: "wx" },
  );

  return projectPath;
}

function parseArgs(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    if (!["--home", "--edit", "--project-dir"].includes(name)) {
      throw new Error(`Unexpected argument: ${name}`);
    }
    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`Missing value for ${name}`);
    }
    options[{ "--home": "homeScreenshot", "--edit": "editScreenshot", "--project-dir": "projectDir" }[name]] = value;
    index += 1;
  }
  if (!options.homeScreenshot || !options.editScreenshot || !options.projectDir) {
    throw new Error("Usage: prepare-demo.mjs --home <image> --edit <image> --project-dir <directory>");
  }
  if (!isAbsolute(options.projectDir)) {
    options.projectDir = resolve(options.projectDir);
  }
  return options;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const projectPath = await prepareDemoProject(parseArgs(process.argv.slice(2)));
  console.log(`Demo project ready: ${projectPath}`);
}
