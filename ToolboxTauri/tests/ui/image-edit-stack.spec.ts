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
                    const edits = (args as { request: { plan: { edits: Array<{ kind: string; mode: string; width: number; height: number; longestSide: number }> } } }).request.plan.edits;
                    const resize = edits.find((edit) => edit.kind === "resize");
                    if (resize?.mode === "longestSide") return { width: resize.longestSide, height: Math.round(resize.longestSide * 0.75), dataUrl: blank };
                    const crop = edits.find((edit) => edit.kind === "crop");
                    if (crop?.mode === "rectangle") return { width: crop.width, height: crop.height, dataUrl: blank };
                    return { width: 384, height: 480, dataUrl: blank };
                }
                if (command.startsWith("plugin:")) return null;
                const request = (args as { request?: { paths?: string[]; steps?: Record<string, unknown> } }).request;
                const stepNames = Object.keys(request?.steps ?? {});
                const suffix = stepNames.length > 1 ? "-edited" : ({ resize: "-resized", rotate: "-rotated" } as Record<string, string>)[stepNames[0]] ?? "-cropped";
                return win.__processingResults ?? (request?.paths ?? []).map((inputPath) => ({
                    inputPath,
                    outputPaths: [inputPath.replace(/(\.\w+)$/, `${suffix}$1`)],
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

const openTool = async (page: Page, tool: string, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, tool);
    await page.evaluate((paths) => { (window as unknown as StackWindow).__dialogResults = [paths]; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(`Previewing ${paths[0].split("/").pop()}`, { exact: true })).toBeVisible();
};

const openCrop = (page: Page, paths: string[]) => openTool(page, "Crop Images", paths);
const openResize = (page: Page, paths: string[]) => openTool(page, "Resize Images", paths);
const openRotate = (page: Page, paths: string[]) => openTool(page, "Rotate and Flip Images", paths);
const addedMessage = (label: string, position: number) =>
    `${label} added as step ${position}. Steps always run Crop, Resize, Rotate and Flip, Colour and Tone, Watermark, then Remove Metadata.`;

const steps = (page: Page) => page.getByRole("list", { name: "Steps, in the order they run" });

test("choosing Crop adds one step and runs the batch through one export", async ({ page }) => {
    const paths = ["/photos/a.png", "/photos/b.jpg", "/photos/c.webp"];
    await openCrop(page, paths);

    await expect(steps(page).getByRole("button", { name: "Step 1 of 1, Crop, showing its options" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("heading", { name: "Crop", exact: true })).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Crop", 1) })).toBeVisible();

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
    await expect(page.getByRole("region", { name: "Result preview" }).getByText("300 × 200px")).toBeVisible();

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

const runRequest = async (page: Page) => {
    const [run] = await invocations(page, "export_image_edit_stack");
    return (run.args as { request: { paths: string[]; steps: Record<string, unknown> } }).request;
};

test("choosing Resize adds a step whose longest side, resampling and aspect lock reach the request", async ({ page }) => {
    await openResize(page, ["/photos/a.png", "/photos/b.jpg"]);
    await expect(steps(page).getByRole("button", { name: "Step 1 of 1, Resize, showing its options" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Resize", 1) })).toBeVisible();
    await expect(page.getByRole("radio", { name: "Longest side" })).toHaveAttribute("aria-checked", "true");

    await page.getByLabel("Longest side, pixels").fill("1200");
    await page.getByLabel("Resampling").selectOption({ label: "Sharp, keeps fine detail" });
    await expect(page.getByRole("region", { name: "Result preview" }).getByText("1200 × 900px")).toBeVisible();
    await expect(page.getByTestId("run-destination-example")).toHaveText("a-resized.png");

    await page.getByRole("button", { name: "Run on 2 images" }).click();
    const request = await runRequest(page);
    expect(request.steps).toEqual({
        resize: { mode: "longestSide", longestSide: 1200, width: 1600, height: 1200, percentage: 50, resampling: "bicubic", keepAspectRatio: true },
    });
    await expect(page.getByRole("button", { name: "Open a-resized.png" })).toBeVisible();
});

test("an exact size keeps the aspect lock on until it is turned off", async ({ page }) => {
    await openResize(page, ["/photos/a.png"]);
    await page.getByRole("radio", { name: "Exact size" }).click();
    const lock = page.getByRole("button", { name: "Keep aspect ratio" });
    await expect(lock).toHaveAttribute("aria-pressed", "true");
    await page.getByLabel("Width", { exact: true }).fill("800");
    await page.getByLabel("Height", { exact: true }).fill("600");
    await expect(page.getByText("Keep aspect ratio is on: each image fits inside 800 × 600 and keeps its shape.", { exact: false })).toBeVisible();
    await lock.click();
    await expect(lock).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByText("Keep aspect ratio is off: every image becomes exactly 800 × 600", { exact: false })).toBeVisible();

    await page.getByRole("button", { name: "Run on 1 image" }).click();
    expect((await runRequest(page)).steps).toMatchObject({ resize: { mode: "exact", width: 800, height: 600, keepAspectRatio: false } });
});

test("a value outside the limits blocks Run and names the field", async ({ page }) => {
    await openResize(page, ["/photos/a.png"]);
    await page.getByRole("radio", { name: "Percentage" }).click();
    const percentage = page.getByLabel("Percentage", { exact: true });
    await percentage.fill("1500");
    await expect(percentage).toHaveAttribute("aria-invalid", "true");
    await expect(page.getByText("Enter a percentage from 1 to 1000.", { exact: true })).toBeVisible();
    await expect(page.getByText("Fix the percentage to run", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeDisabled();

    await page.getByRole("radio", { name: "Longest side" }).click();
    await page.getByLabel("Longest side, pixels").fill("16385");
    await expect(page.getByText("Enter a longest side from 1 to 16384 pixels.", { exact: true })).toBeVisible();
    await page.getByLabel("Longest side, pixels").fill("16384");
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeEnabled();
});

test("a second tool adds a second step and steps run in a fixed order whatever order they were added", async ({ page }) => {
    await openResize(page, ["/photos/a.png"]);
    await page.getByLabel("Longest side, pixels").fill("640");
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();

    await expect(page.getByRole("status").filter({ hasText: addedMessage("Crop", 1) })).toBeVisible();
    const list = steps(page);
    await expect(list.getByRole("button", { name: "Step 1 of 2, Crop, showing its options" })).toHaveAttribute("aria-pressed", "true");
    await expect(list.getByRole("button", { name: "Step 2 of 2, Resize" })).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByText("Step 1 of 2. Runs first, before Resize.", { exact: true })).toBeVisible();
    await expect(page.getByTestId("run-destination-example")).toHaveText("a-edited.png");
    await expect(page.getByRole("region", { name: "Result preview" }).getByText("After all 2 steps")).toBeVisible();

    // Focusing a step shows its options without adding another.
    await list.getByRole("button", { name: "Step 2 of 2, Resize" }).click();
    await expect(page.getByLabel("Longest side, pixels")).toHaveValue("640");
    await expect(page.getByText("Step 2 of 2. Runs after Crop.", { exact: true })).toBeVisible();
    await expect(list.getByRole("button", { name: /^Step \d of 2/ })).toHaveCount(2);

    await page.getByRole("button", { name: "Run on 1 image" }).click();
    const request = await runRequest(page);
    expect(Object.keys(request.steps)).toEqual(["crop", "resize"]);
    expect(request.steps).toMatchObject({ crop: { mode: "aspectRatio" }, resize: { mode: "longestSide", longestSide: 640 } });
    await expect(page.getByRole("button", { name: "Open a-edited.png" })).toBeVisible();
});

test("adding Crop from the Resize step list sends the same request as adding Resize to Crop", async ({ page }) => {
    await openCrop(page, ["/photos/a.png"]);
    await steps(page).getByRole("button", { name: "Add Resize step" }).click();
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Resize", 2) })).toBeVisible();
    await expect(steps(page).getByRole("button", { name: "Step 2 of 2, Resize, showing its options" })).toBeVisible();
    await page.getByRole("button", { name: "Run on 1 image" }).click();
    const cropFirst = await runRequest(page);

    await openResize(page, ["/photos/a.png"]);
    await steps(page).getByRole("button", { name: "Add Crop step" }).click();
    await page.getByRole("button", { name: "Run on 1 image" }).click();
    const resizeFirst = await runRequest(page);

    expect(JSON.stringify(resizeFirst.steps)).toBe(JSON.stringify(cropFirst.steps));
    expect(Object.keys(resizeFirst.steps)).toEqual(["crop", "resize"]);
});

for (const theme of ["light", "dark"] as const) {
    test(`the resize step renders readable controls in the ${theme} theme`, async ({ page }, testInfo) => {
        await page.addInitScript((theme) => { localStorage.setItem("toolbox-theme", theme); }, theme);
        await openResize(page, ["/photos/a.png"]);
        await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
        await page.getByRole("radio", { name: "Exact size" }).click();
        const options = page.getByRole("region", { name: "Options" });
        await expect(options.getByRole("button", { name: "Keep aspect ratio" })).toBeVisible();
        const colors = await options.evaluate((node) => {
            const heading = node.querySelector("h2");
            const separator = document.querySelector(".image-edit-stack-separator");
            return {
                surface: getComputedStyle(node).backgroundColor,
                heading: heading ? getComputedStyle(heading).color : "",
                separatorHidden: separator?.getAttribute("aria-hidden"),
            };
        });
        expect(colors.heading).not.toBe("");
        expect(colors.heading).not.toBe(colors.surface);
        await page.screenshot({ path: testInfo.outputPath(`resize-step-${theme}.png`), animations: "disabled" });
        await steps(page).getByRole("button", { name: "Add Crop step" }).click();
        const separators = page.locator(".image-edit-stack-separator");
        await expect(separators).toHaveCount(2);
        for (const separator of await separators.all()) await expect(separator).toHaveAttribute("aria-hidden", "true");
    });
}

const reEncoded = "Re-encoded at quality 90 after rotating.";

test("choosing Rotate and Flip sets the turn and the mirror in one step and the row shows the re-encode", async ({ page }) => {
    await openRotate(page, ["/photos/a.jpg", "/photos/b.png"]);
    await expect(steps(page).getByRole("button", { name: "Step 1 of 1, Rotate and Flip, showing its options" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Rotate and Flip", 1) })).toBeVisible();
    await expect(page.getByRole("radio", { name: "90 degrees" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByRole("radio", { name: "None" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText("Clockwise, as you see the image in the preview.", { exact: true })).toBeVisible();
    await expect(page.getByText("Rotated images are always re-encoded at quality 90, even with no other changes.", { exact: true })).toBeVisible();

    await page.getByRole("radio", { name: "270 degrees" }).click();
    await page.getByRole("radio", { name: "Horizontal" }).click();
    await expect(page.getByRole("radio", { name: "270 degrees" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByTestId("run-destination-example")).toHaveText("a-rotated.jpg");
    const previews = await invocations(page, "inspect_image_edit_preview");
    expect((previews.at(-1)?.args as { request: { plan: { edits: unknown[] } } }).request.plan.edits).toEqual([{ kind: "rotate", degrees: 270, flip: "horizontal" }]);

    await page.evaluate((detail) => {
        (window as unknown as StackWindow).__processingResults = [
            { inputPath: "/photos/a.jpg", outputPaths: ["/photos/a-rotated.jpg"], detail, state: "succeeded", failure: null, reason: null, warnings: [] },
            { inputPath: "/photos/b.png", outputPaths: ["/photos/b-rotated.png"], detail: "Image saved", state: "succeeded", failure: null, reason: null, warnings: [] },
        ] as JobOutcome[];
    }, reEncoded);
    await page.getByRole("button", { name: "Run on 2 images" }).click();
    const request = await runRequest(page);
    expect(request.steps).toEqual({ rotate: { degrees: 270, flip: "horizontal" } });
    expect(await invocations(page, "rotate_images")).toEqual([]);

    const rows = page.locator(".result-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText(reEncoded);
    await expect(rows.nth(0)).not.toContainText(/warning/i);
    await expect(rows.nth(1)).not.toContainText(reEncoded);
    await expect(page.getByRole("button", { name: "Open a-rotated.jpg" })).toBeVisible();
});

test("Rotate and Flip always runs after Crop and Resize and the run is named -edited", async ({ page }) => {
    await openRotate(page, ["/photos/a.jpg"]);
    await page.getByRole("radio", { name: "180 degrees" }).click();
    await page.getByRole("radio", { name: "Vertical" }).click();
    await steps(page).getByRole("button", { name: "Add Crop step" }).click();
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Crop", 1) })).toBeVisible();
    await steps(page).getByRole("button", { name: "Add Resize step" }).click();
    await expect(page.getByRole("status").filter({ hasText: addedMessage("Resize", 2) })).toBeVisible();

    const list = steps(page);
    await list.getByRole("button", { name: "Step 3 of 3, Rotate and Flip" }).click();
    await expect(page.getByText("Step 3 of 3. Runs after Crop and Resize.", { exact: true })).toBeVisible();
    await expect(page.getByRole("radio", { name: "180 degrees" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByRole("radio", { name: "Vertical" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByTestId("run-destination-example")).toHaveText("a-edited.jpg");

    await page.getByRole("button", { name: "Run on 1 image" }).click();
    const request = await runRequest(page);
    expect(Object.keys(request.steps)).toEqual(["crop", "resize", "rotate"]);
    expect(request.steps).toMatchObject({ rotate: { degrees: 180, flip: "vertical" } });
    await expect(page.getByRole("button", { name: "Open a-edited.jpg" })).toBeVisible();
});

for (const theme of ["light", "dark"] as const) {
    test(`the Rotate and Flip step renders readable controls in the ${theme} theme`, async ({ page }, testInfo) => {
        await page.addInitScript((theme) => { localStorage.setItem("toolbox-theme", theme); }, theme);
        await openRotate(page, ["/photos/a.jpg"]);
        await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
        const options = page.getByRole("region", { name: "Options" });
        await expect(options.getByRole("radio", { name: "90 degrees" })).toBeVisible();
        const colors = await options.evaluate((node) => {
            const legend = node.querySelector("legend");
            const note = Array.from(node.querySelectorAll("p")).find((p) => p.textContent?.startsWith("Rotated images"));
            return {
                surface: getComputedStyle(node).backgroundColor,
                legend: legend ? getComputedStyle(legend).color : "",
                note: note ? getComputedStyle(note).color : "",
            };
        });
        expect(colors.legend).not.toBe("");
        expect(colors.legend).not.toBe(colors.surface);
        expect(colors.note).not.toBe("");
        expect(colors.note).not.toBe(colors.surface);
        await options.screenshot({ path: testInfo.outputPath(`rotate-step-${theme}.png`), animations: "disabled" });
    });
}
