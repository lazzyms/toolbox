import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type ConvertRunWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __emit: (payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

type ConvertArgs = { runId: string; request: { paths: string[]; outputLocation: unknown } };

const converted = (inputPath: string, detail = "1 page."): Outcome => ({
    inputPath,
    outputPaths: [inputPath.replace(/\.[^.]+$/, ".pdf")],
    detail,
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
});

const PAGE_LIMIT = "Would make more than 5000 pages, over the limit for one PDF. Split it into smaller files and try again.";

const overLimit = (inputPath: string): Outcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind: "limitExceeded", details: { limit: "pageCount" }, message: PAGE_LIMIT, pointer: null },
    reason: null,
    warnings: [],
});

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as ConvertRunWindow;
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
                if (command === "get_output_settings") return { mode: "customFolder", folder: "/Users/test/Documents/Toolbox" };
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
    (window as unknown as ConvertRunWindow).__invocations.filter((call) => call.command === command).map((call) => call.args), command);

const convertCalls = async (page: Page) => (await invocations(page, "convert_to_pdf")) as ConvertArgs[];

const queueDialog = (page: Page, value: unknown) => page.evaluate((value) => { (window as unknown as ConvertRunWindow).__dialog.push(value); }, value);

const finish = (page: Page, outcomes: Outcome[]) => page.evaluate((outcomes) => (window as unknown as ConvertRunWindow).__finish(outcomes), outcomes);

const openConvert = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Convert to PDF");
    await expect.poll(() => page.evaluate(() => (window as unknown as ConvertRunWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
};

const dropFiles = async (page: Page, paths: string[]) => {
    await page.evaluate((paths) => (window as unknown as ConvertRunWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
};

const startRun = async (page: Page, count: number) => {
    await page.locator(".workspace-primary-action").click();
    await expect.poll(async () => (await convertCalls(page)).length).toBe(count);
    const { runId } = (await convertCalls(page))[count - 1];
    return (payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as ConvertRunWindow).__emit(payload), { runId, ...payload });
};

test("Convert to PDF shows the Settings destination, sends a folder changed for this run only, and never changes Settings", async ({ page }) => {
    const paths = ["/local/notes.txt", "/local/report.docx"];
    await openConvert(page);
    await dropFiles(page, paths);
    const destination = page.getByRole("region", { name: "Destination" });
    await expect(destination).toContainText("Save to~/Documents/Toolbox");
    const run = page.locator(".workspace-primary-action");
    const sent = async (count: number) => {
        await startRun(page, count);
        await finish(page, paths.map((path) => converted(path)));
        await expect(run).toBeEnabled();
        return (await convertCalls(page))[count - 1].request.outputLocation;
    };

    expect(await sent(1)).toEqual({ customFolder: "/Users/test/Documents/Toolbox" });

    await queueDialog(page, "/local/pdfs-this-run");
    await destination.getByRole("button", { name: "Change for this run" }).click();
    await expect(destination.getByText("This run only")).toBeVisible();
    await expect(destination).toContainText("/local/pdfs-this-run");
    expect(await sent(2)).toEqual({ customFolder: "/local/pdfs-this-run" });

    await destination.getByRole("button", { name: "Reset to default" }).click();
    await expect(destination.getByText("This run only")).toHaveCount(0);
    expect(await sent(3)).toEqual({ customFolder: "/Users/test/Documents/Toolbox" });
    expect(await invocations(page, "set_output_settings")).toEqual([]);
});

test("Convert to PDF shows each file's progress on its own row, and an over-limit file fails alone while the others finish", async ({ page }) => {
    const paths = ["/local/notes.txt", "/local/huge.csv", "/local/deck.pptx"];
    await openConvert(page);
    await dropFiles(page, paths);
    const emit = await startRun(page, 1);

    const status = page.getByRole("status").filter({ hasText: /^Converting/ });
    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
    await expect(status.locator("strong")).toHaveText("Converting 0 of 3");
    await expect(rows).toHaveText([/^notes\.txt.*Waiting/, /^huge\.csv.*Waiting/, /^deck\.pptx.*Waiting/]);

    await emit({ index: 0, phase: "started" });
    await expect(rows.nth(0)).toContainText("Converting");
    await emit({ index: 0, phase: "finished", outcome: converted(paths[0]) });
    await emit({ index: 1, phase: "started" });
    await emit({ index: 1, phase: "finished", outcome: overLimit(paths[1]) });
    await emit({ index: 2, phase: "started" });

    await expect(status.locator("strong")).toHaveText("Converting 2 of 3");
    await expect(rows.nth(0)).toContainText("1 page.");
    await expect(rows.nth(1)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(1)).toContainText(PAGE_LIMIT);
    await expect(rows.nth(2)).toContainText("Converting");
    expect(await page.locator(".live-run-bar > span").evaluateAll((spans) => spans.map((span) => span.getAttribute("data-tone")))).toEqual(["ok", "fail", "wait"]);

    await finish(page, [converted(paths[0]), overLimit(paths[1]), converted(paths[2], "24 slides.")]);
    const results = page.getByRole("list", { name: "Results" }).locator(":scope > li");
    await expect(results).toHaveCount(3);
    await expect(results.nth(1)).toContainText(PAGE_LIMIT);
    await expect(results.nth(2)).toContainText("24 slides.");
});

test("Convert to PDF runs by keyboard: add files, remove the selected one, start the run and open the result", async ({ page }) => {
    await openConvert(page);

    await queueDialog(page, ["/local/notes.txt", "/local/draft.md", "/local/budget.xlsx"]);
    await page.keyboard.press("Meta+O");
    const options = page.getByRole("listbox").getByRole("option");
    await expect(options).toHaveCount(3);
    expect(await invocations(page, "plugin:dialog|open")).toHaveLength(1);

    const draft = options.filter({ hasText: "draft.md" });
    await draft.locator(".file-selection-item").click();
    await expect(draft).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Delete");
    await expect(options).toHaveCount(2);
    await expect(options.filter({ hasText: "draft.md" })).toHaveCount(0);

    await options.filter({ hasText: "budget.xlsx" }).locator(".file-selection-item").click();
    await page.keyboard.press("Backspace");
    await expect(options).toHaveText([/notes\.txt/]);

    await page.keyboard.press("Meta+Enter");
    await expect.poll(async () => (await convertCalls(page)).length).toBe(1);
    expect((await convertCalls(page))[0].request.paths).toEqual(["/local/notes.txt"]);

    await finish(page, [converted("/local/notes.txt")]);
    const open = page.getByRole("list", { name: "Results" }).getByRole("button", { name: "Open notes.pdf" });
    await open.focus();
    await page.keyboard.press("Enter");
    await expect.poll(async () => invocations(page, "open_output_path")).toEqual([{ path: "/local/notes.pdf" }]);
});
