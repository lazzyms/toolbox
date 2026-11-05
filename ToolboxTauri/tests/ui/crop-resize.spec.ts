import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type CropWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __finish: (outcomes: Outcome[]) => void;
    /** Displayed page sizes, in points, for each PDF the preview can read. */
    __scenes: Record<string, Array<{ width: number; height: number }>>;
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
                if (command === "inspect_pdf_scene" || command === "inspect_pdf_scene_page") {
                    const { path, pageIndex } = (args as { request: { path: string; pageIndex?: number } }).request;
                    const sizes = win.__scenes?.[path];
                    if (!sizes) throw "No preview in this test.";
                    const toPage = (size: { width: number; height: number }, index: number) =>
                        ({ index, width: size.width, height: size.height, rotation: 0, pageBox: [0, 0, size.width, size.height], preview: null });
                    if (command === "inspect_pdf_scene") return { path, pages: sizes.map(toPage) };
                    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#eee"/></svg>`;
                    return { ...toPage(sizes[pageIndex ?? 0], pageIndex ?? 0), preview: `data:image/svg+xml,${encodeURIComponent(svg)}`, textRuns: [] };
                }
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

const resizeRequests = (page: Page) => page.evaluate(() =>
    (window as unknown as CropWindow).__invocations.filter((call) => call.command === "resize_pdf").map((call) => call.args as CropArgs));
const useResize = (page: Page) => page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Resize pages" }).click();

const margin = (page: Page, side: string) => panel(page).getByRole("textbox", { name: side, exact: true });
const useMargins = (page: Page) => panel(page).getByRole("radiogroup", { name: "Crop by" }).getByRole("radio", { name: "Margins" }).click();

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
    await useMargins(page);

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
    await useMargins(page);

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
    await useMargins(page);
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

    // Margins belong to Crop: Resize runs without them, even when they are invalid.
    await margin(page, "Top").fill("-5");
    await useResize(page);
    await expect(panel(page)).toContainText("Change the paper size of every page.");
    await expect(page.getByTestId("crop-run-reason")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Resize 2 PDFs", exact: true })).toBeEnabled();
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

const A4 = { width: 595.28, height: 841.89 };
const LETTER = { width: 612, height: 792 };
const LETTER_TURNED = { width: 792, height: 612 };
const SMALL = { width: 58 * 72 / 25.4, height: 120 * 72 / 25.4 };
const DEFAULT = { left: 18 / 210, top: 22 / 297, width: 174 / 210, height: 250 / 297 };

const withScenes = (page: Page, scenes: Record<string, Array<{ width: number; height: number }>>) =>
    page.addInitScript((scenes) => { (window as unknown as CropWindow).__scenes = scenes; }, scenes);

const rectField = (page: Page, name: string) => panel(page).getByRole("textbox", { name, exact: true });
const preview = (page: Page) => page.getByRole("region", { name: "Preview" });
const stage = (page: Page) => preview(page).getByRole("group", { name: "Crop rectangle" });
const caption = (page: Page) => page.getByTestId("crop-preview-caption");

/** Scrolls the whole preview page into the viewport so pointer events reach every part of it. */
const showStage = (page: Page) => stage(page).evaluate((element) => element.scrollIntoView({ block: "center" }));

/** Page coordinates of a point given as fractions of the previewed page. */
const at = async (page: Page, x: number, y: number) => {
    await showStage(page);
    const box = (await stage(page).boundingBox())!;
    return { x: box.x + x * box.width, y: box.y + y * box.height };
};

const drag = async (page: Page, from: [number, number], to: [number, number]) => {
    const start = await at(page, ...from);
    const end = await at(page, ...to);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move((start.x + end.x) / 2, (start.y + end.y) / 2, { steps: 3 });
    await page.mouse.move(end.x, end.y, { steps: 3 });
    await expect(preview(page).locator(".crop-chip")).toBeVisible();
    await page.mouse.up();
    await expect(preview(page).locator(".crop-chip")).toHaveCount(0);
};

const expectRect = async (page: Page, mm: { left: string; top: string; width: string; height: string }) => {
    for (const [name, value] of Object.entries(mm)) await expect(rectField(page, name[0].toUpperCase() + name.slice(1))).toHaveValue(value);
};

const sentRect = async (page: Page, label: string) => {
    await page.getByRole("button", { name: label, exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: { spec: Record<string, unknown>; scope: string } }>;
    expect(request.scope).toBe("all");
    expect(request.spec.kind).toBe("rectangle");
    return request.spec as { left: number; top: number; width: number; height: number };
};

test("Crop by starts on Rectangle with the board's rectangle, shown in millimetres on the previewed page and sent as fractions", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4], "/local/letter.pdf": [LETTER] });
    await openCrop(page);
    const cropBy = panel(page).getByRole("radiogroup", { name: "Crop by" });
    await expect(cropBy.getByRole("radio", { name: "Rectangle" })).toBeChecked();
    await expect(cropBy.getByRole("radio", { name: "Margins" })).not.toBeChecked();
    await expect(page.getByTestId("crop-rect-hint")).toHaveText("Add PDFs, then draw the rectangle on the preview or type it here.");
    await expect(rectField(page, "Width")).toBeDisabled();
    await expect(panel(page).getByLabel("Unit")).toHaveCount(0);

    await addPdfs(page, ["/local/a4.pdf", "/local/letter.pdf"]);
    await expect(page.getByTestId("crop-rect-hint")).toHaveText("Drag on the preview, or type. Arrow keys move it 1 mm. Shift+Arrow resizes it.");
    await expectRect(page, { left: "18", top: "22", width: "174", height: "250" });
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm. Keeps 174 × 250 mm.");
    await expect(preview(page).getByRole("img", { name: "Page 1, A4. The kept area is 174 by 250 mm." })).toBeVisible();
    await expect(preview(page).getByText("Page 1 of 1")).toBeVisible();
    await expect(preview(page).locator(".crop-handle")).toHaveCount(8);
    await expect(preview(page).getByRole("button", { name: "Undo" })).toBeDisabled();
    await expect(preview(page).getByRole("button", { name: "Reset crop" })).toBeDisabled();
    await expect(preview(page).getByRole("button", { name: "Undo" }).locator("kbd")).toHaveText("⌘Z");

    // The fields measure the page in the preview; the rectangle itself stays relative.
    await preview(page).getByLabel("PDF to preview").selectOption({ label: "letter.pdf" });
    await expectRect(page, { left: "18.5", top: "20.7", width: "178.9", height: "235.2" });
    await expect(caption(page)).toHaveText("Letter, 216 × 279 mm. Keeps 179 × 235 mm.");

    const spec = await sentRect(page, "Crop 2 PDFs");
    for (const key of ["left", "top", "width", "height"] as const) expect(spec[key]).toBeCloseTo(DEFAULT[key], 9);
});

test("drawing, dragging a handle and moving the rectangle on the preview each change the request, and Undo steps back", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4] });
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf"]);
    await expect(stage(page)).toBeVisible();

    // Drawing outside the rectangle starts a new one.
    await drag(page, [0.02, 0.02], [0.52, 0.62]);
    await expectRect(page, { left: "4.2", top: "5.9", width: "105", height: "178.2" });
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm. Keeps 105 × 178 mm.");

    // A corner handle moves only its own edges.
    const se = preview(page).locator(".crop-handle-se");
    await showStage(page);
    const corner = (await se.boundingBox())!;
    const target = await at(page, 0.8, 0.9);
    await page.mouse.move(corner.x + corner.width / 2, corner.y + corner.height / 2);
    await page.mouse.down();
    await page.mouse.move(target.x, target.y, { steps: 4 });
    await page.mouse.up();
    await expectRect(page, { left: "4.2", top: "5.9", width: "163.8", height: "261.4" });

    // Dragging inside the rectangle moves it and keeps its size.
    await drag(page, [0.4, 0.4], [0.5, 0.45]);
    await expectRect(page, { left: "25.2", top: "20.8", width: "163.8", height: "261.4" });

    const undo = preview(page).getByRole("button", { name: "Undo" });
    await undo.click();
    await expectRect(page, { left: "4.2", top: "5.9", width: "163.8", height: "261.4" });
    await stage(page).focus();
    await page.keyboard.press("Meta+z");
    await expectRect(page, { left: "4.2", top: "5.9", width: "105", height: "178.2" });
    await undo.click();
    await expectRect(page, { left: "18", top: "22", width: "174", height: "250" });
    await expect(undo).toBeDisabled();

    await drag(page, [0.02, 0.02], [0.52, 0.62]);
    const spec = await sentRect(page, "Crop 1 PDF");
    expect(spec.left).toBeCloseTo(0.02, 2);
    expect(spec.top).toBeCloseTo(0.02, 2);
    expect(spec.width).toBeCloseTo(0.5, 2);
    expect(spec.height).toBeCloseTo(0.6, 2);
});

test("arrow keys move the rectangle 1 mm, Shift+Arrow resizes it, and Ctrl+Z undoes on Windows", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4] });
    await page.addInitScript(() => Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" }));
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf"]);
    await expect(preview(page).getByRole("button", { name: "Undo" }).locator("kbd")).toHaveText("Ctrl+Z");

    await stage(page).focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowUp");
    await expectRect(page, { left: "19", top: "21", width: "174", height: "250" });
    await page.keyboard.press("Shift+ArrowLeft");
    await page.keyboard.press("Shift+ArrowDown");
    await expectRect(page, { left: "19", top: "21", width: "173", height: "251" });
    await expect(preview(page).getByRole("img", { name: "Page 1, A4. The kept area is 173 by 251 mm." })).toBeVisible();

    await page.keyboard.press("Control+z");
    await expectRect(page, { left: "19", top: "21", width: "173", height: "250" });
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Control+z");
    await expectRect(page, { left: "18", top: "22", width: "174", height: "250" });
});

test("typed lengths crop exactly, an empty or off-page rectangle is named and blocks Run, and Reset crop restores the default", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4] });
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf"]);
    const run = page.getByRole("button", { name: "Crop 1 PDF", exact: true });
    const alert = page.locator("#rect-err");

    await rectField(page, "Width").fill("300");
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toHaveText("The rectangle goes past the right edge of the page.");
    for (const name of ["Left", "Width"]) {
        await expect(rectField(page, name)).toHaveAttribute("aria-invalid", "true");
        await expect(rectField(page, name)).toHaveAttribute("aria-describedby", "rect-err");
    }
    await expect(rectField(page, "Top")).not.toHaveAttribute("aria-invalid", "true");
    await expect(page.getByTestId("crop-run-reason")).toHaveText("Fix the rectangle to crop.");
    await expect(run).toBeDisabled();
    await expect(caption(page)).toHaveText("Fix the rectangle to see the crop.");
    await expect(preview(page).locator(".crop-frame")).toHaveCount(0);

    await rectField(page, "Width").fill("0");
    await expect(alert).toHaveText("Width must be more than 0.");
    await rectField(page, "Height").fill("");
    await expect(alert).toHaveText("Height is empty. Width must be more than 0.");
    await expect(run).toBeDisabled();

    await rectField(page, "Width").fill("105");
    await rectField(page, "Height").fill("99");
    await rectField(page, "Height").blur();
    await expect(alert).toHaveCount(0);
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm. Keeps 105 × 99 mm.");
    await expect(run).toBeEnabled();

    const reset = preview(page).getByRole("button", { name: "Reset crop" });
    await reset.click();
    await expectRect(page, { left: "18", top: "22", width: "174", height: "250" });
    await expect(reset).toBeDisabled();
    // Undo brings back the typed rectangle.
    await preview(page).getByRole("button", { name: "Undo" }).click();
    await expectRect(page, { left: "18", top: "22", width: "105", height: "99" });

    // Lengths are measured on the previewed A4 page, 595.28 by 841.89 points.
    const spec = await sentRect(page, "Crop 1 PDF");
    expect(spec.width).toBeCloseTo((105 * 72 / 25.4) / A4.width, 9);
    expect(spec.height).toBeCloseTo((99 * 72 / 25.4) / A4.height, 9);
    expect(spec.left).toBeCloseTo(18 / 210, 9);
});

test("small previews show where the same rectangle lands on the batch's other page sizes, and margins show what they leave", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4, A4], "/local/letter.pdf": [LETTER, LETTER_TURNED], "/local/receipt.pdf": [SMALL] });
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf", "/local/letter.pdf", "/local/receipt.pdf"]);

    const minis = preview(page).getByTestId("crop-mini");
    await expect(preview(page)).toContainText("On other page sizes");
    await expect(minis).toHaveCount(3);
    await expect(minis.nth(0)).toContainText("letter.pdf");
    await expect(minis.nth(0)).toContainText("Letter. Keeps 179 × 235 mm");
    await expect(minis.nth(1)).toContainText("letter.pdf, page 2");
    await expect(minis.nth(1)).toContainText("Letter, landscape. Keeps 232 × 182 mm");
    await expect(minis.nth(2)).toContainText("receipt.pdf");
    await expect(minis.nth(2)).toContainText("58 × 120 mm. Keeps 48 × 101 mm");
    await expect(preview(page)).toContainText("The rectangle keeps its place relative to each page, so it lands in the same spot on any size or orientation.");

    await preview(page).getByRole("button", { name: "Next page in range" }).click();
    await expect(preview(page).getByText("Page 2 of 2")).toBeVisible();
    await preview(page).getByLabel("PDF to preview").selectOption({ label: "letter.pdf" });
    await expect(preview(page).getByText("Page 1 of 2")).toBeVisible();
    await preview(page).getByRole("button", { name: "Next page in range" }).click();
    await expect(caption(page)).toHaveText("Letter, landscape, 279 × 216 mm. Keeps 232 × 182 mm.");
    await expect(preview(page).getByRole("button", { name: "Next page in range" })).toBeDisabled();
    await expect.poll(() => page.evaluate(() => (window as unknown as CropWindow).__invocations.some(({ command, args }) =>
        command === "inspect_pdf_scene_page" && JSON.stringify(args) === JSON.stringify({ request: { path: "/local/letter.pdf", pageIndex: 1 } })))).toBe(true);

    // Margins mode previews the margins read-only and warns about pages they empty.
    await useMargins(page);
    await expect(preview(page).getByRole("button", { name: "Undo" })).toHaveCount(0);
    await expect(preview(page).locator(".crop-handle")).toHaveCount(0);
    await expect(stage(page)).not.toHaveAttribute("tabindex", "0");
    await expect(preview(page)).toContainText("Margins trim the same lengths from every page, so small pages lose more of their area.");
    await expect(minis.filter({ hasText: "receipt.pdf" })).toContainText("58 × 120 mm. No area left. This PDF will fail.");
    await margin(page, "Top").fill("-1");
    await expect(caption(page)).toHaveText("Fix the margins to see the crop.");
});

const pagesField = (page: Page) => panel(page).getByRole("textbox", { name: "Pages", exact: true });

test("Pages sends the typed range exactly as entered, and an empty field crops every page", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/lease.pdf", "/local/cover.pdf"]);
    await expect(pagesField(page)).toHaveValue("");
    await expect(pagesField(page)).toHaveAttribute("placeholder", "All pages");
    await expect(pagesField(page)).toHaveAttribute("aria-describedby", "crop-pages-help");
    await expect(panel(page).locator("#crop-pages-help")).toHaveText("Empty means all pages. Pages past the end are ignored. A PDF with none left is skipped.");

    await pagesField(page).fill(" 2-4,  9- ");
    await page.getByRole("button", { name: "Crop 2 PDFs", exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: { paths: string[]; scope: unknown } }>;
    expect(request.paths).toEqual(["/local/lease.pdf", "/local/cover.pdf"]);
    expect(request.scope).toEqual({ range: { ranges: " 2-4,  9- " } });
});

test("an unreadable page range is named, linked to Pages, and keeps Run disabled until fixed", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/lease.pdf"]);
    const run = page.getByRole("button", { name: "Crop 1 PDF", exact: true });
    await expect(run).toBeEnabled();

    await pagesField(page).fill("1-3,,2");
    const alert = panel(page).locator("#crop-pages-err");
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toHaveText("Entry 2 of the page range is empty. Remove the extra comma.");
    await expect(pagesField(page)).toHaveAttribute("aria-invalid", "true");
    await expect(pagesField(page)).toHaveAttribute("aria-describedby", "crop-pages-err crop-pages-help");
    await expect(page.getByTestId("crop-run-reason")).toHaveText("Fix the pages to crop.");
    await expect(run).toBeDisabled();

    await pagesField(page).fill("1-3, 2");
    await expect(alert).toHaveCount(0);
    await expect(pagesField(page)).not.toHaveAttribute("aria-invalid", "true");
    await expect(page.getByTestId("crop-run-reason")).toHaveCount(0);
    await expect(run).toBeEnabled();

    await pagesField(page).fill("");
    await run.click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: { scope: unknown } }>;
    expect(request.scope).toBe("all");
});

test("a ranged crop reports a cropped PDF, a Skipped PDF with its reason, a removed signature and a failure in one batch", async ({ page }) => {
    await openCrop(page);
    const paths = ["/local/lease.pdf", "/local/cover.pdf", "/local/vendor-nda.pdf", "/local/payroll.pdf"];
    await addPdfs(page, paths);
    await pagesField(page).fill("3-");
    await page.getByRole("button", { name: "Crop 4 PDFs", exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: { paths: string[]; scope: unknown } }>;
    expect(request.paths).toEqual(paths);
    expect(request.scope).toEqual({ range: { ranges: "3-" } });

    await finish(page, [
        outcome(paths[0], { outputPaths: ["/local/lease-cropped.pdf"], detail: "PDF saved" }),
        outcome(paths[1], { state: "skipped", reason: { code: "rangeEmptyAfterClamping", message: "No pages in 3- are in this PDF, which has 2 pages.", pointer: null } }),
        outcome(paths[2], { outputPaths: ["/local/vendor-nda-cropped.pdf"], detail: "PDF saved", warnings: [{ code: "signatureRemoved", message: "native wording" }] }),
        outcome(paths[3], { state: "failed", failure: { kind: "invalidInput", message: "Unlock the PDF before editing", pointer: { kind: "tool", toolId: "lock-unlock" } } }),
    ]);

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(4);
    await expect(results.nth(0)).toHaveAttribute("data-tone", "ok");
    await expect(results.nth(0)).toContainText("lease-cropped.pdf");
    await expect(results.nth(1)).toHaveAttribute("data-tone", "skip");
    await expect(results.nth(1)).toContainText("Skipped");
    await expect(results.nth(1)).toContainText("No pages in 3- are in this PDF, which has 2 pages.");
    await expect(results.nth(1).getByRole("button", { name: /^(Open|Show in)/ })).toHaveCount(0);
    await expect(results.nth(2)).toHaveAttribute("data-tone", "warn");
    await expect(results.nth(2)).toContainText("The digital signature was removed, because changing the file breaks it.");
    await expect(results.nth(2)).not.toContainText("native wording");
    await expect(results.nth(3)).toHaveAttribute("data-tone", "fail");
    await expect(results.nth(3)).toContainText("Unlock the PDF before editing");
});

const presets = [
    { preset: "a3", label: "A3, 297 × 420 mm" },
    { preset: "a4", label: "A4, 210 × 297 mm" },
    { preset: "a5", label: "A5, 148 × 210 mm" },
    { preset: "letter", label: "Letter, 216 × 279 mm" },
    { preset: "legal", label: "Legal, 216 × 356 mm" },
    { preset: "tabloid", label: "Tabloid, 279 × 432 mm" },
];

test("Resize shows only its own options and sends the exact request for each preset, never Crop's values", async ({ page }) => {
    await openCrop(page);
    const paths = ["/local/lease.pdf", "/local/scan.pdf"];
    await addPdfs(page, paths);
    await useMargins(page);
    await margin(page, "Top").fill("40");
    await useResize(page);

    await expect(panel(page).getByRole("radiogroup", { name: "Crop by" })).toHaveCount(0);
    await expect(margin(page, "Top")).toHaveCount(0);
    await expect(page.getByTestId("crop-hidden-note")).toHaveCount(0);
    await expect(panel(page)).toContainText("Names end in -resized. A taken name gets -1, -2.");
    await expect(panel(page)).toContainText("Links, comments and form fields move with the content. Text stays sharp.");
    const size = panel(page).getByLabel("Page size");
    await expect(size.locator("option")).toHaveText([...presets.map(({ label }) => label), "Custom size"]);
    await expect(panel(page).getByRole("radiogroup", { name: "Orientation" }).getByRole("radio")).toHaveText(["Match page", "Portrait", "Landscape"]);
    await expect(panel(page).getByRole("textbox", { name: "Width", exact: true })).toHaveCount(0);
    await expect(size).toHaveValue("a4");
    await expect(panel(page).getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Match page" })).toBeChecked();
    await expect(panel(page).getByRole("radiogroup", { name: "Placement" }).getByRole("radio", { name: /^Scale to fit, centered/ })).toBeChecked();
    await expect(panel(page).getByRole("radiogroup", { name: "Placement" })).toContainText("Nothing is cut off");

    const run = page.getByRole("button", { name: "Resize 2 PDFs", exact: true });
    for (const [index, { preset, label }] of presets.entries()) {
        await size.selectOption({ label });
        await run.click();
        await expect.poll(async () => (await resizeRequests(page)).length).toBe(index + 1);
        const { runId, request } = (await resizeRequests(page))[index];
        expect(runId).toMatch(/\S+/);
        expect(request).toEqual({
            paths,
            size: { preset },
            orientation: "match",
            placement: "fit",
            scope: "all",
            outputLocation: "alongsideInput",
        });
        await finish(page, paths.map((path) => outcome(path, { outputPaths: [path.replace(".pdf", "-resized.pdf")] })));
        await expect(run).toBeEnabled();
    }
    expect(await cropRequests(page)).toHaveLength(0);

    // Back in Crop, the margins are still there and Crop sends only its own values.
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Crop pages" }).click();
    await expect(margin(page, "Top")).toHaveValue("40");
    await page.getByRole("button", { name: "Crop 2 PDFs", exact: true }).click();
    await expect.poll(async () => (await cropRequests(page)).length).toBe(1);
    const [{ request }] = await cropRequests(page) as Array<{ request: Record<string, unknown> }>;
    expect(Object.keys(request).sort()).toEqual(["outputLocation", "paths", "scope", "spec"]);
    expect(await resizeRequests(page)).toHaveLength(presets.length);
});

test("Resize sends the typed range exactly as entered and names a bad range before running", async ({ page }) => {
    await openCrop(page);
    await useResize(page);
    await expect(page.getByTestId("crop-run-reason")).toHaveText("Add PDFs to resize.");
    await expect(page.getByRole("button", { name: "Resize PDFs", exact: true })).toBeDisabled();
    await addPdfs(page, ["/local/lease.pdf"]);
    const run = page.getByRole("button", { name: "Resize 1 PDF", exact: true });

    await pagesField(page).fill("1-3,,2");
    await expect(panel(page).locator("#crop-pages-err")).toHaveText("Entry 2 of the page range is empty. Remove the extra comma.");
    await expect(page.getByTestId("crop-run-reason")).toHaveText("Fix the pages to resize.");
    await expect(run).toBeDisabled();

    await pagesField(page).fill(" 3-14, 20- ");
    await panel(page).getByLabel("Page size").selectOption("letter");
    await run.click();
    await expect.poll(async () => (await resizeRequests(page)).length).toBe(1);
    const [{ request }] = await resizeRequests(page) as Array<{ request: { scope: unknown; size: unknown } }>;
    expect(request.scope).toEqual({ range: { ranges: " 3-14, 20- " } });
    expect(request.size).toEqual({ preset: "letter" });
});

test("a ranged resize reports a resized PDF, a Skipped PDF already at the size, a removed signature and a failure", async ({ page }) => {
    await openCrop(page);
    const paths = ["/local/lease.pdf", "/local/handbook.pdf", "/local/vendor-nda.pdf", "/local/payroll.pdf"];
    await addPdfs(page, paths);
    await useResize(page);
    await pagesField(page).fill("3-");
    await page.getByRole("button", { name: "Resize 4 PDFs", exact: true }).click();
    await expect.poll(async () => (await resizeRequests(page)).length).toBe(1);
    await expect(page.getByText("Resizing 0 of 4")).toBeVisible();

    await finish(page, [
        outcome(paths[0], { outputPaths: ["/local/lease-resized.pdf"], detail: "Pages 3–22 changed to A4." }),
        outcome(paths[1], { state: "skipped", reason: { code: "noEffectiveChange", message: "Pages 3–14 are already A4. Nothing was saved.", pointer: null } }),
        outcome(paths[2], { outputPaths: ["/local/vendor-nda-resized.pdf"], detail: "Pages 3–4 changed to A4.", warnings: [{ code: "signatureRemoved", message: "native wording" }] }),
        outcome(paths[3], { state: "failed", failure: { kind: "invalidInput", message: "Unlock the PDF before editing", pointer: { kind: "tool", toolId: "lock-unlock" } } }),
    ]);

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(4);
    await expect(results.nth(0)).toHaveAttribute("data-tone", "ok");
    await expect(results.nth(0)).toContainText("Resized");
    await expect(results.nth(0)).toContainText("lease-resized.pdf");
    await expect(results.nth(1)).toHaveAttribute("data-tone", "skip");
    await expect(results.nth(1)).toContainText("Skipped");
    await expect(results.nth(1)).toContainText("Pages 3–14 are already A4. Nothing was saved.");
    await expect(results.nth(1).getByRole("button", { name: /^(Open|Show in)/ })).toHaveCount(0);
    await expect(results.nth(2)).toHaveAttribute("data-tone", "warn");
    await expect(results.nth(2)).toContainText("The digital signature was removed, because changing the file breaks it.");
    await expect(results.nth(3)).toHaveAttribute("data-tone", "fail");
    await expect(results.nth(3)).toContainText("Unlock the PDF before editing");
});

const sizeField = (page: Page, name: "Width" | "Height") => panel(page).getByRole("textbox", { name, exact: true });
const useCustomSize = (page: Page) => panel(page).getByLabel("Page size").selectOption({ label: "Custom size" });
const orientationRadio = (page: Page, name: string) => panel(page).getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name });
const placementRadio = (page: Page, name: RegExp) => panel(page).getByRole("radiogroup", { name: "Placement" }).getByRole("radio", { name });

test("a custom size is sent in points, converted once from mm, in or pt, with each orientation and placement", async ({ page }) => {
    await openCrop(page);
    const paths = ["/local/lease.pdf", "/local/scan.pdf"];
    await addPdfs(page, paths);
    await useResize(page);
    await useCustomSize(page);
    const customSize = panel(page).getByRole("region", { name: "Custom size" });
    await expect(sizeField(page, "Width")).toHaveValue("200");
    await expect(sizeField(page, "Height")).toHaveValue("250");
    await expect(customSize.locator(".workspace-unit-input span")).toHaveText(["mm", "mm"]);
    await expect(placementRadio(page, /^Actual size, centered/)).toBeVisible();
    await expect(panel(page).getByRole("radiogroup", { name: "Placement" })).toContainText("Edges may be cut off");

    const run = page.getByRole("button", { name: "Resize 2 PDFs", exact: true });
    const sent = async (count: number) => {
        await expect.poll(async () => (await resizeRequests(page)).length).toBe(count);
        const { request } = (await resizeRequests(page))[count - 1] as { request: Record<string, unknown> };
        await finish(page, paths.map((path) => outcome(path, { outputPaths: [path.replace(".pdf", "-resized.pdf")] })));
        await expect(run).toBeEnabled();
        return request;
    };
    const millimetres = { width: 200 * (72 / 25.4), height: 250 * (72 / 25.4) };
    let count = 0;
    for (const [orientationName, orientation] of [["Match page", "match"], ["Portrait", "portrait"], ["Landscape", "landscape"]] as const) {
        for (const [placementName, placement] of [[/^Scale to fit, centered/, "fit"], [/^Actual size, centered/, "actualSize"]] as const) {
            await orientationRadio(page, orientationName).click();
            await placementRadio(page, placementName).click();
            await expect(orientationRadio(page, orientationName)).toBeChecked();
            await expect(placementRadio(page, placementName)).toBeChecked();
            await run.click();
            expect(await sent(++count)).toEqual({
                paths,
                size: { custom: millimetres },
                orientation,
                placement,
                scope: "all",
                outputLocation: "alongsideInput",
            });
        }
    }

    // Inches and points: the unit changes what the typed numbers mean, and they are converted once.
    await customSize.getByLabel("Unit").selectOption("in");
    await expect(customSize.locator(".workspace-unit-input span")).toHaveText(["in", "in"]);
    await sizeField(page, "Width").fill("8.5");
    await sizeField(page, "Height").fill("11");
    await run.click();
    const inches = await sent(++count);
    expect(inches.size).toEqual({ custom: { width: 612, height: 792 } });
    expect(inches).toMatchObject({ orientation: "landscape", placement: "actualSize" });

    await customSize.getByLabel("Unit").selectOption("pt");
    await sizeField(page, "Width").fill(" 300.5 ");
    await sizeField(page, "Height").fill("400");
    await run.click();
    expect((await sent(++count)).size).toEqual({ custom: { width: 300.5, height: 400 } });
    expect(await cropRequests(page)).toHaveLength(0);
});

test("a custom size outside 72 to 14,400 pt is named, linked to its fields, and keeps Run disabled until fixed", async ({ page }) => {
    await openCrop(page);
    await addPdfs(page, ["/local/lease.pdf"]);
    await useResize(page);
    await useCustomSize(page);
    const run = page.getByRole("button", { name: "Resize 1 PDF", exact: true });
    const alert = panel(page).locator("#size-err");
    const reason = page.getByTestId("crop-run-reason");
    await expect(alert).toHaveCount(0);
    await expect(reason).toHaveCount(0);
    await expect(run).toBeEnabled();

    // The board's invalid state: 2 x 6000 mm.
    await sizeField(page, "Width").fill("2");
    await sizeField(page, "Height").fill("6000");
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toHaveText("Width must be at least 25.4 mm (72 pt). Height can be at most 5,080 mm (14,400 pt).");
    for (const name of ["Width", "Height"] as const) {
        await expect(sizeField(page, name)).toHaveAttribute("aria-invalid", "true");
        await expect(sizeField(page, name)).toHaveAttribute("aria-describedby", "size-err");
    }
    await expect(reason).toHaveText("Fix the page size to resize.");
    await expect(reason).toHaveClass(/workspace-note-error/);
    await expect(run).toBeDisabled();

    // Only the field that is out of bounds is marked.
    await sizeField(page, "Width").fill("200");
    await expect(alert).toHaveText("Height can be at most 5,080 mm (14,400 pt).");
    await expect(sizeField(page, "Width")).not.toHaveAttribute("aria-invalid", "true");
    await expect(run).toBeDisabled();

    for (const [unit, width, height, message] of [
        ["mm", "25.39", "250", "Width must be at least 25.4 mm (72 pt)."],
        ["mm", "", "250", "Width is empty."],
        ["mm", "abc", "250", "Width must be a number."],
        ["in", "0.99", "11", "Width must be at least 1 in (72 pt)."],
        ["in", "8.5", "200.01", "Height can be at most 200 in (14,400 pt)."],
        ["pt", "71.9", "792", "Width must be at least 72 pt."],
        ["pt", "612", "14400.1", "Height can be at most 14,400 pt."],
    ] as const) {
        await panel(page).getByRole("region", { name: "Custom size" }).getByLabel("Unit").selectOption(unit);
        await sizeField(page, "Width").fill(width);
        await sizeField(page, "Height").fill(height);
        await expect(alert, `${width} x ${height} ${unit}`).toHaveText(message);
        await expect(run).toBeDisabled();
    }

    // A preset needs no custom size, so it runs again.
    await panel(page).getByLabel("Page size").selectOption("letter");
    await expect(alert).toHaveCount(0);
    await expect(run).toBeEnabled();

    // The bounds themselves are allowed and sent.
    await useCustomSize(page);
    await panel(page).getByRole("region", { name: "Custom size" }).getByLabel("Unit").selectOption("mm");
    await sizeField(page, "Width").fill("25.4");
    await sizeField(page, "Height").fill("5080");
    await expect(alert).toHaveCount(0);
    await expect(reason).toHaveCount(0);
    await run.click();
    await expect.poll(async () => (await resizeRequests(page)).length).toBe(1);
    const [{ request }] = await resizeRequests(page) as Array<{ request: { size: unknown } }>;
    expect(request.size).toEqual({ custom: { width: 25.4 * (72 / 25.4), height: 5080 * (72 / 25.4) } });
});

test("actual size warns how many pages in scope are larger than the new page, and Scale to fit never does", async ({ page }) => {
    await withScenes(page, { "/local/letter.pdf": [LETTER, LETTER, A4], "/local/receipt.pdf": [SMALL] });
    await openCrop(page);
    await addPdfs(page, ["/local/letter.pdf", "/local/receipt.pdf"]);
    await useResize(page);
    const warning = page.getByTestId("resize-cut-off");
    await expect(placementRadio(page, /^Scale to fit, centered/)).toBeChecked();
    await expect(warning).toHaveCount(0);

    // A4 at actual size cuts off the two Letter pages, which are wider than A4.
    await placementRadio(page, /^Actual size, centered/).click();
    await expect(warning).toHaveAttribute("role", "status");
    await expect(warning).toHaveText("Cuts off content on 2 pages. Choose Scale to fit to keep everything.");
    await expect(page.getByRole("button", { name: "Resize 2 PDFs", exact: true })).toBeEnabled();

    // Landscape A4 is shorter than Letter and portrait A4 pages, so three pages are cut off.
    await orientationRadio(page, "Landscape").click();
    await expect(warning).toHaveText("Cuts off content on 3 pages. Choose Scale to fit to keep everything.");
    await orientationRadio(page, "Portrait").click();
    await expect(warning).toHaveText("Cuts off content on 2 pages. Choose Scale to fit to keep everything.");

    // Only pages in the typed range count, clamped to each PDF.
    await pagesField(page).fill("1");
    await expect(warning).toHaveText("Cuts off content on 1 page. Choose Scale to fit to keep everything.");
    await pagesField(page).fill("3-");
    await expect(warning).toHaveCount(0);
    await pagesField(page).fill("");

    // A custom size larger than every page cuts nothing off; a smaller one cuts off every page.
    await useCustomSize(page);
    await sizeField(page, "Width").fill("300");
    await sizeField(page, "Height").fill("400");
    await expect(warning).toHaveCount(0);
    await sizeField(page, "Height").fill("100");
    await sizeField(page, "Width").fill("100");
    await expect(warning).toHaveText("Cuts off content on 4 pages. Choose Scale to fit to keep everything.");
    // An invalid size names the size, not the cut-off.
    await sizeField(page, "Width").fill("2");
    await expect(warning).toHaveCount(0);
    await sizeField(page, "Width").fill("100");

    await placementRadio(page, /^Scale to fit, centered/).click();
    await expect(warning).toHaveCount(0);
});

type PlacementCase = { name: string; expected: { target: { width: number; height: number }; scale: number; offset: { x: number; y: number } } | null };
const placementCases: PlacementCase[] = JSON.parse(readFileSync(new URL("../../shared/resize-placement-cases.json", import.meta.url), "utf8")).cases;
const resizeStage = (page: Page) => page.getByTestId("resize-stage");
const pageSize = (page: Page) => panel(page).getByLabel("Page size");
const sizeUnit = (page: Page) => panel(page).getByRole("region", { name: "Custom size" }).getByLabel("Unit");

/** The big preview puts the page where the native resize does, as the shared cases record it. */
const expectPlaced = async (page: Page, caseName: string) => {
    const { expected } = placementCases.find(({ name }) => name === caseName)!;
    if (!expected) throw new Error(`${caseName} has no placement`);
    await expect(resizeStage(page)).toHaveAttribute("data-target", `${expected.target.width.toFixed(2)} x ${expected.target.height.toFixed(2)}`);
    await expect(resizeStage(page)).toHaveAttribute("data-scale", expected.scale.toFixed(4));
    await expect(resizeStage(page)).toHaveAttribute("data-offset", `${expected.offset.x.toFixed(2)} ${expected.offset.y.toFixed(2)}`);
};

test("the Resize preview shows the page on its new paper and follows every option without running", async ({ page }) => {
    await withScenes(page, { "/local/letter.pdf": [LETTER, LETTER_TURNED], "/local/a4.pdf": [A4], "/local/receipt.pdf": [SMALL] });
    await openCrop(page);
    await addPdfs(page, ["/local/letter.pdf", "/local/a4.pdf", "/local/receipt.pdf"]);
    await useResize(page);

    // Scale to fit on A4: the Letter page is scaled and centred, exactly where the native resize puts it.
    await expect(caption(page)).toHaveText("Letter, 216 × 279 mm on A4, 210 × 297 mm. Scaled to 97%.");
    await expectPlaced(page, "Letter page fits on A4");
    await expect(resizeStage(page)).toHaveAttribute("aria-label", "Letter page scaled to fit an A4 page, centered.");
    await expect(resizeStage(page).getByTestId("resize-cut")).toHaveCount(0);
    await expect(preview(page).getByRole("button", { name: "Undo" })).toHaveCount(0);
    const minis = preview(page).getByTestId("crop-mini");
    await expect(minis).toHaveCount(3);
    await expect(minis.nth(0)).toContainText("letter.pdf, page 2");
    await expect(minis.nth(0)).toContainText("Letter, landscape. Stays landscape. Scaled to 97%");
    await expect(minis.nth(1)).toContainText("a4.pdf");
    await expect(minis.nth(1)).toContainText("Already A4. Unchanged.");
    await expect(minis.nth(2)).toContainText("receipt.pdf");
    await expect(minis.nth(2)).toContainText("58 × 120 mm. Scaled up to 248%");
    await expect(preview(page)).toContainText("Each page keeps its orientation and is scaled to fit, so nothing is cut off.");

    // The other pages land where the shared cases say.
    await preview(page).getByRole("button", { name: "Next page in range" }).click();
    await expect(caption(page)).toHaveText("Letter, landscape, 279 × 216 mm on A4, landscape, 297 × 210 mm. Scaled to 97%.");
    await expectPlaced(page, "Landscape Letter page fits on landscape A4");
    await preview(page).getByLabel("PDF to preview").selectOption({ label: "receipt.pdf" });
    await expect(caption(page)).toHaveText("58 × 120 mm on A4, 210 × 297 mm. Scaled up to 248%.");
    await expectPlaced(page, "Small receipt is scaled up to A4");
    await preview(page).getByLabel("PDF to preview").selectOption({ label: "a4.pdf" });
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm. Already A4. Unchanged.");
    await expect(resizeStage(page)).not.toHaveAttribute("data-target");
    await expect(resizeStage(page)).toHaveAttribute("aria-label", "Page 1, A4, unchanged.");

    // Orientation and preset change the preview at once.
    await orientationRadio(page, "Landscape").click();
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm on A4, landscape, 297 × 210 mm. Scaled to 71%.");
    await expectPlaced(page, "A4 page forced onto landscape A4");
    await expect(preview(page)).toContainText("Every page gets landscape paper and is scaled to fit, so nothing is cut off.");
    await orientationRadio(page, "Match page").click();
    await pageSize(page).selectOption("letter");
    await expect(caption(page)).toHaveText("A4, 210 × 297 mm on Letter, 216 × 279 mm. Scaled to 94%.");
    await pageSize(page).selectOption("a4");
    await preview(page).getByLabel("PDF to preview").selectOption({ label: "letter.pdf" });

    // Actual size: the page keeps its scale and what falls outside the new page is hatched and named.
    await placementRadio(page, /^Actual size, centered/).click();
    await expect(caption(page)).toHaveText("Letter, 216 × 279 mm on A4, 210 × 297 mm. 6 mm is cut off the sides.");
    await expect(caption(page)).toHaveClass(/workspace-note-warning/);
    await expect(resizeStage(page)).toHaveAttribute("data-scale", "1.0000");
    await expect(resizeStage(page).getByTestId("resize-cut")).toHaveCount(2);
    await expect(resizeStage(page)).toHaveAttribute("aria-label", "Letter page placed at actual size on an A4 page. Hatched areas are cut off.");
    await expect(minis.nth(0)).toContainText("Content cut off");
    await expect(minis.nth(1)).toContainText("Already A4. Unchanged.");
    await expect(minis.nth(2)).toContainText("58 × 120 mm. Fits. Nothing is cut off.");
    await expect(preview(page)).toContainText("Hatched areas are outside the new page and will be cut off.");

    // A custom size, in the chosen unit; an unreadable size shows the page unchanged.
    await useCustomSize(page);
    await expect(caption(page)).toHaveText("Letter, 216 × 279 mm on 200 × 250 mm. 16 mm is cut off the sides and 29 mm off the top and bottom.");
    await expectPlaced(page, "Letter page at actual size on a smaller custom page is cut on every side");
    await expect(resizeStage(page).getByTestId("resize-cut")).toHaveCount(4);
    await sizeUnit(page).selectOption("in");
    await expect(caption(page)).toHaveText("Fix the page size to see the result.");
    await expect(caption(page)).toHaveClass(/workspace-note-error/);
    await expect(resizeStage(page)).not.toHaveAttribute("data-target");
    await expect(resizeStage(page)).toHaveAttribute("aria-label", "Page 1, Letter, unchanged.");
    await sizeField(page, "Width").fill("8");
    await sizeField(page, "Height").fill("10");
    await expect(caption(page)).toHaveText("Letter, 8.5 × 11 in on 8 × 10 in. 0.5 in is cut off the sides and 1 in off the top and bottom.");
    await sizeField(page, "Width").fill("9");
    await sizeField(page, "Height").fill("12");
    await expect(caption(page)).toHaveText("Letter, 8.5 × 11 in on 9 × 12 in. Fits. Nothing is cut off.");
    await expect(resizeStage(page).getByTestId("resize-cut")).toHaveCount(0);
    await placementRadio(page, /^Scale to fit, centered/).click();
    await expect(caption(page)).toHaveText("Letter, 8.5 × 11 in on 9 × 12 in. Scaled up to 106%.");

    expect(await resizeRequests(page)).toHaveLength(0);
});

test("a PDF whose pages in scope already have the new size is marked to be skipped", async ({ page }) => {
    await withScenes(page, { "/local/a4.pdf": [A4, LETTER], "/local/letter.pdf": [LETTER] });
    await openCrop(page);
    await addPdfs(page, ["/local/a4.pdf", "/local/letter.pdf"]);
    await useResize(page);
    const noted = page.locator("li").filter({ has: page.getByTestId("resize-file-note") });
    await expect(noted).toHaveCount(0);

    await pagesField(page).fill("1");
    await expect(noted).toHaveCount(1);
    await expect(noted).toContainText("a4.pdf");
    await expect(noted.getByTestId("resize-file-note")).toHaveText("Already A4. Will be skipped.");

    await pageSize(page).selectOption("letter");
    await expect(noted).toHaveCount(1);
    await expect(noted).toContainText("letter.pdf");
    await expect(noted.getByTestId("resize-file-note")).toHaveText("Already Letter. Will be skipped.");

    await pagesField(page).fill("");
    await expect(noted).toHaveCount(1);
    await expect(noted).toContainText("letter.pdf");
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Crop pages" }).click();
    await expect(page.getByTestId("resize-file-note")).toHaveCount(0);
});
