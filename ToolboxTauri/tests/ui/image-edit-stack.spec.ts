import { expect, test, type Page } from "@playwright/test";
import type { JobOutcome } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";

type StackWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[]>;
    __processingResults?: JobOutcome[];
};

const svg = (width: number, height: number) => `data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='${width}' height='${height}'/%3E`;

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ blank }) => {
        const win = window as unknown as StackWindow;
        win.__invocations = [];
        win.__dialogResults = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "get_quick_actions") return [];
                if (command === "plugin:app|version") return "9.4.2";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: blank };
                if (command === "inspect_image_edit_preview") {
                    const crop = (args as { request: { plan: { edits: Array<{ mode: string; width: number; height: number }> } } }).request.plan.edits[0];
                    if (crop.mode === "rectangle") return { width: crop.width, height: crop.height, dataUrl: blank };
                    return { width: 384, height: 480, dataUrl: blank };
                }
                if (command.startsWith("plugin:")) return null;
                const request = (args as { request?: { paths?: string[] } }).request;
                return win.__processingResults ?? (request?.paths ?? []).map((inputPath) => ({
                    inputPath,
                    outputPaths: [inputPath.replace(/(\.\w+)$/, "-cropped$1")],
                    detail: "Image saved",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: [],
                }));
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, { blank: svg(640, 480) });
});

const invocations = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as StackWindow).__invocations.filter((invocation) => invocation.command === command), command);

const openCrop = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, "Crop Images");
    await page.evaluate((paths) => { (window as unknown as StackWindow).__dialogResults = [paths]; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(`Previewing ${paths[0].split("/").pop()}`, { exact: true })).toBeVisible();
};

const steps = (page: Page) => page.getByRole("list", { name: "Steps, in the order they run" });

test("choosing Crop adds one step and runs the batch through one export", async ({ page }) => {
    const paths = ["/photos/a.png", "/photos/b.jpg", "/photos/c.webp"];
    await openCrop(page, paths);

    await expect(steps(page).getByRole("button", { name: "Step 1 of 1, Crop, showing its options" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("heading", { name: "Crop", exact: true })).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: "Crop added as step 1. Steps always run in the same order." })).toBeVisible();

    // Choosing Crop again focuses the existing step instead of adding a second one.
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await expect(steps(page).getByRole("button", { name: /Crop, showing its options/ })).toHaveCount(1);

    await expect(page.getByText("Originals stay as they are", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Run on 3 images" }).click();

    const [run] = await invocations(page, "export_image_edit_stack");
    expect(run.args).toMatchObject({
        request: {
            paths,
            steps: { crop: { mode: "aspectRatio", aspectWidth: 4, aspectHeight: 5, anchor: "center", x: 0, y: 0, width: 0, height: 0 } },
            outputLocation: "alongsideInput",
        },
    });
    await expect(page.locator(".result-row")).toHaveCount(3);
    await expect(page.getByRole("button", { name: "Open a-cropped.png" })).toBeVisible();
});

test("a pixel rectangle is sent exactly as typed", async ({ page }) => {
    await openCrop(page, ["/photos/a.png"]);
    await page.getByRole("radio", { name: "Pixel rectangle" }).click();
    await page.getByLabel("Left edge, pixels").fill("10");
    await page.getByLabel("Top edge, pixels").fill("20");
    await page.getByLabel("Width, pixels").fill("300");
    await page.getByLabel("Height, pixels").fill("200");
    await expect(page.getByRole("region", { name: "Crop result preview" }).getByText("300 × 200px")).toBeVisible();

    await page.getByRole("button", { name: "Run on 1 image" }).click();
    const [run] = await invocations(page, "export_image_edit_stack");
    expect((run.args as { request: { steps: unknown } }).request.steps).toEqual({
        crop: { mode: "rectangle", aspectWidth: 4, aspectHeight: 5, anchor: "center", x: 10, y: 20, width: 300, height: 200 },
    });
});

test("Previous and Next change which image is previewed", async ({ page }) => {
    await openCrop(page, ["/photos/a.png", "/photos/b.jpg"]);
    await expect(page.getByText("1 of 2", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Previous image" })).toBeDisabled();

    await page.getByRole("button", { name: "Next image" }).click();
    await expect(page.getByText("Previewing b.jpg", { exact: true })).toBeVisible();
    await expect(page.getByText("2 of 2", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Next image" })).toBeDisabled();
    const previews = await invocations(page, "inspect_image_preview");
    expect((previews.at(-1)?.args as { request: { path: string } }).request.path).toBe("/photos/b.jpg");

    await page.getByRole("button", { name: "Previous image" }).click();
    await expect(page.getByText("Previewing a.png", { exact: true })).toBeVisible();
});

test("one failed image is reported on its own row while the others save", async ({ page }) => {
    await openCrop(page, ["/photos/a.png", "/photos/broken.png"]);
    await page.evaluate(() => {
        (window as unknown as StackWindow).__processingResults = [
            { inputPath: "/photos/a.png", outputPaths: ["/photos/a-cropped.png"], detail: "Image saved", state: "succeeded", failure: null, reason: null, warnings: [] },
            { inputPath: "/photos/broken.png", outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: "This image could not be read.", pointer: null }, reason: null, warnings: [] },
        ] as JobOutcome[];
    });
    await page.getByRole("button", { name: "Run on 2 images" }).click();

    const rows = page.locator(".result-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0).getByRole("button", { name: "Open a-cropped.png" })).toBeVisible();
    await expect(rows.nth(0).getByRole("button", { name: /^Show in Finder/ })).toBeVisible();
    await expect(rows.nth(1)).toContainText("This image could not be read.");
    await expect(rows.nth(1).getByRole("button", { name: /^Open / })).toHaveCount(0);
    await expect(rows.nth(1).getByRole("button", { name: /^Show in Finder/ })).toHaveCount(0);
});

test("remove, reset, undo and redo keep the stack and its message in step", async ({ page }) => {
    await openCrop(page, ["/photos/a.png"]);
    const toolbar = page.getByRole("toolbar", { name: "Edit stack" });
    const undo = toolbar.getByRole("button", { name: "Undo, ⌘Z" });
    const redo = toolbar.getByRole("button", { name: "Redo, ⇧⌘Z" });
    await expect(undo).toBeEnabled();
    await expect(redo).toBeDisabled();

    await page.getByRole("radio", { name: "1:1" }).click();
    await toolbar.getByRole("button", { name: "Remove Crop step" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Crop removed. The other steps keep their settings." })).toBeVisible();
    await expect(page.getByRole("note").filter({ hasText: "No steps in the stack" })).toBeVisible();
    await expect(page.getByText("Add a step to run", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeDisabled();
    await expect(toolbar.getByRole("button", { name: "Reset stack" })).toBeDisabled();

    // The Undo in the message restores the step with its settings.
    await page.getByRole("status").getByRole("button", { name: "Undo" }).click();
    await expect(page.getByRole("radio", { name: "1:1" })).toHaveAttribute("aria-checked", "true");
    await expect(redo).toBeEnabled();
    await redo.click();
    await expect(toolbar.getByRole("button", { name: "Add Crop step" })).toBeVisible();
    await undo.click();

    await toolbar.getByRole("button", { name: "Reset stack" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Stack reset. 1 step removed. Your images are still in the list." })).toBeVisible();
    await expect(page.getByText("Previewing a.png", { exact: true })).toBeVisible();
    await page.getByRole("status").getByRole("button", { name: "Undo" }).click();
    await expect(page.getByRole("radio", { name: "1:1" })).toHaveAttribute("aria-checked", "true");

    await toolbar.getByRole("button", { name: "Remove Crop step" }).click();
    await toolbar.getByRole("button", { name: "Add Crop step" }).click();
    await expect(page.getByRole("radio", { name: "4:5" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeEnabled();
});

test("Run waits for images before it can start", async ({ page }) => {
    await page.goto("/");
    await openToolBySearch(page, "Crop Images");
    await expect(page.getByText("Add images to run", { exact: true })).toBeVisible();
    await expect(page.locator(".workspace-primary-action")).toBeDisabled();
    await expect(page.locator(".workspace-primary-action")).toContainText("Run");
});
