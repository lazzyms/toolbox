// Renders every state of every design board to design/screens/ so agents and reviewers can compare the app against the approved design.
import { chromium } from "@playwright/test";
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const boardsDir = resolve("design/boards");
const screensDir = resolve("design/screens");
const { boards } = JSON.parse(readFileSync(join(boardsDir, "canvas.json"), "utf8"));
const entityDecode = (text) => text.replaceAll("&quot;", '"').replaceAll("&#39;", "'").replaceAll("&amp;", "&");

function statesOf(file) {
    const html = readFileSync(join(boardsDir, file), "utf8");
    const match = html.match(/data-props=(["'])([\s\S]*?)\1/);
    const schema = JSON.parse(entityDecode(match[2]));
    const states = [{ name: "default", query: "" }];
    for (const [key, spec] of Object.entries(schema)) {
        if (key === "$preview" || key === "theme" || key === "platform") continue;
        const options = spec.editor === "boolean" ? [true, false] : spec.options ?? [];
        for (const option of options) {
            if (option === spec.default) continue;
            const slug = `${key}-${String(option).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`;
            states.push({ name: slug, query: `${encodeURIComponent(key)}=${encodeURIComponent(option)}` });
        }
    }
    return { states, size: schema.$preview ?? { width: 1280, height: 800 } };
}

rmSync(screensDir, { recursive: true, force: true });
mkdirSync(screensDir, { recursive: true });
const browser = await chromium.launch();
const failures = [];
const index = [];
for (const [file, board] of Object.entries(boards)) {
    const { states, size } = statesOf(file);
    const page = await browser.newPage({ viewport: { width: size.width, height: size.height } });
    page.on("pageerror", (error) => failures.push(`${file}: ${error.message}`));
    for (const state of states) {
        const url = `${pathToFileURL(join(boardsDir, file)).href}${state.query ? `?${state.query}` : ""}`;
        await page.goto(url);
        await page.waitForSelector('html[data-ready="1"]', { timeout: 5000 });
        const unresolved = await page.evaluate(() => document.body.innerHTML.match(/\{\{[^}]+\}\}/)?.[0]);
        if (unresolved) failures.push(`${file} ${state.name}: unresolved ${unresolved}`);
        const shot = `${file.replace(".dc.html", "")}--${state.name}.jpg`;
        await page.screenshot({ path: join(screensDir, shot), type: "jpeg", quality: 82 });
        index.push({ board: file, title: board.title, state: state.name, query: state.query, screen: shot });
    }
    await page.close();
}
await browser.close();
writeFileSync(join(screensDir, "index.json"), `${JSON.stringify(index, null, 2)}\n`);
if (failures.length) throw new Error(`Design render failed:\n${failures.join("\n")}`);
console.log(`Rendered ${index.length} design states from ${readdirSync(boardsDir).filter((name) => name.endsWith(".dc.html")).length} boards`);
