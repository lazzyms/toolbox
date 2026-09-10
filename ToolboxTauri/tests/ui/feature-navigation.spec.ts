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
    __toolboxActionFailure?: { command: string; path: string; message: string; delayMs?: number };
    __toolboxSecondPick?: boolean;
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
                    return { pages: [{ index: 0, x: 0, y: 0, width: 612, height: 792, preview: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" }] };
                }
                if (command === "inspect_image_preview") {
                    return { width: 640, height: 480, dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" };
                }
                if (command === "inspect_image_edit_preview") {
                    return { width: 640, height: 480, dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" };
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

test("media and security workspaces expose ordered inputs and format boundaries", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Create GIF" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("region", { name: "Frame order" })).toBeVisible();
    await expect(page.locator(".media-frame-preview")).toBeVisible();
    await expect(page.getByRole("button", { name: "Move selected file up" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Move selected file down" })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open Protect PDF" }).click();
    await expect(page.getByText("PDF files only", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await expect(page.getByText("PDF, Word, Excel, and PowerPoint files", { exact: true })).toBeVisible();
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
