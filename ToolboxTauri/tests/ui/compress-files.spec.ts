import { expect, test, type Locator, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = {
    inputPath: string;
    outputPaths: string[];
    detail: string;
    state: string;
    failure: unknown;
    reason: unknown;
    warnings: unknown[];
    originalBytes?: number;
    outputBytes?: number;
};

type CompressWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __emit: (payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

type RunArgs = { runId: string; request: { paths: string[]; level: string; stripExif: boolean; outputLocation: unknown } };

const outcome = (inputPath: string, fields: Partial<Outcome>): Outcome => ({
    inputPath, outputPaths: [], detail: "", state: "succeeded", failure: null, reason: null, warnings: [], ...fields,
});
const compressed = (inputPath: string, originalBytes: number, outputBytes: number, output = inputPath.replace(/(\.[^.]+)$/, "-compressed$1")) =>
    outcome(inputPath, { outputPaths: [output], originalBytes, outputBytes });

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as CompressWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        win.__dialog = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        const dispatch = (name: string, payload: unknown) => {
            for (const [id, listener] of listeners) {
                if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
            }
        };
        win.__drop = (paths) => dispatch("toolbox://shell-event", { kind: "dropped-files", paths });
        win.__emit = (payload) => dispatch("batch-progress", payload);
        win.__finish = (outcomes) => finish?.(outcomes);
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "plugin:event|listen") {
                    const { event, handler } = args as { event: string; handler: number };
                    const id = nextId++;
                    listeners.set(id, { event, handler });
                    return id;
                }
                if (command === "plugin:event|unlisten") {
                    listeners.delete((args as { eventId: number }).eventId);
                    return null;
                }
                if (command === "plugin:dialog|open") return win.__dialog.shift() ?? null;
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "cancel_run") return true;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command === "open_output_path" || command === "reveal_output_path" || command === "reveal_output_paths") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                return new Promise((resolve) => { finish = resolve; });
            },
            transformCallback: (callback: (event: unknown) => void) => {
                const id = nextId++;
                callbacks.set(id, callback);
                return id;
            },
            unregisterCallback: (id: number) => callbacks.delete(id),
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    });
});

const invocations = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as CompressWindow).__invocations.filter((call) => call.command === command).map((call) => call.args), command);
const queueDialog = (page: Page, result: unknown) => page.evaluate((result) => { (window as unknown as CompressWindow).__dialog.push(result); }, result);
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as CompressWindow).__emit(payload), payload);
const finish = (page: Page, outcomes: Outcome[]) => page.evaluate((outcomes) => (window as unknown as CompressWindow).__finish(outcomes), outcomes);
const strength = (page: Page) => page.getByRole("radiogroup", { name: "Strength" });
const stripExif = (page: Page) => page.getByRole("switch", { name: "Strip EXIF" });

const openCompress = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Compress");
    await expect(strength(page)).toBeVisible();
    await expect.poll(() => page.evaluate(() => (window as unknown as CompressWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
};

const addFiles = async (page: Page, paths: string[]) => {
    await page.evaluate((paths) => (window as unknown as CompressWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
};

const startRun = async (page: Page, paths: string[]) => {
    await openCompress(page);
    await addFiles(page, paths);
    await page.getByRole("button", { name: `Compress ${paths.length} files`, exact: true }).click();
    await expect.poll(async () => (await invocations(page, "compress_files")).length).toBe(1);
    const [{ runId }] = await invocations(page, "compress_files") as RunArgs[];
    return { runId };
};

// Presses Tab until the target holds focus, so every step is reached the way a keyboard user reaches it.
const tabTo = async (page: Page, target: Locator) => {
    for (let step = 0; step < 80; step += 1) {
        if (await target.evaluate((element) => element === document.activeElement)) return;
        await page.keyboard.press("Tab");
    }
    throw new Error("Tab never reached the target.");
};

test("a per-run Save to folder goes in the request, never touches the Settings default, and resets", async ({ page }) => {
    const paths = ["/local/report.pdf", "/local/IMG_4012.jpg"];
    await openCompress(page);
    await addFiles(page, paths);
    const destination = page.getByTestId("run-destination-path");

    await queueDialog(page, "/Volumes/Share/Compressed");
    await page.getByRole("button", { name: "Change for this run" }).click();
    await expect(page.getByText("This run only", { exact: true })).toBeVisible();
    await expect(destination).toContainText("/Volumes/Share/Compressed");
    await page.getByRole("button", { name: "Compress 2 files", exact: true }).click();
    await expect.poll(async () => (await invocations(page, "compress_files")).length).toBe(1);
    await finish(page, paths.map((path) => compressed(path, 2_000_000, 1_000_000, path.replace("/local", "/Volumes/Share/Compressed"))));
    await expect(page.getByRole("status").filter({ hasText: /^Done\./ })).toBeVisible();

    await page.getByRole("button", { name: "Reset to default" }).click();
    await expect(page.getByText("This run only", { exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Compress 2 files", exact: true }).click();
    await expect.poll(async () => (await invocations(page, "compress_files")).length).toBe(2);

    const requests = (await invocations(page, "compress_files") as RunArgs[]).map(({ request }) => request);
    expect(requests).toEqual([
        { paths, level: "medium", stripExif: false, outputLocation: { customFolder: "/Volumes/Share/Compressed" } },
        { paths, level: "medium", stripExif: false, outputLocation: "alongsideInput" },
    ]);
    expect(await invocations(page, "check_output_folder")).toEqual([{ folder: "/Volumes/Share/Compressed" }]);
    expect(await invocations(page, "set_output_settings")).toEqual([]);
});

test("live rows stream sizes and the running saving, finished copies open mid-run, and the options stay locked", async ({ page }) => {
    const paths = ["/local/site-survey-scan.pdf", "/local/IMG_4012.jpg", "/local/floor-plan.png"];
    const { runId } = await startRun(page, paths);

    const status = page.getByRole("status").filter({ hasText: /^Compressing/ });
    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
    await expect(status.locator("strong")).toHaveText("Compressing 0 of 3");
    await expect(rows).toHaveText([/^site-survey-scan\.pdf.*Waiting$/, /^IMG_4012\.jpg.*Waiting$/, /^floor-plan\.png.*Waiting$/]);
    await expect(page.getByRole("group", { name: "Running batch" })).toContainText("Strength, Strip EXIF, Save to and the file list are locked until the batch finishes.");
    for (const radio of await strength(page).getByRole("radio").all()) await expect(radio).toBeDisabled();
    await expect(stripExif(page)).toBeDisabled();
    await expect(page.getByRole("button", { name: "Choose files to process" })).toBeDisabled();

    await emit(page, { runId, index: 0, phase: "started" });
    await emit(page, { runId, index: 1, phase: "started" });
    await expect(rows).toHaveText([/^site-survey-scan\.pdf.*Compressing$/, /^IMG_4012\.jpg.*Compressing$/, /^floor-plan\.png.*Waiting$/]);

    await emit(page, { runId, index: 0, phase: "finished", outcome: compressed(paths[0], 14_600_000, 3_800_000) });
    await expect(status.locator("strong")).toHaveText("Compressing 1 of 3");
    await expect(status).toContainText("10.8 MB saved so far");
    await expect(page.locator(".live-run-note")).toHaveText("Finished copies can be opened now. The batch total appears when every file is done.");
    await expect(rows.nth(0)).toContainText("14.6 MB → 3.8 MB, 74% smaller");
    await expect(rows.nth(0)).toContainText("Compressed");
    expect(await page.locator(".live-run-bar > span").evaluateAll((spans) => spans.map((span) => span.getAttribute("data-tone")))).toEqual(["ok", "wait", "wait"]);

    await rows.nth(0).getByRole("button", { name: "Open site-survey-scan-compressed.pdf" }).click();
    await rows.nth(0).getByRole("button", { name: "Show in Finder: site-survey-scan-compressed.pdf" }).click();
    expect(await invocations(page, "open_output_path")).toEqual([{ path: "/local/site-survey-scan-compressed.pdf" }]);
    expect(await invocations(page, "reveal_output_path")).toEqual([{ path: "/local/site-survey-scan-compressed.pdf" }]);

    await emit(page, { runId, index: 1, phase: "finished", outcome: compressed(paths[1], 4_200_000, 1_400_000) });
    await expect(status.locator("strong")).toHaveText("Compressing 2 of 3");
    await expect(status).toContainText("13.6 MB saved so far");

    await finish(page, [
        compressed(paths[0], 14_600_000, 3_800_000),
        compressed(paths[1], 4_200_000, 1_400_000),
        compressed(paths[2], 900_000, 600_000),
    ]);
    const done = page.getByRole("status").filter({ hasText: /^Done\./ });
    await expect(done.locator("strong")).toHaveText("Done. 3 compressed");
    await expect(done).toContainText("19.7 MB → 5.8 MB across 3 compressed files");
    await expect(strength(page).getByRole("radio", { name: /^High/ })).toBeEnabled();
});

test("Show all in Finder reveals every compressed copy in one call and is absent when nothing was written", async ({ page }) => {
    const paths = ["/local/report.pdf", "/local/IMG_4012.jpg", "/local/minutes.pdf", "/local/menu.pdf"];
    await startRun(page, paths);
    await finish(page, [
        compressed(paths[0], 2_000_000, 900_000),
        compressed(paths[1], 3_000_000, 1_000_000, "/Volumes/Share/IMG_4012-compressed.jpg"),
        outcome(paths[2], { state: "failed", failure: { kind: "invalidInput", message: "Not a readable PDF. The file may be damaged or incomplete.", pointer: null } }),
        outcome(paths[3], { state: "skipped", reason: { code: "noSavings", message: "No savings: already well compressed", pointer: null }, originalBytes: 640_000, outputBytes: 652_000 }),
    ]);
    const showAll = page.getByRole("button", { name: "Show all in Finder" });
    await expect(showAll).toBeVisible();
    await showAll.click();
    expect(await invocations(page, "reveal_output_paths")).toEqual([{ paths: ["/local/report-compressed.pdf", "/Volumes/Share/IMG_4012-compressed.jpg"] }]);
});

test("Show all is absent when no file got a compressed copy", async ({ page }) => {
    const paths = ["/local/broken.pdf", "/local/menu.pdf"];
    await startRun(page, paths);
    await finish(page, [
        outcome(paths[0], { state: "failed", failure: { kind: "invalidInput", message: "Not a readable PDF.", pointer: null } }),
        outcome(paths[1], { state: "skipped", reason: { code: "noSavings", message: "No savings: already well compressed", pointer: null }, originalBytes: 640_000, outputBytes: 652_000 }),
    ]);
    await expect(page.getByRole("status").filter({ hasText: /^Done\./ })).toBeVisible();
    await expect(page.getByRole("button", { name: /^Show all in/ })).toHaveCount(0);
});

test("an over-limit PDF fails alone without Retry and points to Split PDF", async ({ page }) => {
    const paths = ["/local/drone-survey-full.pdf", "/local/report.pdf"];
    await startRun(page, paths);
    const message = "Over the size limit: 5,400 pages, and the limit is 5,000. Split it, then compress the parts.";
    await finish(page, [
        outcome(paths[0], { state: "failed", failure: { kind: "limitExceeded", details: { limit: "pageCount" }, message, pointer: { kind: "tool", toolId: "pdf-split" } } }),
        compressed(paths[1], 2_000_000, 900_000),
    ]);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows.nth(0)).toContainText(message);
    await expect(rows.nth(0)).toContainText("Failed");
    await expect(rows.nth(0).getByRole("button", { name: /Retry/ })).toHaveCount(0);
    await expect(rows.nth(0).getByRole("button", { name: "Open Split PDF" })).toBeVisible();
    await expect(rows.nth(1)).toContainText("2 MB → 900 KB, 55% smaller");
    await rows.nth(0).getByRole("button", { name: "Open Split PDF" }).click();
    await expect(page.getByRole("button", { name: "Split PDF", pressed: true })).toBeVisible();
    await expect(page.getByRole("radiogroup", { name: "Split by" })).toBeVisible();
});

test("a PDF over 1 GB is compressed with no size refusal while a slow PDF fails alone and points to Split PDF", async ({ page }) => {
    const paths = ["/local/Photo.pdf", "/local/scan-archive.pdf", "/local/report.pdf"];
    await startRun(page, paths);
    const [{ request }] = await invocations(page, "compress_files") as RunArgs[];
    expect(request.paths).toEqual(paths);
    const message = "Took longer than 25 minutes and was stopped. Nothing was saved. Split it, then compress the parts.";
    await finish(page, [
        compressed(paths[0], 1_570_000_000, 412_000_000),
        outcome(paths[1], { state: "failed", failure: { kind: "limitExceeded", details: { limit: "time" }, message, pointer: { kind: "tool", toolId: "pdf-split" } } }),
        compressed(paths[2], 2_000_000, 900_000),
    ]);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows.nth(0)).toContainText("1.6 GB → 412 MB, 74% smaller");
    await expect(rows.nth(1)).toContainText(message);
    await expect(rows.nth(1)).toContainText("Failed");
    await expect(rows.nth(1).getByRole("button", { name: "Open Split PDF" })).toBeVisible();
    await expect(rows.nth(2)).toContainText("2 MB → 900 KB, 55% smaller");
    await expect(page.getByText(/Larger than/)).toHaveCount(0);
});

test("the whole Compress flow runs by keyboard: add files, strength, Strip EXIF, run and results", async ({ page }) => {
    const paths = ["/local/report.pdf", "/local/IMG_4012.jpg"];
    await openCompress(page);
    await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveAttribute("aria-keyshortcuts", "Meta+O");

    await queueDialog(page, paths);
    await page.locator("body").press("Meta+O");
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(2);

    await tabTo(page, strength(page).getByRole("radio", { checked: true }));
    await expect(strength(page).getByRole("radio", { name: /^Medium/ })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(strength(page).getByRole("radio", { name: /^High/ })).toBeFocused();
    await page.keyboard.press("Space");
    await expect(strength(page).getByRole("radio", { name: /^High/ })).toBeChecked();
    await expect(page.getByText("What High does", { exact: true })).toBeVisible();

    await tabTo(page, stripExif(page));
    await page.keyboard.press("Space");
    await expect(stripExif(page)).toBeChecked();

    const run = page.getByRole("button", { name: "Compress 2 files", exact: true });
    await expect(run).toHaveAttribute("aria-keyshortcuts", "Meta+Enter");
    await page.keyboard.press("Meta+Enter");
    await expect.poll(async () => (await invocations(page, "compress_files")).length).toBe(1);
    const [{ request }] = await invocations(page, "compress_files") as RunArgs[];
    expect(request).toEqual({ paths, level: "high", stripExif: true, outputLocation: "alongsideInput" });

    await finish(page, paths.map((path) => compressed(path, 2_000_000, 800_000)));
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows).toHaveCount(2);
    await tabTo(page, page.getByRole("button", { name: "Show all in Finder" }));
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await invocations(page, "reveal_output_paths")).length).toBe(1);
    await tabTo(page, rows.nth(0).getByRole("button", { name: "Open report-compressed.pdf" }));
    await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
    await expect(rows.nth(0).getByRole("button", { name: "Show in Finder: report-compressed.pdf" })).toBeFocused();
    await page.keyboard.press("Enter");
    expect(await invocations(page, "open_output_path")).toEqual([{ path: "/local/report-compressed.pdf" }]);
    expect(await invocations(page, "reveal_output_path")).toEqual([{ path: "/local/report-compressed.pdf" }]);
});

test("strength, Strip EXIF, sizes, savings and Skipped reasons have spoken labels", async ({ page }) => {
    const paths = ["/local/report.pdf", "/local/menu.pdf", "/local/IMG_4020.heic"];
    await openCompress(page);
    const group = strength(page);
    await expect(group).toHaveAccessibleName("Strength");
    await expect(group.getByRole("radio")).toHaveCount(3);
    await expect(group.getByRole("radio", { name: "Low Least saving, detail stays" })).toHaveAttribute("aria-checked", "false");
    await expect(group.getByRole("radio", { name: "Medium Good for sharing and email" })).toHaveAttribute("aria-checked", "true");
    await expect(group.getByRole("radio", { name: "High Smallest, photos look softer" })).toHaveAttribute("aria-checked", "false");
    await expect(stripExif(page)).toHaveAccessibleName("Strip EXIF");
    await expect(stripExif(page)).toHaveAccessibleDescription("Off: camera, date and GPS details stay in the copies.");

    await addFiles(page, paths);
    await page.getByRole("button", { name: "Compress 3 files", exact: true }).click();
    await expect.poll(async () => (await invocations(page, "compress_files")).length).toBe(1);
    await finish(page, [
        compressed(paths[0], 14_600_000, 3_800_000),
        outcome(paths[1], { state: "skipped", reason: { code: "noSavings", message: "No savings: already well compressed", pointer: null }, originalBytes: 640_000, outputBytes: 652_000 }),
        outcome(paths[2], { state: "skipped", reason: { code: "unsupportedFormat", message: "HEIC isn't compressed. Convert it to JPEG, then compress the copy.", pointer: { kind: "tool", toolId: "heic-convert" } } }),
    ]);

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    const spoken = async (row: Locator) => (await row.evaluate((element) => {
        const hidden = (node: Element) => node.closest('[aria-hidden="true"]') !== null;
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
        const parts: string[] = [];
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            if (node.parentElement && !hidden(node.parentElement) && node.textContent?.trim()) parts.push(node.textContent.trim());
        }
        return parts.join(" | ");
    }));
    expect(await spoken(rows.nth(0))).toContain("report.pdf | 14.6 MB → 3.8 MB, 74% smaller | report-compressed.pdf | Compressed");
    expect(await spoken(rows.nth(1))).toBe("menu.pdf | 640 KB → 652 KB, no savings | No savings: already well compressed | Skipped");
    expect(await spoken(rows.nth(2))).toContain("IMG_4020.heic | HEIC isn't compressed. Convert it to JPEG, then compress the copy. | Skipped");
    await expect(rows.nth(1).getByRole("button")).toHaveCount(0);
    await expect(rows.nth(0).getByRole("button", { name: "Open report-compressed.pdf" })).toBeVisible();
    const summary = page.getByRole("status").filter({ hasText: /^Done\./ });
    await expect(summary).toContainText("Saved 10.8 MB, 74% smaller");
});
