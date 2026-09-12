import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { UtilityRegistry, workspaceForTool } from "../../src/registry";

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const fixtureName = path.basename(fixturePath);
const mockedOutputPaths = [`${fixturePath}.output-one`, `${fixturePath}.output-two`];

type MockOutcome = {
    inputPath: string;
    outputPaths: string[];
    detail: string;
    failure: { kind: string; message: string } | null;
};

type TestWindow = Window & {
    __toolboxInvocations?: Array<{ command: string; args: unknown }>;
    __toolboxProcessingResults?: MockOutcome[];
    __toolboxProcessingDelayMs?: number;
    __toolboxInspectionDelayMs?: number;
    __toolboxInspectionError?: string;
    __toolboxActionFailure?: { command: string; path: string; message: string; delayMs?: number };
    __toolboxSecondPick?: boolean;
    __toolboxDialogResults?: Array<string | string[] | null>;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ fixturePath, mockedOutputPaths }) => {
        const invocations: Array<{ command: string; args: unknown }> = [];
        (window as TestWindow).__toolboxInvocations = invocations;
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });

        window.__TAURI_INTERNALS__ = {
            metadata: {
                currentWindow: { label: "main" },
                currentWebview: { label: "main", windowLabel: "main" },
            },
            invoke: async (command, args) => {
                invocations.push({ command, args });

                if (command === "plugin:dialog|open") {
                    const dialogResults = (window as TestWindow).__toolboxDialogResults;
                    if (dialogResults?.length) return dialogResults.shift() ?? null;
                    if ((window as TestWindow).__toolboxSecondPick) {
                        delete (window as TestWindow).__toolboxSecondPick;
                        return `${fixturePath}.second.pdf`;
                    }
                    return fixturePath;
                }
                if (command === "open_output_path" || command === "reveal_output_path") {
                    const failure = (window as TestWindow).__toolboxActionFailure;
                    if (failure?.command === command && failure.path === (args as { path: string }).path) {
                        if (failure.delayMs) {
                            await new Promise((resolve) => window.setTimeout(resolve, failure.delayMs));
                        }
                        throw failure.message;
                    }
                    return null;
                }
if (command === "preview_pdf_scene") return { dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'/%3E", width: 612, height: 792 };
                if (command === "inspect_pdf" || command === "inspect_pdf_scene") {
                    const inspectionDelay = (window as TestWindow).__toolboxInspectionDelayMs;
                    if (inspectionDelay) await new Promise((resolve) => window.setTimeout(resolve, inspectionDelay));
                    const requestPath = (args as { request: { path: string } }).request.path;
                    const inspectionError = (window as TestWindow).__toolboxInspectionError;
                    if (inspectionError && requestPath.endsWith(".rejected.pdf")) throw inspectionError;
                    const pageCount = requestPath.endsWith(".replacement.pdf") ? 2 : requestPath.endsWith(".scoped.pdf") ? 3 : 1;
                    return { pages: Array.from({ length: pageCount }, (_, index) => ({ index, x: 0, y: 0, width: 612, height: 792, preview: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" })) };
                }
                if (command === "inspect_image_preview") {
                    return { width: 640, height: 480, dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" };
                }
                if (command === "inspect_tiff_pages") {
                    const requestPath = (args as { request: { path: string } }).request.path;
                    const pageCount = requestPath.endsWith("two-page.tiff") ? 2 : 1;
                    return Array.from({ length: pageCount }, (_, page) => ({
                        width: 640,
                        height: 480,
                        dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E",
                    }));
                }
                if (command === "inspect_image_edit_preview") {
                    const edits = (args as { request?: { plan?: { edits?: Array<{ kind: string; width?: number; height?: number; mode?: string; aspectWidth?: number; aspectHeight?: number; anchor?: string }> } } }).request?.plan?.edits ?? [];
                    const crop = [...edits].reverse().find((edit) => edit.kind === "crop");
                    const sourceWidth = 640;
                    const sourceHeight = 480;
                    if (crop?.mode === "rectangle" && (!crop.width || !crop.height)) throw "Crop rectangle must have positive dimensions.";
                    if (crop?.mode === "aspectRatio" && (!crop.aspectWidth || !crop.aspectHeight)) throw "Aspect ratio dimensions must be positive.";
                    let width = crop?.width ?? sourceWidth;
                    let height = crop?.height ?? sourceHeight;
                    if (crop?.mode === "aspectRatio" && crop.aspectWidth && crop.aspectHeight) {
                        const ratio = crop.aspectWidth / crop.aspectHeight;
                        if (sourceWidth / sourceHeight > ratio) {
                            height = sourceHeight;
                            width = Math.round(height * ratio);
                        } else {
                            width = sourceWidth;
                            height = Math.round(width / ratio);
                        }
                    }
                    return { width, height, dataUrl: `data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='${width}' height='${height}'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E` };
                }
                if (command === "inspect_image_metadata") return ["fixture image"];
                if (command.startsWith("plugin:")) return null;

                const processingDelay = (window as TestWindow).__toolboxProcessingDelayMs;
                if (processingDelay) await new Promise((resolve) => window.setTimeout(resolve, processingDelay));

                return (window as TestWindow).__toolboxProcessingResults ?? [{
                    inputPath: fixturePath,
                    outputPaths: mockedOutputPaths,
                    detail: "Test output",
                    failure: null,
                }];
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
            unregisterListener: () => undefined,
        };
    }, { fixturePath, mockedOutputPaths });
});

test("every registered feature opens its detail pane", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Ready to process" })).toBeVisible();

    for (const [index, utility] of UtilityRegistry.entries()) {
        if (index > 0) {
            await page.getByRole("button", { name: "← All tools" }).click();
        }

        const navigationButton = page.getByRole("button", {
            name: `Open ${utility.title}`,
        });

        await expect(navigationButton).toBeVisible();
        await navigationButton.click();
        if (utility.status === "unavailable") {
            await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
        } else {
            await expect(page.getByRole("heading", { name: workspaceForTool(utility.id)?.title, exact: true })).toBeVisible();
        }
        await expect(page.locator('p[role="status"]')).toHaveText(`${workspaceForTool(utility.id)?.title ?? utility.title} workspace open.`);
    }
});

test("sidebar quick access pairs design icons with labels", async ({ page }) => {
    await page.goto("/");
    const quickAccess = page.locator("nav.quick-tools");
    await expect(quickAccess).toBeVisible();
    await expect(quickAccess.getByRole("button")).toHaveCount(4);
    await expect(quickAccess.locator('[aria-hidden="true"]')).toHaveCount(4);

    const iconMetrics = await quickAccess.locator(".quick-tool-icon").first().evaluate((node) => {
        const icon = node.querySelector("span");
        const box = node.getBoundingClientRect();
        const glyph = icon?.getBoundingClientRect();
        return {
            color: getComputedStyle(node).color,
            boxWidth: box.width,
            boxHeight: box.height,
            glyphWidth: glyph?.width ?? 0,
            glyphHeight: glyph?.height ?? 0,
        };
    });

    expect(iconMetrics.color).not.toBe("rgb(153, 153, 153)");
    expect(iconMetrics.boxWidth).toBeGreaterThanOrEqual(26);
    expect(iconMetrics.boxHeight).toBeGreaterThanOrEqual(26);
    expect(iconMetrics.glyphWidth).toBeGreaterThanOrEqual(15);
    expect(iconMetrics.glyphHeight).toBeGreaterThanOrEqual(15);
});

test("settings highlight matches navigation items", async ({ page }) => {
    await page.goto("/");
    const settings = page.getByRole("button", { name: "Settings", exact: true });
    const allTools = page.locator('nav[aria-label="Workspace navigation"]').getByRole("button", {
        name: "All tools",
    });

    const settingsStyle = await settings.evaluate((node) => {
        const style = getComputedStyle(node);
        return {
            borderRadius: style.borderRadius,
            borderTopWidth: style.borderTopWidth,
        };
    });
    const allToolsStyle = await allTools.evaluate((node) => {
        const style = getComputedStyle(node);
        return {
            borderRadius: style.borderRadius,
            borderTopWidth: style.borderTopWidth,
        };
    });

    expect(settingsStyle.borderRadius).toBe(allToolsStyle.borderRadius);
    expect(settingsStyle.borderTopWidth).toBe("0px");

    await settings.click();
    await expect(settings).toHaveAttribute("aria-current", "page");
});

test("file upload surface follows the selected theme", async ({ page }) => {
    await page.goto("/");
    const utility = UtilityRegistry[0];
    await page.getByRole("button", { name: `Open ${utility.title}` }).click();

    const sourceBar = page.getByRole("region", { name: "Open document" });
    const readSurface = () => sourceBar.evaluate((node) => ({
        background: getComputedStyle(node).backgroundColor,
        border: getComputedStyle(node).borderTopColor,
        copy: getComputedStyle(node.querySelector(".workspace-source-copy > span:last-child")!).color,
    }));

    const settings = page.getByRole("button", { name: "Settings", exact: true });
    await settings.click();
    const dialog = page.getByRole("dialog", { name: "Settings" });
    await dialog.getByRole("button", { name: "Dark" }).click();
    await expect.poll(() => page.locator("body").getAttribute("data-theme")).toBe("dark");
    await page.mouse.move(0, 0);
    const darkSurface = await readSurface();
    await dialog.getByRole("button", { name: "Light" }).click();
    await expect.poll(() => page.locator("body").getAttribute("data-theme")).toBe("light");
    await page.mouse.move(0, 0);
    await expect.poll(async () => (await readSurface()).background).not.toBe(darkSurface.background);

    const lightSurface = await readSurface();
    expect(lightSurface.background).not.toBe(darkSurface.background);
    expect(lightSurface.border).not.toBe(darkSurface.border);
    expect(lightSurface.copy).not.toBe(darkSurface.copy);

    await dialog.getByRole("button", { name: "Dark" }).click();
    await page.mouse.move(0, 0);
    await expect.poll(readSurface).toEqual(darkSurface);
});

test("favorites and recent navigation show their intended libraries", async ({ page }) => {
    await page.goto("/");
    const workspaceNav = page.locator('nav[aria-label="Workspace navigation"]');

    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.getByRole("heading", { name: "Favorites", exact: true })).toBeVisible();
    await expect(page.getByText("No favorite tools yet", { exact: true })).toBeVisible();
    await expect(workspaceNav.getByRole("button", { name: "Favorites" })).toHaveAttribute("aria-current", "page");

    await workspaceNav.getByRole("button", { name: "Recent" }).click();
    await expect(page.getByRole("heading", { name: "Recent", exact: true })).toBeVisible();
    await expect(page.getByText("No recent tools yet", { exact: true })).toBeVisible();
    await expect(workspaceNav.getByRole("button", { name: "Recent" })).toHaveAttribute("aria-current", "page");

    await workspaceNav.getByRole("button", { name: "All tools" }).click();
    await page.getByRole("button", { name: "Add Remove Password to favorites" }).click();
    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Remove Password" })).toBeVisible();

    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await page.getByRole("button", { name: "← All tools" }).click();
    await workspaceNav.getByRole("button", { name: "Recent" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Remove Password" })).toBeVisible();
});

test("tool cards open from the card surface without favorite navigation", async ({ page }) => {
    await page.goto("/");
    const card = page.locator(".tool-card").first();
    await card.click();
    await expect(page.getByRole("heading", { name: "Remove Password", exact: true })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.locator(".tool-card").first().getByRole("button", { name: "Add Remove Password to favorites" }).click();
    await expect(page.getByRole("heading", { name: "All tools", exact: true })).toBeVisible();
});

test("Windows labels the reveal action as opening the file location", async ({ page }) => {
    await page.addInitScript(() => {
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" });
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByLabel("Lossless compression").check();
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByRole("button", { name: "Open file location" })).toHaveCount(2);
});

test("settings checks for and installs available updates", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Settings" });
    await expect(dialog.getByRole("button", { name: "Check for updates" })).toBeVisible();
    await dialog.getByRole("button", { name: "Check for updates" }).click();
    await expect(dialog.getByRole("status")).toHaveText("Toolbox is up to date.");
});

test("tool cards render their design icon masks", async ({ page }) => {
    await page.goto("/");
    const icons = page.locator(".tool-card .card-icon > span");
    await expect(icons).toHaveCount(UtilityRegistry.length);

    const maskImages = await icons.evaluateAll((nodes) =>
        nodes.map((node) => getComputedStyle(node).maskImage),
    );

    expect(maskImages.every((maskImage) => maskImage !== "none")).toBe(true);
    expect(new Set(maskImages).size).toBeGreaterThan(1);
});

test("search shortcut stays in one inline keycap", async ({ page }) => {
    await page.goto("/");
    const shortcut = page.locator(".search-box kbd");
    const metrics = await shortcut.evaluate((node) => {
        const style = getComputedStyle(node);
        const rect = node.getBoundingClientRect();
        return { width: rect.width, height: rect.height, whiteSpace: style.whiteSpace };
    });

    expect(metrics.whiteSpace).toBe("nowrap");
    expect(metrics.width).toBeGreaterThan(metrics.height);
});

test("desktop scrolling stays inside the command pane", async ({ page }) => {
    await page.goto("/");
    const layout = await page.evaluate(() => ({
        documentOverflow: getComputedStyle(document.documentElement).overflow,
        bodyOverflow: getComputedStyle(document.body).overflow,
        shellFillsViewport:
            document.querySelector(".app-shell").getBoundingClientRect().height === window.innerHeight,
        commandOverflow: getComputedStyle(document.querySelector(".command-center")).overflowY,
        commandOverscroll: getComputedStyle(document.querySelector(".command-center")).overscrollBehaviorY,
    }));

    expect(layout.documentOverflow).toBe("hidden");
    expect(layout.bodyOverflow).toBe("hidden");
    expect(layout.shellFillsViewport).toBe(true);
    expect(layout.commandOverflow).toBe("auto");
    expect(layout.commandOverscroll).toBe("contain");
});

const drawSceneMark = async (page: Page, tool: string) => {
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: tool, exact: true }).click();
    const preview = page.getByRole("group", { name: "PDF page canvas" });
    await expect(preview).toBeVisible();
    const box = (await preview.boundingBox())!;
    await page.mouse.move(box.x + box.width * .2, box.y + box.height * .2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * .6, box.y + box.height * .4, { steps: 5 });
    await page.mouse.up();
};

test("crop is applied visually and exported as part of the scene", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Crop PDF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await drawSceneMark(page, "Crop");
    await page.getByRole("button", { name: "Export PDF", exact: true }).click();
    const invocation = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.find(c => c.command === "export_pdf_scene"));
    expect(invocation?.args).toMatchObject({ request: { scene: { pages: [{ crop: { x: expect.any(Number), width: expect.any(Number) } }] } } });
});

test("pdf editor renders page previews", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Sign PDF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await expect(page.getByRole("img", { name: "Thumbnail of page 1" })).toBeVisible();
});

test("PDF editor inserts editable blank pages from thumbnails", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Organize PDF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Add blank page", exact: true }).click();
    await drawSceneMark(page, "Shape");
    await page.getByRole("button", { name: "Export PDF", exact: true }).click();
    const invocation = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.find(c => c.command === "export_pdf_scene"));
    expect(invocation?.args).toMatchObject({ request: { scene: { pages: [{ sourceIndex: 0 }, { sourceIndex: null, objects: [{ kind: "shape" }] }] } } });
});

test("editor command rails keep one document session while changing tools", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("img", { name: `Preview of ${fixtureName}` })).toBeVisible();

    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Rotate and Flip Images" }).click();
    await expect(page.getByRole("heading", { name: "Rotate and Flip Images", exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file open");
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Colour and Tone Adjustments" }).click();
    await expect(page.getByRole("slider", { name: "Contrast" })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open PDF editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Crop", exact: true }).click();
    await expect(page.getByText("Drag a rectangle on the page to crop.", { exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file open");
});

test("vision tools explain unavailable resources before file selection", async ({ page }) => {
    await page.goto("/");
    for (const [index, id] of ["pdf-ocr", "image-blur-faces", "image-remove-bg"].entries()) {
        const utility = UtilityRegistry.find((item) => item.id === id);
        expect(utility).toBeDefined();
        if (!utility) continue;
        if (index > 0) await page.getByRole("button", { name: "← All tools" }).click();
        await page.getByRole("button", { name: `Open ${utility.title}` }).click();
        await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
        await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveCount(0);
    }
});

test("PDF editor protects the last page and supports unified history", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Remove PDF Pages" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("button", { name: "Delete selected pages" })).toBeDisabled();
    await drawSceneMark(page, "Text");
    await expect(page.locator(".scene-object-hit")).toHaveCount(1);
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(0);
    await page.getByRole("button", { name: "Redo", exact: true }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(1);
    await page.getByRole("button", { name: "Reset edits" }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(0);
});

test("Image editor previews a reversible edit stack and exports one combined plan", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("img", { name: `Preview of ${fixtureName}` })).toBeVisible();
    await expect(page.getByRole("button", { name: "Undo" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Redo" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeDisabled();
    await expect(page.locator('[role="tablist"]')).toHaveCount(0);
    await expect(page.getByRole("toolbar", { name: "Image editor tools" })).toBeVisible();
    await expect(page.locator(".workspace-primary-action")).toHaveCount(1);

    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Resize Images" }).click();
    await page.getByLabel("Width").fill("320");
    await expect.poll(async () =>
        (await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []))
            .some(({ command }) => command === "inspect_image_edit_preview"),
    ).toBe(true);
    await page.getByRole("button", { name: "Add edit to plan" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Rotate and Flip Images" }).click();
    await page.getByLabel("Rotation").selectOption("90");
    await page.getByRole("button", { name: "Add edit to plan" }).click();
    await expect(page.getByText("2 committed edits", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Export edited images" }).click();

    const invocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "export_image_edit_plan"),
    );
    expect(invocation?.args).toMatchObject({ request: { plan: { edits: expect.any(Array) } } });
    expect((invocation?.args as { request: { plan: { edits: unknown[] } } } | undefined)?.request.plan.edits).toHaveLength(2);
});

test("Image crop keeps the source interaction surface and shows a separate result", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Crop Images", exact: true }).click();
    await page.getByLabel("Crop width").fill("160");
    await page.getByLabel("Crop height").fill("120");
    await page.getByLabel("Crop left").fill("80");
    await page.getByLabel("Crop top").fill("60");

    await expect(page.getByRole("img", { name: `Original image preview of ${fixtureName}` })).toBeVisible();
    await expect(page.getByRole("region", { name: "Crop result preview" })).toBeVisible();
    const stage = (await page.locator(".image-editor-preview-stage").boundingBox())!;
    const overlay = page.getByLabel("Crop selection");
    await expect(overlay).toHaveAttribute("style", /left: 12\.5%/);
    await expect(overlay).toHaveAttribute("style", /top: 12\.5%/);

    await page.mouse.move(stage.x + stage.width * .25, stage.y + stage.height * .25);
    await page.mouse.down();
    await page.mouse.move(stage.x + stage.width * .5, stage.y + stage.height * .5, { steps: 4 });
    await page.mouse.up();
    await expect(page.getByLabel("Crop left")).toHaveValue("240");
    await expect(page.getByLabel("Crop top")).toHaveValue("180");
    await expect(overlay).toHaveAttribute("style", /left: 37\.5%/);
    await expect(overlay).toHaveAttribute("style", /top: 37\.5%/);
});

test("Image aspect crop uses the anchored maximum-fit geometry for overlay and result", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Crop Images", exact: true }).click();
    await page.getByLabel("Crop mode").selectOption("aspectRatio");
    await page.getByLabel("Crop width").fill("1");
    await page.getByLabel("Crop height").fill("1");
    await page.getByLabel("Anchor").selectOption("right");

    const overlay = page.getByLabel("Crop selection");
    await expect(overlay).toHaveAttribute("style", /left: 25%/);
    await expect(overlay).toHaveAttribute("style", /top: 0%/);
    await expect(overlay).toHaveAttribute("style", /width: 75%/);
    await expect(overlay).toHaveAttribute("style", /height: 100%/);
    await expect(page.getByRole("region", { name: "Crop result preview" })).toContainText("480 × 480px");
});

test("Image crop clears an old result and disables export after invalid preview validation", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Crop Images", exact: true }).click();
    await page.getByLabel("Crop width").fill("160");
    await page.getByLabel("Crop height").fill("120");
    await expect(page.getByRole("region", { name: "Crop result preview" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeEnabled();

    await page.getByLabel("Crop mode").selectOption("aspectRatio");
    await page.getByLabel("Crop width").fill("0");

    await expect(page.getByText("Aspect ratio dimensions must be positive.", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "Crop result preview" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
});

test("Image editor history can undo, redo, and reset the composed plan", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Rotate and Flip Images" }).click();
    await page.getByLabel("Rotation").selectOption("90");
    await page.getByRole("button", { name: "Add edit to plan" }).click();

    await expect(page.getByRole("button", { name: "Undo" })).toBeEnabled();
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Redo" })).toBeEnabled();

    await page.getByRole("button", { name: "Redo" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeEnabled();
    await page.getByRole("button", { name: "Reset edits" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
});

test("PDF conversion exposes page selection and an output preview", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("region", { name: "Conversion preview" })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
    await expect(page.getByLabel("Render DPI")).toBeVisible();
    await expect(page.getByLabel("Output format")).toBeVisible();
    await expect(page.getByText("1 page selected", { exact: true })).toBeVisible();

    await page.getByRole("checkbox", { name: "Page 1" }).uncheck();
    await expect(page.locator(".workspace-primary-action")).toBeDisabled();
});

test("PDF range splitting rejects a blank range and accepts a range", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "Split PDF" }).click();
    await page.getByLabel("Split mode").selectOption("ranges");

    await expect(page.getByRole("alert")).toHaveText("Enter at least one page range to split by ranges.");
    await expect(page.getByRole("button", { name: "Export Split PDF" })).toBeDisabled();

    await page.getByRole("textbox", { name: "Page ranges" }).fill("1");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export Split PDF" })).toBeEnabled();
});

test("media and security workspaces expose ordered inputs and format boundaries", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Create GIF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("region", { name: "Frame order" })).toBeVisible();
    await expect(page.locator(".media-frame-preview")).toBeVisible();
    await expect.poll(async () => page.evaluate(() => (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_image_preview").length ?? 0)).toBe(1);
    await expect(page.getByRole("button", { name: "Move selected file up" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Move selected file down" })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open Protect PDF" }).click();
    await expect(page.getByText("PDF files only", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await expect(page.getByText("PDF, Word, Excel, and PowerPoint files", { exact: true })).toBeVisible();
});

test("TIFF workspace expands internal pages and sends the displayed order", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Split and Combine TIFF" }).click();
    await page.evaluate(() => {
        (window as TestWindow).__toolboxDialogResults = [["/local/two-page.tiff", "/local/one-page.tiff"]];
    });
    await page.getByRole("button", { name: "Choose files to process" }).click();

    const frames = page.locator(".media-frame-list button");
    await expect(frames).toHaveCount(3);
    await expect(frames.nth(0)).toContainText("two-page.tiff · page 1");
    await expect(frames.nth(1)).toContainText("two-page.tiff · page 2");
    await expect(frames.nth(2)).toContainText("one-page.tiff · page 1");
    await frames.nth(1).click();
    await page.getByRole("button", { name: "Move selected TIFF page up" }).click();
    await expect(frames.nth(0)).toContainText("two-page.tiff · page 2");

    await page.locator(".workspace-primary-action").click();
    const invocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "process_tiff_pages"),
    );
    expect(invocation?.args).toMatchObject({
        request: {
            paths: ["/local/two-page.tiff", "/local/one-page.tiff"],
            pages: [
                { path: "/local/two-page.tiff", page: 1 },
                { path: "/local/two-page.tiff", page: 0 },
                { path: "/local/one-page.tiff", page: 0 },
            ],
        },
    });
});

test("shared workspace ignores a stale processing completion after action changes", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByLabel("Lossless compression").check();
    await page.evaluate(() => { (window as TestWindow).__toolboxProcessingDelayMs = 150; });
    await page.getByRole("button", { name: "Export edited images" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Rotate and Flip Images" }).click();
    await page.waitForTimeout(220);
    await expect(page.getByText("Test output", { exact: true })).toHaveCount(0);
});

test("command K focuses the tool search", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });

    await page.locator("body").press("Meta+K");

    await expect(search).toBeFocused();
});

test("remove password exposes one cross-format document tool", async ({ page }) => {
    const utility = UtilityRegistry.find((item) => item.id === "pdf-unlock");
    expect(utility).toBeDefined();
    expect(utility?.title).toBe("Remove Password");
    expect(utility?.category).toBe("Documents");
    expect(utility?.command).toBe("remove_password");

    await page.goto("/");
    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.locator('input[type="password"]').fill("test-password");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
});

test("every output exposes native file actions and keeps action errors inline", async ({ page }) => {
    const failedOutput = `${fixturePath}.partial-output`;
    const outputs = [...mockedOutputPaths, failedOutput];
    await page.goto("/");
    await page.evaluate(({ fixturePath, mockedOutputPaths, failedOutput }) => {
        (window as TestWindow).__toolboxProcessingResults = [
            {
                inputPath: fixturePath,
                outputPaths: mockedOutputPaths,
                detail: "Test output",
                failure: null,
            },
            {
                inputPath: `${fixturePath}.failed-input`,
                outputPaths: [failedOutput],
                detail: "",
                failure: { kind: "processing", message: "Processing failed" },
            },
        ];
        (window as TestWindow).__toolboxActionFailure = {
            command: "reveal_output_path",
            path: failedOutput,
            message: "Could not reveal test output",
        };
    }, { fixturePath, mockedOutputPaths, failedOutput });

    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByLabel("Lossless compression").check();
    await page.getByRole("button", { name: "Export edited images" }).click();

    const outputRows = page.locator(".result-output");
    await expect(outputRows).toHaveCount(outputs.length);
    await expect(page.getByRole("button", { name: "Open file" })).toHaveCount(outputs.length);
    await expect(page.getByRole("button", { name: "Show in Finder" })).toHaveCount(outputs.length);

    for (let index = 0; index < outputs.length; index += 1) {
        await outputRows.nth(index).getByRole("button", { name: "Open file" }).click();
        await outputRows.nth(index).getByRole("button", { name: "Show in Finder" }).click();
    }

    const actionInvocations = await page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) =>
            command === "open_output_path" || command === "reveal_output_path"
        ),
    );
    expect(actionInvocations).toEqual(outputs.flatMap((path) => [
        { command: "open_output_path", args: { path } },
        { command: "reveal_output_path", args: { path } },
    ]));
    await expect(page.getByRole("alert")).toHaveText("Could not reveal test output");
    await expect(page.getByText("1 of 2 files failed", { exact: true })).toBeVisible();
    await expect(page.getByText("Processing failed", { exact: true })).toBeVisible();
});

test("output action state resets and ignores stale completions", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByLabel("Lossless compression").check();
    const processButton = page.getByRole("button", { name: "Export edited images" });
    await processButton.click();

    await page.evaluate(({ path }) => {
        (window as TestWindow).__toolboxActionFailure = {
            command: "open_output_path",
            path,
            message: "Stale action failure",
            delayMs: 150,
        };
        (window as TestWindow).__toolboxProcessingResults = [{
            inputPath: `${path}.next-input`,
            outputPaths: [`${path}.next-output`],
            detail: "New result set",
            failure: null,
        }];
    }, { path: mockedOutputPaths[0] });

    await page.locator(".result-output").first().getByRole("button", { name: "Open file" }).click();
    await processButton.click();

    await expect(page.getByText("New result set", { exact: true })).toBeVisible();
    await expect(page.locator(".result-output")).toHaveCount(1);
    await page.waitForTimeout(200);
    await expect(page.getByText("Stale action failure", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open file" })).toBeEnabled();
});

test("single-input actions report every selected file instead of truncating the selection", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Merge PDF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.evaluate(() => { (window as TestWindow).__toolboxSecondPick = true; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Compress PDF", exact: true }).click();

    const exportButton = page.getByRole("button", { name: "Export Compress PDF", exact: true });
    await expect(exportButton).toBeEnabled();
    await exportButton.click();

    await expect(page.getByText("2 of 2 files failed", { exact: true })).toBeVisible();
    const processingInvocations = await page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "compress_pdf"),
    );
    expect(processingInvocations).toHaveLength(0);
    await expect(page.getByText("This action accepts one input file, but 2 were selected.", { exact: true })).toHaveCount(2);
});

test("single-input replacement keeps the source and edit state after cancellation or the same path", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Crop PDF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await drawSceneMark(page, "Crop");
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();

    await page.evaluate((path) => { (window as TestWindow).__toolboxDialogResults = [null, path]; }, fixturePath);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file open");
    await expect(page.getByText(fixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();

    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file open");
    await expect(page.getByText(fixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();
});

test("page-scoped actions disable export when the explicit selection is empty", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Text" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    const pageCheckbox = page.getByRole("checkbox", { name: "Page 1" });
    await expect(pageCheckbox).toBeChecked();
    await pageCheckbox.uncheck();
    await expect(page.getByRole("button", { name: "Export PDF to Text", exact: true })).toBeDisabled();
});

test("PDF replacement clears stale inspection state while inspection is pending or rejected", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Extract PDF Pages" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeEnabled();

    await page.evaluate(() => {
        (window as TestWindow).__toolboxInspectionDelayMs = 250;
        (window as TestWindow).__toolboxDialogResults = ["/local/document.replacement.pdf"];
    });
    await page.getByRole("button", { name: "Choose files to process" }).click();

    await expect(page.getByRole("checkbox", { name: "Page 1" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeDisabled();
    await expect(page.getByRole("checkbox", { name: "Page 2" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeEnabled();

    await page.evaluate(() => {
        (window as TestWindow).__toolboxInspectionError = "inspection rejected";
        (window as TestWindow).__toolboxDialogResults = ["/local/document.rejected.pdf"];
    });
    await page.getByRole("button", { name: "Choose files to process" }).click();

    await expect(page.getByRole("checkbox", { name: "Page 2" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeDisabled();
    await expect(page.getByText("inspection rejected", { exact: true })).toBeVisible();
});

test("PDF conversion resets the displayed scope when switching actions", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Images" }).click();
    await page.evaluate(() => {
        (window as TestWindow).__toolboxDialogResults = ["/local/document.scoped.pdf"];
    });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeVisible();
    await page.getByRole("textbox", { name: "Page range" }).fill("1");
    await expect(page.getByRole("checkbox", { name: "Page 2" })).not.toBeChecked();

    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "PDF to Text", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeVisible();
    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "PDF to Images", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeVisible();

    await expect(page.getByRole("textbox", { name: "Page range" })).toHaveValue("");
    await expect(page.getByRole("checkbox", { name: "Page 2" })).toBeChecked();
    await page.getByRole("button", { name: "Export PDF to Images", exact: true }).click();

    const invocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "pdf_to_images"),
    );
    expect(invocation?.args).toMatchObject({ request: { pages: [0, 1, 2], pageRange: "1-3" } });
});

test("GIF extraction does not request a source preview", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Extract GIF Frames" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByLabel("Tool detail").getByRole("button", { name: "Frames", exact: true })).toBeEnabled();

    const previewInvocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_image_preview") ?? []);
    expect(previewInvocations).toHaveLength(0);
});

const exerciseFeature = async (page: Page, utility: (typeof UtilityRegistry)[number]) => {
    await page.goto("/");
    await page.getByRole("button", { name: `Open ${utility.title}` }).click();

    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(fixtureName, { exact: true })).toBeVisible();

    if (utility.id === "pdf-unlock" || utility.id === "pdf-protect") {
        await page.locator('input[type="password"]').fill("test-password");
    }
    if (workspaceForTool(utility.id)?.id === "pdf-editor") {
        if (utility.id === "pdf-page-numbers") await page.getByRole("button", { name: "Page numbers", exact: true }).click();
        else if (utility.id === "pdf-organize") await page.getByRole("button", { name: "Add blank page", exact: true }).click();
        else if (utility.id === "pdf-remove-pages") {
            await page.getByRole("button", { name: "Add blank page", exact: true }).click();
            await page.getByRole("button", { name: "Delete selected pages", exact: true }).click();
        } else await drawSceneMark(page, utility.id === "pdf-sign" ? "Signature" : utility.id === "pdf-watermark" ? "Watermark" : utility.id === "pdf-crop" ? "Crop" : "Text");
    }
    if (workspaceForTool(utility.id)?.id === "image-editor") {
        if (utility.id === "heic-convert") await page.getByLabel("Target format").selectOption("jpg");
        if (utility.id === "compress") await page.getByLabel("Lossless compression").check();
        if (utility.id === "resize") await page.getByLabel("Width").fill("640");
        if (utility.id === "rotate") await page.getByLabel("Rotation").selectOption("90");
        if (utility.id === "crop") await page.getByLabel("Crop width").fill("320");
        if (utility.id === "image-watermark") await page.getByLabel("Watermark text").fill("Test watermark");
        if (utility.id === "image-tone") await page.getByRole("slider", { name: "Brightness" }).fill("10");
    }
    if (utility.id === "pdf-merge") {
        await page.evaluate(() => { (window as TestWindow).__toolboxSecondPick = true; });
        await page.getByRole("button", { name: "Choose files to process" }).click();
    }
    if (workspaceForTool(utility.id)?.id === "image-editor") {
        await page.getByRole("button", { name: "Add edit to plan" }).click();
    }

    const action = page.locator(".workspace-primary-action");
    await expect(action).toBeEnabled();
    await action.click();

    await expect(page.getByText("Test output", { exact: true })).toBeVisible();
    const invocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []);
    const composedCommand = workspaceForTool(utility.id)?.id === "image-editor"
        ? "export_image_edit_plan"
        : workspaceForTool(utility.id)?.id === "pdf-editor" && ["pdf-edit", "pdf-crop", "pdf-watermark", "pdf-sign", "pdf-page-numbers", "pdf-remove-pages", "pdf-organize"].includes(utility.id)
? "export_pdf_scene"
            : utility.command;
    expect(invocations.some(({ command }) => command === composedCommand)).toBe(true);
};

test.describe("registered feature actions", () => {
    expect(UtilityRegistry).toHaveLength(32);

    for (const utility of UtilityRegistry) {
        test(`${utility.id} accepts the fixture and runs`, async ({ page }) => {
            test.skip(utility.status !== "implemented", "Tool is not available in this build.");
            await exerciseFeature(page, utility);
        });
    }
});
