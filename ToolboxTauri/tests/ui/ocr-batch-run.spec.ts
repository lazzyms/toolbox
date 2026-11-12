import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type OcrWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __emit: (payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

const searchable = (inputPath: string, detail = "1 page recognized."): Outcome => ({
    inputPath,
    outputPaths: [inputPath.replace(/\.[^.]+$/, "-ocr.pdf")],
    detail,
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
});

const overLimit = (inputPath: string, limit: string, message: string): Outcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind: "limitExceeded", details: { limit }, message, pointer: null },
    reason: null,
    warnings: [],
});

const TIME_LIMIT = "Stopped at page 212 of 480: it reached the time limit for one file. Nothing was saved. Split it into smaller PDFs, then try again.";
const PIXEL_LIMIT = "poster.png is over the 40 million pixel limit for one OCR page.";

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as OcrWindow;
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
                if (command.startsWith("plugin:") || command === "shell_ready" || command === "open_output_path" || command === "reveal_output_path") return null;
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
    (window as unknown as OcrWindow).__invocations.filter((call) => call.command === command).map((call) => call.args), command);

const ocrRequests = async (page: Page) =>
    (await invocations(page, "ocr_pdf")) as Array<{ runId: string; request: { paths: string[]; outputLocation: unknown } }>;

const openOcr = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, "OCR PDF");
    await expect.poll(() => page.evaluate(() => (window as unknown as OcrWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as OcrWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
};

const finish = (page: Page, outcomes: Outcome[]) => page.evaluate((outcomes) => (window as unknown as OcrWindow).__finish(outcomes), outcomes);

const startRun = async (page: Page, count: number) => {
    await page.locator(".workspace-primary-action").click();
    await expect.poll(async () => (await ocrRequests(page)).length).toBe(count);
    const { runId } = (await ocrRequests(page))[count - 1];
    return (payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as OcrWindow).__emit(payload), { runId, ...payload });
};

test("OCR sends the default destination, then a folder changed for this run only, and never changes Settings", async ({ page }) => {
    const paths = ["/local/scan.pdf", "/local/receipt.jpg"];
    await openOcr(page, paths);
    const destination = page.getByRole("region", { name: "Destination" });
    const run = page.locator(".workspace-primary-action");
    const sent = async (count: number) => {
        await startRun(page, count);
        await finish(page, paths.map((path) => searchable(path)));
        await expect(run).toBeEnabled();
        return (await ocrRequests(page))[count - 1].request.outputLocation;
    };

    expect(await sent(1)).toBe("alongsideInput");

    await page.evaluate(() => { (window as unknown as OcrWindow).__dialog.push("/local/ocr-this-run"); });
    await destination.getByRole("button", { name: "Change for this run" }).click();
    await expect(destination.getByText("This run only")).toBeVisible();
    expect(await sent(2)).toEqual({ customFolder: "/local/ocr-this-run" });

    await destination.getByRole("button", { name: "Reset to default" }).click();
    await expect(destination.getByText("This run only")).toHaveCount(0);
    expect(await sent(3)).toBe("alongsideInput");
    expect(await invocations(page, "set_output_settings")).toEqual([]);
});

test("OCR counts pages as the board shows: the file being read, its page, and pages done across the run", async ({ page }) => {
    const paths = ["/local/annual-report.pdf", "/local/board-minutes-q3.pdf", "/local/receipt.jpg"];
    await openOcr(page, paths);
    const emit = await startRun(page, 1);

    const status = page.getByRole("status").filter({ hasText: /^Recognizing file/ });
    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
    const bar = page.getByRole("progressbar", { name: "Pages recognized" });
    await expect(status.locator("strong")).toHaveText("Recognizing file 1 of 3");
    await expect(bar).toHaveAttribute("aria-valuetext", "0 of 0 pages done");
    await expect(rows).toHaveText([/^annual-report\.pdfPage count unknownWaiting/, /^board-minutes-q3\.pdf.*Waiting/, /^receipt\.jpg.*Waiting/]);
    await expect(page.getByText("Each file is recognized on its own, so one that fails doesn't stop the others.")).toBeVisible();

    await emit({ index: 0, phase: "started" });
    await emit({ index: 0, phase: "progress", completed: 0, total: 31 });
    await emit({ index: 0, phase: "progress", completed: 31, total: 31 });
    await emit({ index: 0, phase: "finished", outcome: searchable(paths[0], "31 pages recognized.") });
    await emit({ index: 1, phase: "started" });
    await emit({ index: 1, phase: "progress", completed: 0, total: 9 });
    await emit({ index: 1, phase: "progress", completed: 3, total: 9 });

    await expect(status.locator("strong")).toHaveText("Recognizing file 2 of 3");
    await expect(status).toContainText("board-minutes-q3.pdf, page 4 of 9");
    await expect(bar).toHaveAttribute("aria-valuetext", "34 of 40 pages done");
    await expect(page.getByText("34 of 40 pages done. Long scans take a while; the bar counts pages, not files.")).toBeVisible();
    await expect(rows.nth(0)).toContainText("Recognized");
    await expect(rows.nth(1)).toContainText("Page 4 of 9");
    await expect(rows.nth(1).locator(".result-row-state")).toHaveText("Recognizing");
    const minibar = rows.nth(1).getByRole("progressbar", { name: "board-minutes-q3.pdf" });
    await expect(minibar).toHaveAttribute("aria-valuetext", "Page 4 of 9, 3 done");
    await expect(minibar).toHaveAttribute("aria-valuemax", "9");
    // The fill follows the share of pages done, not the raw count.
    expect(await bar.locator("[data-slot=progress-indicator]").getAttribute("style")).toContain("translateX(-15%)");
    await expect(rows.nth(2)).toHaveText(/^receipt\.jpgPage count unknownWaiting/);

    // A page report for another run, or one that counts past its total, changes nothing.
    await page.evaluate(() => (window as unknown as OcrWindow).__emit({ runId: "another-run", index: 1, phase: "progress", completed: 8, total: 9 }));
    await emit({ index: 1, phase: "progress", completed: 10, total: 9 });
    await expect(minibar).toHaveAttribute("aria-valuetext", "Page 4 of 9, 3 done");

    await finish(page, [searchable(paths[0], "31 pages recognized."), searchable(paths[1], "9 pages recognized."), searchable(paths[2])]);
    await expect(page.getByRole("list", { name: "Results" }).locator(":scope > li")).toHaveCount(3);
    await expect(bar).toHaveCount(0);
});

test("OCR shows over-limit files failed on their own rows while the others finish", async ({ page }) => {
    const paths = ["/local/lease.pdf", "/local/archive.pdf", "/local/poster.png", "/local/receipt.jpg"];
    await openOcr(page, paths);
    const emit = await startRun(page, 1);
    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");

    await emit({ index: 1, phase: "started" });
    await emit({ index: 1, phase: "progress", completed: 211, total: 480 });
    await emit({ index: 1, phase: "finished", outcome: overLimit(paths[1], "time", TIME_LIMIT) });
    await emit({ index: 0, phase: "started" });
    await emit({ index: 0, phase: "progress", completed: 1, total: 2 });
    await expect(rows.nth(1)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(1)).toContainText(TIME_LIMIT);
    await expect(rows.nth(0)).toContainText("Page 2 of 2");
    // A file that stopped early adds only the pages it reached.
    await expect(page.getByRole("progressbar", { name: "Pages recognized" })).toHaveAttribute("aria-valuetext", "212 of 213 pages done");

    await finish(page, [
        searchable(paths[0], "2 pages recognized."),
        overLimit(paths[1], "time", TIME_LIMIT),
        overLimit(paths[2], "decompression", PIXEL_LIMIT),
        searchable(paths[3]),
    ]);
    const results = page.getByRole("list", { name: "Results" }).locator(":scope > li");
    await expect(results).toHaveCount(4);
    for (const index of [0, 3]) await expect(results.nth(index)).toHaveAttribute("data-tone", "ok");
    for (const [index, message] of [[1, TIME_LIMIT], [2, PIXEL_LIMIT]] as const) {
        await expect(results.nth(index)).toHaveAttribute("data-tone", "fail");
        await expect(results.nth(index)).toContainText(message);
        await expect(results.nth(index).locator(".result-row-state .sr-only")).toHaveText("Failed, over the limit");
    }
    // Only a run that ran out of time may pass on another try.
    await expect(results.nth(1).getByRole("button", { name: "Retry archive.pdf" })).toBeVisible();
    await expect(results.nth(2).getByRole("button", { name: /^Retry/ })).toHaveCount(0);
});

test("the run shortcut starts an OCR run", async ({ page }) => {
    await openOcr(page, ["/local/scan.pdf"]);
    await page.locator("body").press("Meta+Enter");
    await expect.poll(async () => (await ocrRequests(page)).map(({ request }) => request.paths)).toEqual([["/local/scan.pdf"]]);
    await expect(page.getByRole("status").filter({ hasText: /^Recognizing file/ }).locator("strong")).toHaveText("Recognizing file 1 of 1");
});
