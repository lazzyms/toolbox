import { expect, test, type Page } from "@playwright/test";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type LiveWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __drop: (paths: string[]) => void;
    __emit: (payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

const succeeded = (inputPath: string): Outcome => ({
    inputPath,
    outputPaths: [inputPath.replace(/\.pdf$/, "-protected.pdf")],
    detail: "",
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
});

const failed = (inputPath: string): Outcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind: "processing", message: "This PDF is damaged.", pointer: null },
    reason: null,
    warnings: [],
});

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as LiveWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        const dispatch = (name: string, payload: unknown) => {
            for (const [id, listener] of listeners) {
                if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
            }
        };
        win.__drop = (paths) => dispatch("tauri://drag-drop", { paths, position: { x: 10, y: 10 } });
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
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "cancel_run") return true;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "open_output_path" || command === "reveal_output_path") return null;
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

const startLocking = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as unknown as LiveWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "tauri://drag-drop"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as LiveWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
    for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await field.fill("test-password");
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => page.evaluate(() => (window as unknown as LiveWindow).__invocations.some(({ command }) => command === "protect_files"))).toBe(true);
    const runId = await page.evaluate(() => {
        const invocation = (window as unknown as LiveWindow).__invocations.find(({ command }) => command === "protect_files");
        return (invocation?.args as { runId: string }).runId;
    });
    expect(runId).toMatch(/\S+/);
    const emit = (payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as LiveWindow).__emit(payload), { runId, ...payload });
    return { runId, emit };
};

test("rows stream in input order, the count follows finished files, and early rows open mid-run", async ({ page }) => {
    const paths = ["/local/lease.pdf", "/local/survey.pdf", "/local/minutes.pdf"];
    const { emit } = await startLocking(page, paths);

    const status = page.getByRole("status").filter({ hasText: /^Locking/ });
    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
    await expect(status.locator("strong")).toHaveText("Locking 0 of 3");
    await expect(rows).toHaveText([/^lease\.pdf.*Waiting/, /^survey\.pdf.*Waiting/, /^minutes\.pdf.*Waiting/]);
    await expect(page.locator(".live-run-bar > span")).toHaveCount(3);

    await emit({ index: 0, phase: "started" });
    await emit({ index: 1, phase: "started" });
    await emit({ index: 1, phase: "finished", outcome: succeeded(paths[1]) });

    await expect(status.locator("strong")).toHaveText("Locking 1 of 3");
    await expect(status).toContainText("1 locked so far. Finished files can be opened now.");
    await expect(rows).toHaveText([/^lease\.pdf.*Locking/, /^survey\.pdf.*Locked/, /^minutes\.pdf.*Waiting/]);
    expect(await page.locator(".live-run-bar > span").evaluateAll((spans) => spans.map((span) => span.getAttribute("data-tone")))).toEqual(["wait", "ok", "wait"]);

    await rows.nth(1).getByRole("button", { name: "Open survey-protected.pdf" }).click();
    await rows.nth(1).getByRole("button", { name: "Show in Finder: survey-protected.pdf" }).click();
    const actions = await page.evaluate(() => (window as unknown as LiveWindow).__invocations
        .filter(({ command }) => command === "open_output_path" || command === "reveal_output_path")
        .map(({ command, args }) => [command, (args as { path: string }).path]));
    expect(actions).toEqual([["open_output_path", "/local/survey-protected.pdf"], ["reveal_output_path", "/local/survey-protected.pdf"]]);

    await page.evaluate(() => (window as unknown as LiveWindow).__emit({ runId: "another-run", index: 2, phase: "finished", outcome: {
        inputPath: "/local/minutes.pdf", outputPaths: ["/elsewhere.pdf"], detail: "", state: "succeeded", failure: null, reason: null, warnings: [],
    } }));
    await emit({ index: 9, phase: "started" });
    await emit({ index: 0, phase: "finished", outcome: failed(paths[0]) });
    await expect(status.locator("strong")).toHaveText("Locking 2 of 3");
    await expect(status).toContainText("1 locked, 1 failed so far.");
    await expect(rows.nth(0)).toContainText("This PDF is damaged.");
    await expect(rows.nth(2)).toContainText("Waiting");
    await expect(rows.nth(0).getByRole("button", { name: /Retry/ })).toHaveCount(0);

    await page.evaluate((outcomes) => (window as unknown as LiveWindow).__finish(outcomes), [failed(paths[0]), succeeded(paths[1]), succeeded(paths[2])]);

    await expect(page.getByRole("status").filter({ hasText: /^Done\./ }).locator("strong")).toHaveText("Done. 2 locked, 1 failed");
    await expect(page.getByRole("list", { name: "Files and results" })).toHaveCount(0);
    await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveText([/^lease\.pdf.*Failed/, /^survey\.pdf.*Locked/, /^minutes\.pdf.*Locked/]);
    await expect(page.locator(".live-run-bar")).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as LiveWindow).__invocations.some(({ command }) => command === "plugin:event|unlisten"))).toBe(true);
});

test("over 60 files the bar shows proportions with the board note", async ({ page }) => {
    const paths = Array.from({ length: 61 }, (_, index) => `/local/scan-${index + 1}.pdf`);
    const { emit } = await startLocking(page, paths);

    await emit({ index: 0, phase: "finished", outcome: succeeded(paths[0]) });
    await emit({ index: 1, phase: "finished", outcome: failed(paths[1]) });

    await expect(page.getByText("Over 60 files, the bar shows proportions. Rows for finished files can be opened now.")).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: /^Locking/ })).toHaveText(/Locking 2 of 61\s*1 locked, 1 failed so far\.$/);
    const grow = await page.locator('.live-run-bar[data-mode="proportional"] > span').evaluateAll((spans) =>
        spans.map((span) => [span.getAttribute("data-tone"), (span as HTMLElement).style.flexGrow]));
    expect(grow).toEqual([["ok", "1"], ["skip", "0"], ["fail", "1"], ["wait", "59"]]);
    await expect(page.getByRole("list", { name: "Files and results" }).getByRole("listitem")).toHaveCount(61);
});

test("cancel asks first, sends the run id, and ends with cancelled rows and counts", async ({ page }) => {
    const paths = ["/local/lease.pdf", "/local/survey.pdf", "/local/minutes.pdf", "/local/deed.pdf", "/local/notes.pdf"];
    const { runId, emit } = await startLocking(page, paths);
    await emit({ index: 0, phase: "finished", outcome: succeeded(paths[0]) });
    await emit({ index: 1, phase: "finished", outcome: succeeded(paths[1]) });
    await emit({ index: 2, phase: "started" });

    const controls = page.getByRole("group", { name: "Running batch" });
    await expect(controls).toContainText("Mode, passwords and files are locked until the batch finishes.");
    await expect(controls).toContainText("2 of 5 done");
    const cancelRequests = () => page.evaluate(() => (window as unknown as LiveWindow).__invocations.filter(({ command }) => command === "cancel_run").map(({ args }) => args));

    const dialog = page.getByRole("alertdialog");
    await page.keyboard.press("Escape");
    await expect(dialog.getByRole("heading")).toHaveText("Cancel this batch?");
    await expect(dialog).toContainText("The 2 finished files keep their outputs. The file in progress is stopped and leaves nothing half-written. The other 2 files won't be processed.");
    await dialog.getByRole("button", { name: "Keep running" }).click();
    await expect(dialog).toHaveCount(0);
    expect(await cancelRequests()).toEqual([]);

    await controls.getByRole("button", { name: "Cancel batch" }).click();
    await dialog.getByRole("button", { name: "Cancel batch" }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(cancelRequests).toEqual([{ runId }]);
    await expect(controls.getByRole("button", { name: "Cancel batch" })).toBeDisabled();

    const cancelledWhileRunning: Outcome = { ...failed(paths[2]), failure: { kind: "cancelled", message: "Cancelled while it was running. Nothing was saved.", pointer: null } };
    const notStarted = (inputPath: string): Outcome => ({ ...failed(inputPath), state: "skipped", failure: null, reason: { code: "cancelled", message: "Cancelled before it started" } });
    await page.evaluate((outcomes) => (window as unknown as LiveWindow).__finish(outcomes),
        [succeeded(paths[0]), succeeded(paths[1]), cancelledWhileRunning, notStarted(paths[3]), notStarted(paths[4])]);

    const summary = page.getByRole("status").filter({ hasText: /^Cancelled\./ });
    await expect(summary.locator("strong")).toHaveText("Cancelled. 2 locked, 2 skipped as cancelled, 1 failed");
    await expect(summary).toContainText("Finished files were kept. Nothing was half-written.");
    await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveText([
        /^lease\.pdf.*Locked/,
        /^survey\.pdf.*Locked/,
        /^minutes\.pdf.*Cancelled while it was running\. Nothing was saved\..*Failed/,
        /^deed\.pdf.*Cancelled before it started.*Skipped/,
        /^notes\.pdf.*Cancelled before it started.*Skipped/,
    ]);
    await expect(controls).toHaveCount(0);
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
});
