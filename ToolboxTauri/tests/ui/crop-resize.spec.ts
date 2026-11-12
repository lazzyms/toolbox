import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type CropWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __finish: (outcomes: Outcome[]) => void;
};

type CropArgs = { runId: string; request: unknown };

const outcome = (inputPath: string, fields: Partial<Outcome>): Outcome => ({
    inputPath, outputPaths: [], detail: "", state: "succeeded", failure: null, reason: null, warnings: [], ...fields,
});

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as CropWindow;
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

const cropRequests = (page: Page) => page.evaluate(() =>
    (window as unknown as CropWindow).__invocations.filter((call) => call.command === "crop_pdf").map((call) => call.args as CropArgs));
const queueDialog = (page: Page, result: unknown) => page.evaluate((result) => { (window as unknown as CropWindow).__dialog.push(result); }, result);
const finish = (page: Page, outcomes: Outcome[]) => page.evaluate((outcomes) => (window as unknown as CropWindow).__finish(outcomes), outcomes);
const panel = (page: Page) => page.locator(".workspace-control-panel");

const waitForShell = (page: Page) => expect.poll(() => page.evaluate(() => (window as unknown as CropWindow).__invocations.some(({ command, args }) =>
    command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);

const openCrop = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Crop and Resize PDF");
    await waitForShell(page);
};

const addPdfs = async (page: Page, paths: string[]) => {
    await page.evaluate((paths) => (window as unknown as CropWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
};

const margin = (page: Page, side: string) => panel(page).getByRole("textbox", { name: side, exact: true });

test("the PDF Editor rail opens Crop and Resize as a batch tool in crop mode", async ({ page }) => {
    await page.goto("/");
    await openToolBySearch(page, "Rotate PDFs");
    const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
    await rail.getByRole("button", { name: "Crop and Resize PDF" }).click();
    await expect(rail.getByRole("button", { name: "Crop and Resize PDF" })).toHaveAttribute("aria-pressed", "true");

    await expect(panel(page).getByRole("heading", { name: "Crop and Resize PDF" })).toBeVisible();
    const mode = page.getByRole("radiogroup", { name: "Mode" });
    await expect(mode.getByRole("radio", { name: "Crop pages" })).toBeChecked();
    await expect(mode.getByRole("radio", { name: "Resize pages" })).not.toBeChecked();
    await expect(panel(page)).toContainText("Trim the edges of every page.");
    await expect(panel(page)).toContainText("Names end in -cropped. A taken name gets -1, -2.");
    // A batch tool, not the full-window page editor.
    await expect(page.getByRole("toolbar", { name: "PDF editor tools", exact: true })).toHaveCount(0);

    await expect(page.getByTestId("crop-run-reason")).toHaveText("Add PDFs to crop.");
    await expect(page.getByRole("button", { name: "Crop PDFs", exact: true })).toBeDisabled();
});

test("Crop sends every PDF in one request with margins converted to points once", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf", "/local/letter.pdf"]);

    await panel(page).getByLabel("Unit").selectOption("in");
    await margin(page, "Top").fill("1");
    await margin(page, "Right").fill("0.5");
    await margin(page, "Bottom").fill("0");
    await margin(page, "Left").fill("0.25");
    await expect(panel(page).locator(".workspace-unit-input > span")).toHaveCount(4);

    await queueDialog(page, "/local/out");
    await page.getByRole("button", { name: "Change for this run" }).click();
    await expect(page.getByRole("button", { name: "Reset to default" })).toBeVisible();

    await page.getByRole("button", { name: "Crop 2 PDFs", exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ runId, request }] = await cropRequests(page);
    expect(runId).toMatch(/\S+/);
    expect(request).toEqual({
        paths: ["/local/a4.pdf", "/local/letter.pdf"],
        spec: { kind: "margins", top: 72, right: 36, bottom: 0, left: 18 },
        scope: "all",
        outputLocation: { customFolder: "/local/out" },
    });
});

test("Same on all sides sends one length for every side, in millimetres by default, next to each original", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/report.pdf"]);

    await panel(page).getByRole("checkbox", { name: "Same on all sides" }).check();
    await expect(margin(page, "Top")).toHaveCount(0);
    await margin(page, "All sides").fill("25.4");
    await expect(panel(page).locator(".workspace-unit-input > span")).toHaveCount(1);
    await page.getByRole("button", { name: "Crop 1 PDF", exact: true }).click();

    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: { paths: string[]; spec: Record<string, unknown>; scope: string; outputLocation: unknown } }>;
    expect(request.paths).toEqual(["/local/report.pdf"]);
    expect(request.scope).toBe("all");
    expect(request.outputLocation).toBe("alongsideInput");
    expect(request.spec.kind).toBe("margins");
    for (const side of ["top", "right", "bottom", "left"]) expect(request.spec[side]).toBeCloseTo(72, 9);
});

test("invalid margins are named, linked to their fields, and keep Run disabled until fixed", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/report.pdf", "/local/lease.pdf"]);
    const run = page.getByRole("button", { name: "Crop 2 PDFs", exact: true });
    await expect(run).toBeEnabled();
    await expect(page.getByRole("alert")).toHaveCount(0);

    await margin(page, "Top").fill("-5");
    await margin(page, "Bottom").fill("");
    const alert = page.locator("#mar-err");
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toHaveText("Top can't be less than 0. Bottom is empty; enter 0 for no margin.");
    for (const side of ["Top", "Bottom"]) {
        await expect(margin(page, side)).toHaveAttribute("aria-invalid", "true");
        await expect(margin(page, side)).toHaveAttribute("aria-describedby", "mar-err");
    }
    for (const side of ["Right", "Left"]) await expect(margin(page, side)).not.toHaveAttribute("aria-invalid", "true");
    await expect(page.getByTestId("crop-run-reason")).toHaveText("Fix the margins to crop.");
    await expect(run).toBeDisabled();

    await margin(page, "Top").fill("abc");
    await expect(alert).toContainText("Top must be a number.");

    await margin(page, "Top").fill("10");
    await margin(page, "Bottom").fill("0");
    await expect(alert).toHaveCount(0);
    await expect(page.getByTestId("crop-run-reason")).toHaveCount(0);
    await expect(run).toBeEnabled();

    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Resize pages" }).click();
    await expect(panel(page)).toContainText("Change the paper size of every page.");
    await expect(page.getByRole("button", { name: "Resize 2 PDFs", exact: true })).toBeDisabled();
    expect(await cropRequests(page)).toHaveLength(0);
});

test("the crop note explains hiding and opens Redact PDF with the same files", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/report.pdf"]);
    await expect(page.getByTestId("crop-hidden-note")).toContainText("Crop hides the rest of the page. It stays in the file and can be shown again.");
    await page.getByRole("button", { name: "Remove content with Redact PDF" }).click();
    await expect(page.getByRole("toolbar", { name: "PDF Editor tools" }).getByRole("button", { name: "Redact PDF" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("report.pdf", { exact: true }).first()).toBeVisible();
});

test("each PDF reports its own result, and a page with no area left fails only its PDF", async ({ page }) => {
    await openCrop(page);
    const paths = ["/local/a4.pdf", "/local/narrow.pdf", "/local/letter.pdf"];
    await addPdfs(page, paths);
    await page.getByRole("button", { name: "Crop 3 PDFs", exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);

    await finish(page, [
        outcome(paths[0], { outputPaths: ["/local/a4-cropped.pdf"] }),
        outcome(paths[1], { state: "failed", failure: { kind: "invalidInput", message: "Page 2 has no area left after these margins.", pointer: null } }),
        outcome(paths[2], { outputPaths: ["/local/letter-cropped-1.pdf"], warnings: [{ code: "renamedToAvoidReplacing", message: "", details: { chosenName: "letter-cropped.pdf", finalName: "letter-cropped-1.pdf" } }] }),
    ]);

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(3);
    await expect(results.nth(0)).toContainText("a4-cropped.pdf");
    await expect(results.nth(0)).toContainText("Cropped");
    await expect(results.nth(1)).toContainText("Page 2 has no area left after these margins.");
    await expect(results.nth(1)).toContainText("Failed, the input can't be used");
    await expect(results.nth(2)).toContainText("Saved as letter-cropped-1.pdf.");
});
