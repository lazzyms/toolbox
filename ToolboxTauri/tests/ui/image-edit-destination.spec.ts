import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type EditWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
    __emit: (event: string, payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

const blank = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E";

const edited = (inputPath: string, folder = inputPath.replace(/\/[^/]+$/, "")): Outcome => ({
    inputPath,
    outputPaths: [`${folder}/${inputPath.split("/").pop()!.replace(/(\.\w+)$/, "-cropped$1")}`],
    detail: "Image saved",
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
});

test.beforeEach(async ({ page }) => {
    await page.addInitScript((blank) => {
        const win = window as unknown as EditWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__emit = (name, payload) => {
            for (const [id, listener] of listeners) {
                if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
            }
        };
        win.__finish = (outcomes) => finish?.(outcomes);
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
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
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "get_quick_actions") return [];
                if (command === "cancel_run") return true;
                if (command === "plugin:app|version") return "9.4.2";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: blank };
                if (command === "inspect_image_edit_preview") return { width: 384, height: 480, dataUrl: blank };
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command === "export_image_edit_stack") return new Promise((resolve) => { finish = resolve; });
                return null;
            },
            transformCallback: (callback: (event: unknown) => void) => {
                const id = nextId++;
                callbacks.set(id, callback);
                return id;
            },
            unregisterCallback: (id: number) => callbacks.delete(id),
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, blank);
});

const invocations = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as EditWindow).__invocations.filter((invocation) => invocation.command === command), command);

const queueDialog = (page: Page, result: string | string[] | null) =>
    page.evaluate((result) => { (window as unknown as EditWindow).__dialogResults.push(result); }, result);

const openCrop = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, "Crop Images");
    await queueDialog(page, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(`Previewing ${paths[0].split("/").pop()}`, { exact: true })).toBeVisible();
};

const shellListening = (page: Page) => expect.poll(() => page.evaluate(() => (window as unknown as EditWindow).__invocations.some(({ command, args }) =>
    command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);

const shellCommand = (page: Page, command: string) =>
    page.evaluate((command) => (window as unknown as EditWindow).__emit("toolbox://shell-event", { kind: "command", command }), command);

const steps = (page: Page) => page.getByRole("list", { name: "Steps, in the order they run" });
const destination = (page: Page) => page.getByRole("region", { name: "Destination" });

test("the destination shows the Settings default before running, and a per-run folder is sent without changing it", async ({ page }) => {
    const paths = ["/photos/IMG_4012.jpg", "/photos/IMG_4013.jpg"];
    await openCrop(page, paths);

    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("Next to each original");
    await expect(destination(page).getByTestId("run-destination-example")).toContainText("IMG_4012-cropped.jpg");
    await expect(destination(page).getByText("This run only")).toHaveCount(0);

    await queueDialog(page, "/Volumes/Archive/Edited");
    await destination(page).getByRole("button", { name: "Change for this run" }).click();
    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("/Volumes/Archive/Edited");
    await expect(destination(page).getByText("This run only")).toBeVisible();
    await expect(destination(page)).toContainText("Your default is next to each original.");

    await page.getByRole("button", { name: "Run on 2 images" }).click();
    const [run] = await invocations(page, "export_image_edit_stack");
    expect(run.args).toMatchObject({ request: { paths, outputLocation: { customFolder: "/Volumes/Archive/Edited" } } });
    expect(await invocations(page, "set_output_settings")).toEqual([]);
    await page.evaluate((outcomes) => (window as unknown as EditWindow).__finish(outcomes), paths.map((path) => edited(path, "/Volumes/Archive/Edited")));
    await expect(page.getByRole("button", { name: "Open IMG_4012-cropped.jpg" })).toBeVisible();

    await destination(page).getByRole("button", { name: "Reset to default" }).click();
    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("Next to each original");
    expect(await invocations(page, "set_output_settings")).toEqual([]);
});

test("a large batch streams per-file progress and rows while it runs", async ({ page }) => {
    const paths = Array.from({ length: 24 }, (_, index) => `/photos/IMG_${4000 + index}.jpg`);
    await openCrop(page, paths);
    await page.getByRole("button", { name: "Run on 24 images" }).click();
    await expect.poll(async () => (await invocations(page, "export_image_edit_stack")).length).toBe(1);
    const [run] = await invocations(page, "export_image_edit_stack");
    const runId = (run.args as { runId: string }).runId;
    expect(runId).toMatch(/\S+/);
    const emit = (payload: Record<string, unknown>) =>
        page.evaluate((payload) => (window as unknown as EditWindow).__emit("batch-progress", payload), { runId, ...payload });

    const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
    await expect(rows).toHaveCount(24);
    await expect(page.locator(".live-run-bar > span")).toHaveCount(24);
    for (let index = 0; index < 8; index += 1) await emit({ index, phase: "finished", outcome: edited(paths[index]) });
    await emit({ index: 8, phase: "started" });

    const status = page.getByRole("status").filter({ has: page.locator("strong") }).filter({ hasText: /8 of 24/ });
    await expect(status.locator("strong")).toHaveText(/8 of 24$/);
    await expect(rows.nth(0)).toContainText("IMG_4000.jpg");
    await expect(rows.nth(0).getByRole("button", { name: "Open IMG_4000-cropped.jpg" })).toBeVisible();
    await expect(rows.nth(23)).toContainText("Waiting");
    expect(await page.locator(".live-run-bar > span").evaluateAll((spans) => spans.slice(0, 10).map((span) => span.getAttribute("data-tone"))))
        .toEqual(["ok", "ok", "ok", "ok", "ok", "ok", "ok", "ok", "wait", "wait"]);

    await page.evaluate((outcomes) => (window as unknown as EditWindow).__finish(outcomes), paths.map((path) => edited(path)));
    await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveCount(24);
    await expect(page.locator(".live-run-bar")).toHaveCount(0);
});

test("add files, run, undo and redo work from the keyboard and stay out of text fields", async ({ page }) => {
    const paths = ["/photos/a.png", "/photos/b.png"];
    await openCrop(page, paths);
    await shellListening(page);

    await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveAttribute("aria-keyshortcuts", "Meta+O");
    await queueDialog(page, ["/photos/c.png"]);
    await page.keyboard.press("Meta+O");
    await expect(page.getByRole("button", { name: "Run on 3 images" })).toBeVisible();

    // Add a second step, then undo and redo it through the native Edit menu commands.
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Resize", exact: true }).click();
    await expect(steps(page).getByRole("button", { name: "Remove Resize step" })).toHaveCount(1);
    await shellCommand(page, "undo");
    await expect(steps(page).getByRole("button", { name: "Remove Resize step" })).toHaveCount(0);
    await shellCommand(page, "redo");
    await expect(steps(page).getByRole("button", { name: "Remove Resize step" })).toHaveCount(1);

    // While typing, the menu's undo belongs to the field and the stack stays as it is.
    const width = page.getByRole("spinbutton", { name: "Longest side, pixels" });
    await width.focus();
    await shellCommand(page, "undo");
    await expect(steps(page).getByRole("button", { name: "Remove Resize step" })).toHaveCount(1);
    await shellCommand(page, "redo");
    await expect(steps(page).getByRole("button", { name: "Remove Resize step" })).toHaveCount(1);
    await queueDialog(page, ["/photos/d.png"]);
    await page.keyboard.press("Meta+O");
    await expect(page.getByRole("button", { name: "Run on 3 images" })).toBeVisible();
    expect(await page.evaluate(() => (window as unknown as EditWindow).__dialogResults.length)).toBe(1);
    await page.evaluate(() => { (window as unknown as EditWindow).__dialogResults = []; });

    await page.getByRole("button", { name: "Run on 3 images" }).focus();
    await page.keyboard.press("Meta+Enter");
    await expect.poll(async () => (await invocations(page, "export_image_edit_stack")).length).toBe(1);
});

test("every stack control has a name and is reachable from the keyboard", async ({ page }) => {
    await openCrop(page, ["/photos/a.png", "/photos/b.png"]);
    const toolbar = page.getByRole("toolbar", { name: "Edit stack" });
    await expect(toolbar.getByRole("button", { name: "Undo, ⌘Z" })).toBeVisible();
    await expect(toolbar.getByRole("button", { name: "Redo, ⇧⌘Z" })).toBeVisible();
    await expect(toolbar.getByRole("button", { name: "Reset stack" })).toBeVisible();
    await expect(steps(page).getByRole("button", { name: "Step 1 of 1, Crop, showing its options" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Run on 2 images" })).toHaveAttribute("aria-keyshortcuts", "Meta+Enter");

    const unnamed = await page.locator(".image-edit-stack").evaluate((root) => {
        const name = (element: Element) => {
            const labelledBy = element.getAttribute("aria-labelledby");
            const byLabel = labelledBy?.split(/\s+/).map((id) => document.getElementById(id)?.textContent ?? "").join(" ");
            const label = element.id ? document.querySelector(`label[for="${CSS.escape(element.id)}"]`)?.textContent : "";
            return (element.getAttribute("aria-label") || byLabel || label || element.closest("label")?.textContent || element.textContent || element.getAttribute("title") || "").trim();
        };
        return [...root.querySelectorAll("button, input, select, textarea, [role='button'], [role='radio'], [role='tab'], [role='slider'], [role='combobox'], [role='switch'], [role='checkbox']")]
            .filter((element) => !(element as HTMLElement).closest("[aria-hidden='true']") && (element as HTMLElement).offsetParent !== null)
            .filter((element) => !name(element))
            .map((element) => element.outerHTML.slice(0, 120));
    });
    expect(unnamed).toEqual([]);

    const reachable = await page.locator(".image-edit-stack").evaluate((root) =>
        [...root.querySelectorAll("button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [role='radio'], [role='tab'], [role='slider']")]
            .filter((element) => (element as HTMLElement).offsetParent !== null && !(element as HTMLElement).closest("[aria-hidden='true']"))
            .filter((element) => (element as HTMLElement).tabIndex < 0 && !element.closest("[role='radiogroup'], [role='tablist'], [role='toolbar'] [role='list']"))
            .map((element) => element.outerHTML.slice(0, 120)));
    expect(reachable).toEqual([]);

    await toolbar.getByRole("button", { name: "Reset stack" }).focus();
    await expect(toolbar.getByRole("button", { name: "Reset stack" })).toBeFocused();
});

for (const theme of ["light", "dark"] as const) {
    test(`the destination and live progress render in ${theme} mode`, async ({ page }, testInfo) => {
        await page.addInitScript((theme) => localStorage.setItem("toolbox-theme", theme), theme);
        const paths = Array.from({ length: 5 }, (_, index) => `/photos/IMG_${4012 + index}.jpg`);
        await openCrop(page, paths);
        await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
        await queueDialog(page, "/Volumes/Archive/Edited");
        await destination(page).getByRole("button", { name: "Change for this run" }).click();
        await expect(destination(page).getByText("This run only")).toBeVisible();
        await page.screenshot({ path: testInfo.outputPath(`image-edit-destination-${theme}.png`), fullPage: true });

        await page.getByRole("button", { name: "Run on 5 images" }).click();
        await expect.poll(async () => (await invocations(page, "export_image_edit_stack")).length).toBe(1);
        const [run] = await invocations(page, "export_image_edit_stack");
        const runId = (run.args as { runId: string }).runId;
        await page.evaluate(({ runId, outcome }) => (window as unknown as EditWindow).__emit("batch-progress", { runId, index: 0, phase: "finished", outcome }), { runId, outcome: edited(paths[0], "/Volumes/Archive/Edited") });
        await expect(page.locator(".live-run-bar > span").first()).toHaveAttribute("data-tone", "ok");
        await page.screenshot({ path: testInfo.outputPath(`image-edit-progress-${theme}.png`), fullPage: true });
    });
}
