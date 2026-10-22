#!/usr/bin/env node
// Captures Microsoft Store screenshots of the native Windows app through tauri-driver.
// Sample files are generated fictional documents; every capture writes a new file.

import { execFileSync, spawn } from "node:child_process";
import { access, mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { deflateSync } from "node:zlib";
import webdriver from "selenium-webdriver";

const { Builder, By, until } = webdriver;

if (process.platform !== "win32") throw new Error("Store screenshots must be captured on Windows");

const temp = process.env.RUNNER_TEMP ?? process.env.TEMP ?? ".";
const outputDirectory = resolve(temp, "toolbox-store-screenshots");
const samplesDirectory = resolve(temp, "toolbox-store-samples");
const webviewUserDataFolder = resolve(temp, "toolbox-store-webview2");
// scripts/build-msix.ps1 stages the exact files the Store package contains.
const executable = resolve(".artifacts/msix/stage/toolbox.exe");
const report = {
  startedAt: new Date().toISOString(),
  commit: process.env.GITHUB_SHA ?? null,
  run: process.env.GITHUB_RUN_ID ?? null,
  build: "scripts/build-msix.ps1 stage folder (release x86_64-pc-windows-msvc, tauri.windows-store.conf.json, updates off, vision and OCR staged)",
  scenes: [],
};

// ---------- Sample files ----------

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buffer) => {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const pngChunk = (type, data) => {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
};
const mix = (a, b, t) => a.map((value, index) => Math.round(value + (b[index] - value) * Math.min(1, Math.max(0, t))));

// A stylised landscape: sky gradient, sun, three hill layers and a lake reflection.
function landscapePng(width, height, palette) {
  const horizon = Math.round(height * 0.68);
  const hills = palette.hills.map((colour, layer) => ({
    colour,
    base: height * (0.42 + layer * 0.09),
    amplitude: height * (0.09 - layer * 0.015),
    frequency: 2.2 + layer * 1.3,
    phase: layer * 1.7 + palette.seed,
  }));
  const ridge = (hill, x) => hill.base
    + Math.sin((x / width) * Math.PI * hill.frequency + hill.phase) * hill.amplitude
    + Math.sin((x / width) * Math.PI * hill.frequency * 2.7 + hill.phase * 2) * hill.amplitude * 0.35;
  const scene = (x, y) => {
    let colour = mix(palette.skyTop, palette.skyBottom, y / horizon);
    const sunDistance = Math.hypot(x - width * palette.sunX, y - height * 0.33);
    if (sunDistance < height * 0.09) colour = palette.sun;
    else if (sunDistance < height * 0.2) colour = mix(palette.sun, colour, (sunDistance - height * 0.09) / (height * 0.11));
    for (const hill of hills) if (y >= ridge(hill, x)) colour = mix(hill.colour, palette.skyBottom, 0.12 - (y - ridge(hill, x)) / height);
    return colour;
  };
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(1 + width * 3);
    for (let x = 0; x < width; x += 1) {
      let colour;
      if (y < horizon) colour = scene(x, y);
      else {
        const mirrored = scene(x + Math.sin(y * 0.35) * 3, horizon - (y - horizon) * 1.4);
        colour = mix(mirrored, palette.water, 0.45 + ((y - horizon) / (height - horizon)) * 0.3);
      }
      row.set(colour, 1 + x * 3);
    }
    rows.push(row);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", header),
    pngChunk("IDAT", deflateSync(Buffer.concat(rows))),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

const palettes = {
  dusk: { seed: 0.3, sunX: 0.68, skyTop: [36, 52, 110], skyBottom: [244, 160, 120], sun: [255, 228, 170], water: [40, 60, 110], hills: [[112, 86, 140], [70, 60, 110], [34, 36, 70]] },
  morning: { seed: 2.1, sunX: 0.3, skyTop: [96, 160, 230], skyBottom: [214, 236, 250], sun: [255, 250, 220], water: [60, 120, 170], hills: [[120, 170, 120], [70, 130, 90], [36, 84, 64]] },
  autumn: { seed: 4.2, sunX: 0.52, skyTop: [70, 110, 170], skyBottom: [250, 210, 150], sun: [255, 240, 200], water: [70, 90, 120], hills: [[214, 140, 70], [170, 90, 50], [100, 54, 40]] },
};

const pdfText = (value) => value.replaceAll("\\", "\\\\").replaceAll("(", "\\(").replaceAll(")", "\\)");

// A fictional multi-page report with headings, body text and a bar chart on each page.
function reportPdf(title, accent) {
  const pages = [
    ["Overview", "Northwind Studio is a fictional design team used for this sample.", "This document contains no personal data."],
    ["Quarterly results", "Revenue grew across all three product lines.", "Figures below are invented for illustration."],
    ["Roadmap", "Next quarter focuses on accessibility and offline tools.", "Milestones are reviewed every two weeks."],
    ["Appendix", "Glossary, methodology and fictional sources.", "Prepared with Toolbox sample data."],
  ];
  const [r, g, b] = accent.map((value) => (value / 255).toFixed(3));
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", null, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>"];
  const kids = [];
  pages.forEach(([heading, line1, line2], index) => {
    const bars = [0.55, 0.8, 0.65, 0.95].map((height, bar) => `${r} ${g} ${b} rg ${90 + bar * 110} 200 70 ${Math.round(height * 220)} re f`).join("\n");
    const content = [
      `${r} ${g} ${b} rg 0 742 612 50 re f`,
      "BT 1 1 1 rg /F2 18 Tf 54 760 Td", `(${pdfText(title)}) Tj ET`,
      "BT 0.1 0.1 0.12 rg /F2 30 Tf 54 680 Td", `(${pdfText(heading)}) Tj ET`,
      "BT 0.25 0.25 0.3 rg /F1 14 Tf 54 640 Td", `(${pdfText(line1)}) Tj 0 -22 Td (${pdfText(line2)}) Tj ET`,
      "0.92 0.93 0.95 rg 54 180 504 300 re f",
      bars,
      "BT 0.4 0.4 0.45 rg /F1 11 Tf 54 60 Td", `(Page ${index + 1} of ${pages.length}) Tj ET`,
    ].join("\n");
    const contentId = objects.length + 2;
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents ${contentId} 0 R >>`);
    objects.push(`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`);
    kids.push(`${contentId - 1} 0 R`);
  });
  objects[1] = `<< /Type /Pages /Kids [${kids.join(" ")}] /Count ${kids.length} >>`;
  let body = "%PDF-1.4\n";
  const offsets = objects.map((object, index) => {
    const offset = Buffer.byteLength(body);
    body += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = Buffer.byteLength(body);
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}`;
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(body, "latin1");
}

async function writeNew(path, data) {
  try {
    await access(path);
    throw new Error(`Refusing to overwrite ${path}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await writeFile(path, data);
  return path;
}

async function createSamples() {
  await mkdir(samplesDirectory, { recursive: true });
  const pdf = (name, title, accent) => writeNew(join(samplesDirectory, name), reportPdf(title, accent));
  const image = (name, palette, width = 1600, height = 1067) => writeNew(join(samplesDirectory, name), landscapePng(width, height, palette));
  return {
    report: await pdf("Northwind quarterly report.pdf", "Northwind Studio - Quarterly report", [37, 99, 235]),
    brief: await pdf("Project brief.pdf", "Northwind Studio - Project brief", [16, 150, 120]),
    invoice: await pdf("Sample invoice.pdf", "Northwind Studio - Sample invoice", [220, 110, 40]),
    dusk: await image("Lake at dusk.png", palettes.dusk),
    morning: await image("Morning hills.png", palettes.morning),
    autumn: await image("Autumn valley.png", palettes.autumn),
    icon: await image("App icon source.png", palettes.dusk, 1024, 1024),
  };
}

// ---------- Driving the app ----------

let driverProcess;
let driver;
const pause = (ms) => new Promise((done) => setTimeout(done, ms));

async function waitForDriverServer() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch("http://127.0.0.1:4444/status", { signal: AbortSignal.timeout(1_000) })).ok) return;
    } catch {
      // tauri-driver is still starting.
    }
    await pause(250);
  }
  throw new Error("tauri-driver did not become ready within 30 seconds");
}

async function clickVisible(xpath, description) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    for (const element of await driver.findElements(By.xpath(xpath))) {
      if (await element.isDisplayed().catch(() => false)) {
        await element.click();
        return;
      }
    }
    await pause(200);
  }
  throw new Error(`Could not find ${description}`);
}

const openTool = (title) => clickVisible(`//button[@aria-label=${JSON.stringify(`Open ${title}`)}]`, `the Open ${title} control`);

async function goHome() {
  for (const section of await driver.findElements(By.css("main.command-center .tool-library-section"))) {
    if (await section.isDisplayed().catch(() => false)) return;
  }
  await clickVisible('//button[normalize-space(.)="← All tools"]', "the All tools control");
  await driver.wait(until.elementLocated(By.css(".tool-library-section")), 10_000);
}

const invoke = (command, args) => driver.executeAsyncScript(
  "const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(() => done(null), (error) => done(String(error)));",
  command,
  args,
);

const dropIntoTool = (paths) => driver.executeScript("window.dispatchEvent(new CustomEvent('toolbox:tool-drop', { detail: arguments[0] }))", paths);

async function waitForText(text) {
  await driver.wait(async () => (await driver.executeScript("return document.body.innerText")).includes(text), 15_000, `“${text}” did not appear`);
}

// Waits for images to decode and the layout to settle before capturing.
async function settle() {
  await driver.wait(() => driver.executeScript("return [...document.images].every((image) => image.complete) && !document.querySelector('[aria-busy=\"true\"]')"), 20_000).catch(() => undefined);
  await pause(1_500);
}

const thumbnailRendered = (page) => driver.executeScript(
  "const button = document.querySelector(`.scene-thumbnail[aria-label='Page ${arguments[0]}']`); const image = button?.querySelector('img'); return Boolean(image && image.complete && image.naturalWidth > 0);",
  page,
);

// The editor renders thumbnails for the page in view and its neighbours, so visit every page once.
async function renderAllPdfThumbnails() {
  await driver.wait(until.elementLocated(By.css(".scene-thumbnail")), 20_000, "PDF page thumbnails did not appear");
  const count = (await driver.findElements(By.css(".scene-thumbnail"))).length;
  for (let page = count; page >= 1; page -= 1) {
    await clickVisible(`//button[contains(@class, "scene-thumbnail") and @aria-label="Page ${page}"]`, `the page ${page} thumbnail`);
    await driver.wait(() => thumbnailRendered(page), 20_000, `Page ${page} thumbnail did not render`);
  }
  await driver.wait(async () => {
    for (let page = 1; page <= count; page += 1) if (!(await thumbnailRendered(page))) return false;
    return true;
  }, 20_000, "Not every PDF page thumbnail rendered");
  return count;
}

// Labels of visible controls the app marks as unavailable in this build, kept in the report as captured.
const unavailableControls = () => driver.executeScript(
  "return [...document.querySelectorAll('button, a')].filter((element) => element.offsetParent && /\\bUnavailable\\b/i.test(element.innerText)).map((element) => element.innerText.replace(/\\s+/g, ' ').trim());",
);

function captureWindow(path, rect) {
  const script = [
    "Add-Type -AssemblyName System.Windows.Forms, System.Drawing",
    "$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds",
    `$left = [Math]::Max($screen.Left, ${Math.round(rect.x)}); $top = [Math]::Max($screen.Top, ${Math.round(rect.y)})`,
    `$right = [Math]::Min($screen.Right, ${Math.round(rect.x + rect.width)}); $bottom = [Math]::Min($screen.Bottom, ${Math.round(rect.y + rect.height)})`,
    "$bitmap = New-Object System.Drawing.Bitmap ($right - $left), ($bottom - $top)",
    "$graphics = [System.Drawing.Graphics]::FromImage($bitmap)",
    "$graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)",
    `$bitmap.Save(${JSON.stringify(path).replaceAll('"', "'")}, [System.Drawing.Imaging.ImageFormat]::Png)`,
  ].join("; ");
  execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { stdio: "inherit" });
}

async function capture(index, name) {
  const base = `${String(index).padStart(2, "0")}-${name}`;
  const webviewPath = join(outputDirectory, `${base}.png`);
  const windowPath = join(outputDirectory, `${base}-window.png`);
  await writeNew(webviewPath, Buffer.from(await driver.takeScreenshot(), "base64"));
  try {
    captureWindow(windowPath, await driver.manage().window().getRect());
  } catch (error) {
    console.warn(`Window capture failed for ${base}:`, error);
  }
  return base;
}

async function setTheme(theme) {
  await driver.executeScript("document.body.dataset.theme = arguments[0]; localStorage.setItem('toolbox-theme', arguments[0]);", theme);
  await pause(500);
}

async function run() {
  await mkdir(outputDirectory, { recursive: true });
  await mkdir(webviewUserDataFolder, { recursive: true });
  const samples = await createSamples();

  driverProcess = spawn("tauri-driver", [], { stdio: "inherit", windowsHide: true });
  await waitForDriverServer();
  driver = await new Builder()
    .usingServer("http://127.0.0.1:4444")
    .withCapabilities({ browserName: "wry", "tauri:options": { application: executable, webviewOptions: { userDataFolder: webviewUserDataFolder } } })
    .build();
  await driver.wait(until.elementLocated(By.css("main.command-center")), 30_000, "Toolbox home did not open");
  await driver.manage().window().maximize().catch(() => undefined);
  await pause(1_000);
  report.window = await driver.manage().window().getRect();

  const scenes = [
    ["home", async () => {}],
    ["pdf-editor", async () => {
      const error = await invoke("open_paths", { paths: [samples.report] });
      if (error) throw new Error(error);
      return { thumbnails: await renderAllPdfThumbnails() };
    }],
    ["organize-pdf", async () => {
      await openTool("Organize PDF");
      await dropIntoTool([samples.brief]);
      await waitForText("Project brief");
      const thumbnails = await renderAllPdfThumbnails();
      // Select pages 2-3 and rotate them, the way Organize is used.
      await clickVisible('//button[contains(@class, "scene-thumbnail") and @aria-label="Page 2"]', "the page 2 thumbnail");
      // WebView2's WebDriver drops held modifier keys, so send the Shift-click the thumbnail handles.
      await driver.executeScript("document.querySelector(`.scene-thumbnail[aria-label='Page 3']`).dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, shiftKey: true }));");
      await driver.wait(async () => (await driver.findElements(By.css('.scene-thumbnail[aria-pressed="true"]'))).length === 2, 10_000, "Pages 2-3 were not selected");
      await clickVisible('//button[@aria-label="Rotate selected pages"]', "the Rotate selected pages control");
      await driver.wait(until.elementLocated(By.css('button[aria-label="Reset edits"]:not([disabled])')), 10_000, "Rotation did not change the document");
      // Pages 2-3 stay selected; wait for their rotated thumbnails.
      for (const page of [2, 3]) await driver.wait(() => thumbnailRendered(page), 20_000, `Rotated page ${page} thumbnail did not render`);
      return { thumbnails, rotatedPages: [2, 3] };
    }],
    ["image-editor", async () => {
      const error = await invoke("open_paths", { paths: [samples.dusk] });
      if (error) throw new Error(error);
      await waitForText("Lake at dusk");
      // An opened image lands on Convert Image Format; switch to the Edit Image tools.
      await clickVisible('//nav[contains(@class, "workspace-command-rail")]//button[@aria-label="Edit Image"]', "the Edit Image tool");
      await driver.wait(until.elementLocated(By.css('[role="tablist"][aria-label="Edit Image"]')), 10_000, "Edit Image tabs did not appear");
      await clickVisible('//*[@role="tablist" and @aria-label="Edit Image"]//button[normalize-space(.)="Crop"]', "the Crop tab");
      await driver.wait(until.elementLocated(By.css(".image-editor-crop-overlay")), 10_000, "Crop selection did not appear");
    }],
    ["compress-images", async () => {
      await openTool("Compress Images");
      await dropIntoTool([samples.dusk, samples.morning, samples.autumn]);
      await waitForText("Autumn valley");
    }],
    ["merge-pdf", async () => {
      await openTool("Merge PDF");
      await dropIntoTool([samples.report, samples.brief, samples.invoice]);
      await waitForText("Sample invoice");
    }],
    ["lock-unlock", async () => {
      await openTool("Lock & Unlock");
      await dropIntoTool([samples.report, samples.brief, samples.invoice]);
      await waitForText("Project brief");
    }],
    ["pdf-to-images", async () => {
      await openTool("PDF to Images");
      await dropIntoTool([samples.brief]);
      await waitForText("Project brief");
    }],
    ["app-icons", async () => {
      await openTool("Generate App Icons");
      await dropIntoTool([samples.icon]);
      await waitForText("App icon source");
    }],
    ["home-light", async () => setTheme("light")],
  ];

  for (const [index, [name, prepare]] of scenes.entries()) {
    try {
      await goHome();
      const details = await prepare();
      await settle();
      const file = await capture(index + 1, name);
      report.scenes.push({ name, status: "captured", file, unavailable: await unavailableControls(), ...details });
    } catch (error) {
      console.error(`Scene ${name} failed:`, error);
      let file;
      try {
        file = await capture(index + 1, `${name}-failed`);
      } catch {
        file = undefined;
      }
      report.scenes.push({ name, status: "failed", error: error instanceof Error ? error.message : String(error), file });
    }
  }
}

try {
  await run();
} catch (error) {
  console.error("Store screenshot capture failed:", error);
  report.error = error instanceof Error ? error.message : String(error);
} finally {
  if (driver) await driver.quit().catch(() => undefined);
  if (driverProcess?.pid && driverProcess.exitCode === null) {
    try {
      execFileSync("taskkill", ["/pid", String(driverProcess.pid), "/t", "/f"], { stdio: "ignore" });
    } catch {
      driverProcess.kill();
    }
  }
  report.finishedAt = new Date().toISOString();
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(join(outputDirectory, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
}

if (report.error || !report.scenes.some((scene) => scene.status === "captured")) process.exitCode = 1;
console.log(`Store screenshots: ${outputDirectory}`);
