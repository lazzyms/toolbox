import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import path from "node:path";
import { UtilityRegistry } from "../../src/registry";

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const fixtureName = path.basename(fixturePath);
const mockedOutputPaths = [`${fixturePath}.output-one`, `${fixturePath}.output-two`];
const appVersion = (JSON.parse(readFileSync(path.resolve("package.json"), "utf8")) as { version: string }).version;

type MockOutcome = {
    inputPath: string;
    outputPaths: string[];
    detail: string;
    failure: { kind: string; message: string } | null;
};

type TestWindow = Window & {
    __toolboxInvocations?: Array<{ command: string; args: unknown }>;
    __toolboxProcessingResults?: MockOutcome[];
    __toolboxActionFailure?: { command: string; path: string; message: string; delayMs?: number };
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

                if (command === "plugin:dialog|open") return fixturePath;
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
                if (command === "inspect_pdf") {
                    return { pages: [{ index: 0, width: 612, height: 792 }] };
                }
                if (command === "inspect_image_metadata") return ["fixture image"];
                if (command.startsWith("plugin:")) return null;

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
        await expect(page.getByRole("heading", { name: utility.title, exact: true })).toBeVisible();
        await expect(page.getByRole("status")).toHaveText(`${utility.title} selected.`);
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

    const dropzone = page.getByRole("button", { name: "Choose files to process" });
    const readSurface = () => dropzone.evaluate((node) => ({
        background: getComputedStyle(node).backgroundColor,
        border: getComputedStyle(node).borderTopColor,
        copy: getComputedStyle(node.querySelector("p")!).color,
    }));

    await page.mouse.move(0, 0);
    const darkSurface = await readSurface();
    const settings = page.getByRole("button", { name: "Settings", exact: true });
    await settings.click();
    const dialog = page.getByRole("dialog", { name: "Settings" });
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

test("sidebar and detail pane scroll independently", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator('nav[aria-label="Utilities"]')).toHaveCSS("overflow-y", "auto");
    await expect(page.locator('main[aria-label="Tool detail"]')).toHaveCSS("overflow-y", "auto");
});

test("sidebar does not show the engine badge", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("Unified Native Engine", { exact: true })).toHaveCount(0);
});

test("settings shows app info and the donation QR code", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings" }).click();

    const settings = page.getByRole("dialog", { name: "Settings" });
    await expect(settings).toBeVisible();
    await expect(settings.getByText("App info", { exact: true })).toBeVisible();
    await expect(settings.getByText("Toolbox", { exact: true })).toBeVisible();
    await expect(settings.getByText(`Version ${appVersion}`, { exact: true })).toBeVisible();
    await expect(settings.getByRole("img", { name: "Buy Me a Coffee donation QR code" })).toBeVisible();

    await settings.getByRole("button", { name: "Close settings" }).click();
    await expect(settings).toBeHidden();
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
    await expect(page.getByLabel("Tool detail").getByRole("button", { name: "Remove Password" })).toBeEnabled();
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
    await page.getByLabel("Tool detail").getByRole("button", { name: "Compress Images", exact: true }).click();

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
    const processButton = page.getByLabel("Tool detail").getByRole("button", { name: "Compress Images", exact: true });
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
    if (utility.id === "pdf-edit") {
        await page.getByLabel("Edit text").fill("Test annotation");
    }

    const action = page.locator("main button").filter({ hasText: utility.shortTitle }).last();
    await expect(action).toBeEnabled();
    await action.click();

    await expect(page.getByText("Test output", { exact: true })).toBeVisible();
    const invocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []);
    expect(invocations.some(({ command }) => command === utility.command)).toBe(true);
};

test.describe("registered feature actions", () => {
    expect(UtilityRegistry).toHaveLength(32);

    for (const utility of UtilityRegistry) {
        test(`${utility.id} accepts the fixture and runs`, async ({ page }) => {
            await exerciseFeature(page, utility);
        });
    }
});
