import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import type { JobOutcome, OutputSettings } from "../../src/contracts";
import { ToolWorkspaceRegistry, UtilityRegistry, workspaceForTool } from "../../src/registry";
import { RetiredToolIds } from "../../src/registry/retiredToolIds";

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const fixtureName = path.basename(fixturePath);
const pdfFixturePath = path.resolve("tests/fixtures/document.pdf");
const pdfFixtureName = path.basename(pdfFixturePath);
const mockedOutputPaths = [`${fixturePath}.output-one`, `${fixturePath}.output-two`];

type MockOutcome = JobOutcome;

type TestWindow = Window & {
    __toolboxInvocations?: Array<{ command: string; args: unknown }>;
    __toolboxProcessingResults?: MockOutcome[];
    __toolboxProcessingDelayMs?: number;
    __toolboxInspectionDelayMs?: number;
    __toolboxTiffInspectionDelayMs?: number;
    __toolboxInspectionError?: string;
    __toolboxActionFailure?: { command: string; path: string; message: string; delayMs?: number };
    __toolboxSecondPick?: boolean;
    __toolboxDialogResults?: Array<string | string[] | null>;
    __toolboxOutputSettings?: OutputSettings;
    __toolboxFolderChecks?: Record<string, "ready" | "missing" | "unwritable">;
};

const defaultOutputSettings: OutputSettings = { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ fixturePath, pdfFixturePath, mockedOutputPaths, defaultOutputSettings }) => {
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

                if (command === "get_output_settings") return (window as TestWindow).__toolboxOutputSettings ?? defaultOutputSettings;
                if (command === "set_output_settings") {
                    const { settings } = args as { settings: OutputSettings };
                    (window as TestWindow).__toolboxOutputSettings = settings;
                    return settings;
                }
                if (command === "check_output_folder") return (window as TestWindow).__toolboxFolderChecks?.[(args as { folder: string }).folder] ?? "ready";
                if (command === "plugin:dialog|open") {
                    const dialogResults = (window as TestWindow).__toolboxDialogResults;
                    if (dialogResults?.length) return dialogResults.shift() ?? null;
                    if ((window as TestWindow).__toolboxSecondPick) {
                        delete (window as TestWindow).__toolboxSecondPick;
                        return `${pdfFixturePath}.second.pdf`;
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
if (command === "preview_pdf_scene_pages") return (args as { request: { pageIndices: number[] } }).request.pageIndices.map((pageIndex) => ({ pageIndex, preview: { dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'/%3E", width: 612, height: 792 } }));
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
                    const inspectionDelay = (window as TestWindow).__toolboxTiffInspectionDelayMs;
                    if (inspectionDelay) await new Promise((resolve) => window.setTimeout(resolve, inspectionDelay));
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
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:")) return null;

                const processingDelay = (window as TestWindow).__toolboxProcessingDelayMs;
                if (processingDelay) await new Promise((resolve) => window.setTimeout(resolve, processingDelay));

                return (window as TestWindow).__toolboxProcessingResults ?? [{
                    inputPath: fixturePath,
                    outputPaths: mockedOutputPaths,
                    detail: "Test output",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: [],
                }];
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
            unregisterListener: () => undefined,
        };
    }, { fixturePath, pdfFixturePath, mockedOutputPaths, defaultOutputSettings });
});

test("every registered feature opens its detail pane", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".topbar--home").getByRole("textbox", { name: "Search tools" })).toBeVisible();

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
        await expect(page.locator('p.sr-only[role="status"]')).toHaveText(`${workspaceForTool(utility.id)?.title ?? utility.title} workspace open.`);
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
    const sections = page.getByRole("navigation", { name: "Settings sections" });
    await expect(sections.getByRole("button", { name: "Appearance" })).toHaveAttribute("aria-pressed", "true");
    await expect(sections.getByRole("button", { name: "Appearance" })).toBeFocused();
    await expect(page.getByRole("heading", { level: 1, name: "Appearance" })).toBeVisible();
    await expect(settings).toBeHidden();
});

test("file upload surface follows the selected theme", async ({ page }) => {
    await page.goto("/");
    const utility = UtilityRegistry[0];
    await page.getByRole("button", { name: `Open ${utility.title}` }).click();

    const sourceBar = page.locator(".workspace-source-bar");
    const readSurface = () => sourceBar.evaluate((node) => ({
        background: getComputedStyle(node).backgroundColor,
        border: getComputedStyle(node).borderTopColor,
        copy: getComputedStyle(node.querySelector(".workspace-source-copy > span:last-child")!).color,
    }));

    const settings = page.getByRole("button", { name: "Settings", exact: true });
    const theme = page.getByRole("radiogroup", { name: "Theme" });
    const chooseTheme = async (name: "Dark" | "Light") => {
        await settings.click();
        await theme.getByRole("radio", { name }).click();
        await page.keyboard.press("Escape");
        await expect(sourceBar).toBeVisible();
    };
    await chooseTheme("Dark");
    await expect.poll(() => page.locator("body").getAttribute("data-theme")).toBe("dark");
    await page.mouse.move(0, 0);
    const darkSurface = await readSurface();
    await chooseTheme("Light");
    await expect.poll(() => page.locator("body").getAttribute("data-theme")).toBe("light");
    await page.mouse.move(0, 0);
    await expect.poll(async () => (await readSurface()).background).not.toBe(darkSurface.background);

    const lightSurface = await readSurface();
    expect(lightSurface.background).not.toBe(darkSurface.background);
    expect(lightSurface.border).not.toBe(darkSurface.border);
    expect(lightSurface.copy).not.toBe(darkSurface.copy);

    await chooseTheme("Dark");
    await page.mouse.move(0, 0);
    await expect.poll(readSurface).toEqual(darkSurface);
});

test("favorites and recent navigation show their intended libraries", async ({ page }) => {
    await page.goto("/");
    const workspaceNav = page.locator('nav[aria-label="Workspace navigation"]');

    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(
        page.getByRole("radiogroup", { name: "Tool library filters" })
            .getByRole("radio", { name: "Favorites", exact: true }),
    ).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText("No favorite tools yet", { exact: true })).toBeVisible();
    await expect(workspaceNav.getByRole("button", { name: "Favorites" })).toHaveAttribute("aria-current", "page");

    await workspaceNav.getByRole("button", { name: "Recent" }).click();
    await expect(
        page.getByRole("radiogroup", { name: "Tool library filters" })
            .getByRole("radio", { name: "Recent", exact: true }),
    ).toHaveAttribute("aria-checked", "true");
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
    await expect(
        page.getByRole("radiogroup", { name: "Tool library filters" })
            .getByRole("radio", { name: "Recent" }),
    ).toHaveAttribute("aria-checked", "true");

    await page.reload();
    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Remove Password" })).toBeVisible();
    await workspaceNav.getByRole("button", { name: "Recent" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Remove Password" })).toBeVisible();
});

test("saved favorites and recents survive the startup id migration and only show registered tools", async ({ page }) => {
    await page.addInitScript(() => {
        localStorage.setItem("toolbox-favorites", JSON.stringify(["pdf-merge", "mystery-tool"]));
        localStorage.setItem("toolbox-recent", JSON.stringify(["mystery-tool", "pdf-unlock"]));
    });
    await page.goto("/");
    const workspaceNav = page.locator('nav[aria-label="Workspace navigation"]');

    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Merge PDF" })).toBeVisible();
    await workspaceNav.getByRole("button", { name: "Recent" }).click();
    await expect(page.locator(".tool-card")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Remove Password" })).toBeVisible();

    const stored = await page.evaluate(() => ({
        favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null"),
        recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null"),
        marker: localStorage.getItem("toolbox-tool-id-migration"),
    }));
    expect(stored).toEqual(
        RetiredToolIds.version === 0
            ? { favorites: ["pdf-merge", "mystery-tool"], recent: ["mystery-tool", "pdf-unlock"], marker: null }
            : { favorites: ["pdf-merge"], recent: ["pdf-unlock"], marker: String(RetiredToolIds.version) },
    );
});

test("all five workspace cards open their workspace", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".tool-card")).toHaveCount(ToolWorkspaceRegistry.length);

    for (const workspace of ToolWorkspaceRegistry) {
        await page.getByRole("button", { name: `Open ${workspace.title}`, exact: true }).click();
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);
        await page.getByRole("button", { name: "← All tools" }).click();
    }
});

test("search finds and opens all 33 registered tool actions", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });
    expect(UtilityRegistry).toHaveLength(33);

    for (const utility of UtilityRegistry) {
        await search.fill(utility.title);
        const result = page.getByRole("button", { name: `Open ${utility.title}`, exact: true });
        await expect(result).toBeVisible();
        await result.click();
        if (utility.status === "unavailable") {
            await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
        } else {
            await expect(page.locator(".workspace-title")).toHaveText(workspaceForTool(utility.id)?.title);
        }
        await page.getByRole("button", { name: "← All tools" }).click();
    }
});

test("search arrows navigate visible results and Enter opens the focused result", async ({ page }, testInfo) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await page.getByRole("button", { name: "← All tools" }).click();
    await expect(page.locator(".recent-section")).toHaveCount(0);
    await expect(page.locator(".topbar--home + .tool-library-section")).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("phase2-command-center-gallery.png") });

    const search = page.getByRole("textbox", { name: "Search tools" });
    await search.fill("Protect");
    const composingArrowPrevented = await search.evaluate((input) => {
        const event = new KeyboardEvent("keydown", {
            key: "ArrowDown",
            bubbles: true,
            cancelable: true,
            isComposing: true,
        });
        input.dispatchEvent(event);
        return event.defaultPrevented;
    });
    expect(composingArrowPrevented).toBe(false);
    await expect(search).toBeFocused();
    await expect(page.locator(".recent-section")).toHaveCount(0);
    await expect(page.locator("button[data-command-result]")).toHaveCount(3);
    const removePassword = page.getByRole("button", { name: "Open Remove Password", exact: true });
    const protectPdf = page.getByRole("button", { name: "Open Protect PDF", exact: true });
    const protectOfficeFiles = page.getByRole("button", { name: "Open Protect Office Files", exact: true });

    await page.keyboard.press("ArrowDown");
    await expect(removePassword).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(protectPdf).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(protectOfficeFiles).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(protectPdf).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(removePassword).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(search).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await expect(protectPdf).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath("phase2-command-center-search.png") });
    await page.keyboard.press("Enter");
    await expect(page.locator(".workspace-title")).toHaveText("Protect & unlock files");

    await page.getByRole("button", { name: "← All tools" }).click();
    await search.fill("no matching tool");
    await expect(page.getByText("No matching tools", { exact: true })).toBeVisible();
    await page.keyboard.press("ArrowDown");
    await expect(search).toBeFocused();
});

test("home search lives in the top bar while the privacy badge stays in workspaces", async ({ page }, testInfo) => {
    await page.goto("/");
    const homeTopbar = page.locator(".topbar--home");
    const search = homeTopbar.getByRole("textbox", { name: "Search tools" });

    await expect(search).toBeVisible();
    await expect(homeTopbar.getByText("On-device workspace", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "All tools", exact: true })).toHaveCount(0);
    await expect(page.getByRole("radio", { name: "All", exact: true })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("home-topbar-desktop.png") });

    await page.getByRole("button", { name: "Open Remove Password" }).click();
    await expect(page.locator(".topbar--home")).toHaveCount(0);
    await expect(page.locator(".topbar").getByText("On-device workspace", { exact: true })).toBeVisible();
    await expect(page.locator(".topbar").getByRole("textbox", { name: "Search tools" })).toHaveCount(0);

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.setViewportSize({ width: 700, height: 900 });
    await expect(page.locator(".topbar--home").getByRole("textbox", { name: "Search tools" })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("home-topbar-narrow.png") });
});

test("workspace cards expose an accessible open action without favorite navigation", async ({ page }) => {
    await page.goto("/");
    const card = page.locator(".tool-card").first();
    await expect(card).not.toHaveAttribute("tabindex");
    const openWorkspace = card.getByRole("button", { name: "Open Protect & unlock files" });
    await expect(openWorkspace).toBeVisible();
    await openWorkspace.click();
    await expect(page.getByRole("heading", { name: "Remove Password", exact: true })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.locator(".tool-card").first().getByRole("button", { name: "Add Remove Password to favorites" }).click();
    await expect(page.getByRole("heading", { name: "All tools", exact: true })).toHaveCount(0);
    await expect(page.getByRole("radio", { name: "All", exact: true })).toBeVisible();
});

test("Windows labels the reveal action for File Explorer", async ({ page }) => {
    await page.addInitScript(() => {
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" });
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("switch", { name: "Preserve original pixels" }).click();
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByRole("button", { name: "Show in File Explorer: icon.png.output-one", exact: true })).toHaveText("Show in File Explorer");
    await expect(page.getByRole("button", { name: /Show in Finder/ })).toHaveCount(0);
});

test("settings checks for and installs available updates", async ({ page }) => {
    await page.goto("/");
    const updates = await openSettingsSection(page, "Updates");
    await expect(updates.getByRole("status")).toHaveText("Check for updatesVersion 1.0.13.");
    await updates.getByRole("button", { name: "Check now" }).click();
    await expect(updates.getByRole("status")).toHaveText("Toolbox is up to dateVersion 1.0.13.");
});

async function mockUpdateAvailable(page: Page, dialogChoice: string | null) {
    await page.evaluate(
        ({ dialogChoice }) => {
            const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
            const origInvoke = internals.invoke.bind(internals);
            internals.invoke = async (command: string, args: unknown) => {
                if (command === "plugin:updater|check") {
                    return {
                        rid: 1,
                        currentVersion: "1.0.12",
                        version: "9.9.9",
                        date: "2026-09-27",
                        body: "test update",
                    };
                }
                if (command === "plugin:dialog|message") return dialogChoice;
                return origInvoke(command, args);
            };
        },
        { dialogChoice },
    );
}

async function invokedCommands(page: Page): Promise<string[]> {
    return page.evaluate(() =>
        ((window as unknown as { __toolboxInvocations?: Array<{ command: string }> }).__toolboxInvocations ?? []).map(
            (invocation) => invocation.command,
        ),
    );
}

test("settings asks before installing and does nothing when declined", async ({ page }) => {
    await page.goto("/");
    // Native confirm dialog returns null => user declined.
    await mockUpdateAvailable(page, null);
    const updates = await openSettingsSection(page, "Updates");
    await updates.getByRole("button", { name: "Check now" }).click();
    await expect(updates.getByRole("status")).toHaveText(
        "Version 9.9.9 is availableCheck again when you want to install it.",
    );
    expect(await invokedCommands(page)).not.toContain("plugin:updater|download_and_install");
});

test("settings downloads and installs after the update is confirmed", async ({ page }) => {
    await page.goto("/");
    // Native confirm dialog returns "Ok" (default button label) => user approved.
    await mockUpdateAvailable(page, "Ok");
    const updates = await openSettingsSection(page, "Updates");
    await updates.getByRole("button", { name: "Check now" }).click();
    await expect(updates.getByRole("status")).toHaveText("Installing version 9.9.9Toolbox restarts when it finishes.");
    expect(await invokedCommands(page)).toContain("plugin:updater|download_and_install");
});

test("settings privacy opt-out persists across reloads", async ({ page }) => {
    await page.goto("/");
    const privacy = await openSettingsSection(page, "Privacy");
    await expect(privacy).toContainText("Your files never leave this device.");
    const privacyToggle = privacy.getByRole("switch", { name: "Share anonymous usage counts" });
    await expect(privacyToggle).toHaveAttribute("aria-checked", "true");
    await privacyToggle.click();
    await expect(privacyToggle).toHaveAttribute("aria-checked", "false");
    expect(await page.evaluate(() => localStorage.getItem("toolbox.analytics.opt-out"))).toBe("1");

    await page.reload();
    await expect(
        (await openSettingsSection(page, "Privacy")).getByRole("switch", { name: "Share anonymous usage counts" }),
    ).toHaveAttribute("aria-checked", "false");
});

test("settings use shared theme and privacy controls in both palettes", async ({ page }, testInfo) => {
    for (const theme of ["dark", "light"] as const) {
        await page.goto("/");
        await page.evaluate((nextTheme) => {
            localStorage.setItem("toolbox-theme", nextTheme);
            document.body.dataset.theme = nextTheme;
        }, theme);
        await page.reload();
        await page.getByRole("button", { name: "Settings", exact: true }).click();

        await page.screenshot({ path: testInfo.outputPath(`settings-${theme}.png`) });
        const themeControl = page.getByRole("radiogroup", { name: "Theme" });
        await expect(themeControl.getByRole("radio", { name: theme === "dark" ? "Dark" : "Light" }))
            .toHaveAttribute("aria-checked", "true");

        const privacyToggle = (await openSettingsSection(page, "Privacy", false)).getByRole("switch", { name: "Share anonymous usage counts" });
        const before = await privacyToggle.getAttribute("aria-checked");
        await privacyToggle.click();
        await expect(privacyToggle).toHaveAttribute("aria-checked", before === "true" ? "false" : "true");
    }
});

test("workspace command rails follow the responsive toolbar keyboard contract", async ({ page }, testInfo) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    const rail = page.getByRole("toolbar", { name: "Image editor tools" });
    const commands = rail.getByRole("button");
    const commandCount = await commands.count();
    expect(commandCount).toBeGreaterThan(2);
    const first = commands.first();
    const second = commands.nth(1);
    const last = commands.nth(commandCount - 1);
    await expect(rail).toHaveAttribute("aria-orientation", "vertical");
    await first.focus();
    await page.screenshot({ path: testInfo.outputPath("workspace-command-rail.png") });

    await page.keyboard.press("ArrowRight");
    await expect(first).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(second).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(first).toBeFocused();
    await page.keyboard.press("End");
    await expect(last).toBeFocused();
    await page.keyboard.press("Home");
    await expect(first).toBeFocused();

    await page.setViewportSize({ width: 900, height: 720 });
    await expect(rail).toHaveAttribute("aria-orientation", "horizontal");
    await page.screenshot({ path: testInfo.outputPath("workspace-command-rail-narrow.png") });
    await first.focus();
    await page.keyboard.press("ArrowDown");
    await expect(first).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect(second).toBeFocused();
    await page.keyboard.press("ArrowLeft");
    await expect(first).toBeFocused();
    await page.keyboard.press("End");
    await expect(last).toBeFocused();
    await page.keyboard.press("Home");
    await expect(first).toBeFocused();
});

const openSettingsSection = async (page: Page, name: string, openSettings = true) => {
    if (openSettings) await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name, exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name, exact: true })).toBeVisible();
    return page.getByRole("region", { name, exact: true });
};

test("escape leaves the settings page and returns focus to the Settings button", async ({ page }) => {
    await page.goto("/");
    const settingsTrigger = page.getByRole("button", { name: "Settings", exact: true });
    await settingsTrigger.click();
    const privacy = await openSettingsSection(page, "Privacy", false);
    await privacy.getByRole("switch", { name: "Share anonymous usage counts" }).focus();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("complementary", { name: "Settings" })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Tool library" })).toBeVisible();
    await expect(settingsTrigger).toBeFocused();
});

test("All tools on the settings page returns to the open tool with its files", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Resize Images" }).click();
    await chooseFixture(page, fixturePath);
    const sourceFile = page.getByText(path.basename(fixturePath), { exact: true });

    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Appearance" })).toBeVisible();
    await expect(sourceFile).toBeHidden();
    await page.getByRole("complementary", { name: "Settings" }).getByRole("button", { name: "All tools" }).click();

    await expect(page.getByRole("heading", { level: 1, name: workspaceForTool("resize")!.title, exact: true })).toBeVisible();
    await expect(sourceFile).toBeVisible();
    await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeFocused();
});

test("the settings sidebar switches sections and lists only the built ones", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const sections = page.getByRole("navigation", { name: "Settings sections" });
    await expect(sections.getByRole("button")).toHaveText(["Appearance", "Output location", "Privacy", "Updates", "About and support"]);
    await expect(page.getByText("Changes save as you make them.")).toBeVisible();

    const about = await openSettingsSection(page, "About and support", false);
    await expect(sections.getByRole("button", { name: "About and support" })).toHaveAttribute("aria-pressed", "true");
    await expect(sections.getByRole("button", { name: "Appearance" })).toHaveAttribute("aria-pressed", "false");
    await expect(about).toContainText("Version 1.0.13 for macOS");
    await expect(about.getByRole("img", { name: "Buy Me a Coffee donation QR code" })).toBeVisible();
    await expect(about).toContainText("Support Toolbox");
});

test("Find a tool on the settings page opens the tool search", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Resize Images" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const findTool = page.getByRole("button", { name: "Find a tool" });
    await expect(findTool).toHaveAttribute("aria-keyshortcuts", "Meta+K");
    await expect(findTool).toContainText("⌘K");
    await findTool.click();
    await expect(page.getByRole("textbox", { name: "Search tools" })).toBeFocused();

    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.keyboard.press("Meta+k");
    await expect(page.getByRole("textbox", { name: "Search tools" })).toBeFocused();
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

const chooseFixture = async (page: Page, selectedPath: string) => {
    await page.evaluate((fixture) => {
        (window as TestWindow).__toolboxDialogResults = [fixture];
    }, selectedPath);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(path.basename(selectedPath), { exact: true })).toBeVisible();
};

test("crop is applied visually and exported as part of the scene", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Crop PDF" }).click();
    await chooseFixture(page, pdfFixturePath);
    await drawSceneMark(page, "Crop");
    await page.getByRole("button", { name: "Export PDF", exact: true }).click();
    const invocation = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.find(c => c.command === "export_pdf_scene"));
    expect(invocation?.args).toMatchObject({ request: { scene: { pages: [{ crop: { x: expect.any(Number), width: expect.any(Number) } }] } } });
});

test("pdf editor renders page previews", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Sign PDF" }).click();
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await expect(page.getByRole("img", { name: "Thumbnail of page 1" })).toBeVisible();
});

test("PDF editor inserts editable blank pages from thumbnails", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Organize PDF" }).click();
    await chooseFixture(page, pdfFixturePath);
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
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Colour and Tone Adjustments" }).click();
    await expect(page.getByRole("slider", { name: "Contrast" })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open PDF editor", exact: true }).click();
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Crop", exact: true }).click();
    await expect(page.getByText("Drag a rectangle on the page to crop.", { exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
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
    await chooseFixture(page, pdfFixturePath);
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
    await expect(page.getByText("Edits are append-only. Removing or reordering individual operations is deferred.", { exact: true })).toBeVisible();
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
    await expect(page.locator(".image-edit-timeline li")).toContainText(["Resize 320 × 768", "Rotate 90°"]);
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

test("Image crop cancels a global pointer interruption before a later move", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Crop Images", exact: true }).click();
    await page.getByLabel("Crop width").fill("160");
    await page.getByLabel("Crop height").fill("120");
    await page.getByLabel("Crop left").fill("80");
    await page.getByLabel("Crop top").fill("60");
    await expect(page.getByLabel("Crop selection")).toBeVisible();

    const stage = (await page.locator(".image-editor-preview-stage").boundingBox())!;
    const before = { left: await page.getByLabel("Crop left").inputValue(), top: await page.getByLabel("Crop top").inputValue() };
    await page.locator(".image-editor-preview-stage").dispatchEvent("pointerdown", {
        pointerId: 77, button: 0, clientX: stage.x + stage.width * .25, clientY: stage.y + stage.height * .25,
    });
    await page.evaluate(() => window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 77, bubbles: true })));
    await page.locator(".image-editor-preview-stage").dispatchEvent("pointermove", {
        pointerId: 77, clientX: stage.x + stage.width * .75, clientY: stage.y + stage.height * .75,
    });
    await expect(page.getByLabel("Crop left")).toHaveValue(before.left);
    await expect(page.getByLabel("Crop top")).toHaveValue(before.top);
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
    await expect(page.getByText("1 committed edit", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Redo" })).toBeEnabled();

    await page.getByRole("button", { name: "Redo" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeEnabled();
    await expect(page.getByText("1 committed edit", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Reset edits" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
    await expect(page.getByText("0 committed edits", { exact: true })).toBeVisible();
});

test("PDF to Images identifies source selection and summarizes its non-previewable output", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Open PDF to Images" }).click();
  await chooseFixture(page, pdfFixturePath);
  const pageCard = page.locator(".conversion-page-card");
  await expect(pageCard).toHaveAttribute("data-slot", "card");
  await expect(pageCard.locator('[data-slot="card-content"]')).toHaveCount(1);
  await expect(page.getByRole("region", { name: "Source page selection" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Conversion preview" })).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
  await expect(page.getByLabel("Render DPI")).toBeVisible();
  await expect(page.getByLabel("Output format")).toBeVisible();
  await expect(page.getByText("1 page selected", { exact: true })).toBeVisible();
  await expect(page.getByText("Output preview unavailable. Export renders page 1 at 150 DPI as JPEG image files.", { exact: true })).toBeVisible();

  await page.getByLabel("Render DPI").selectOption("300");
  await page.getByLabel("Output format").selectOption("png");
  await expect(page.getByText("Output preview unavailable. Export renders page 1 at 300 DPI as PNG image files.", { exact: true })).toBeVisible();

  await page.getByRole("checkbox", { name: "Page 1" }).click();
  await expect(page.locator(".workspace-primary-action")).toBeDisabled();
});

test("PDF range splitting rejects a blank range and accepts a range", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Images" }).click();
    await chooseFixture(page, pdfFixturePath);
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
    await expect(page.getByRole("button", { name: `Move ${fixtureName} up` })).toBeDisabled();
    await expect(page.getByRole("button", { name: `Move ${fixtureName} down` })).toBeDisabled();

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

test("non-previewable PDF conversions identify source selection and show deterministic output limits", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Text" }).click();
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("region", { name: "Source page selection" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Conversion preview" })).toHaveCount(0);
    await expect(page.getByText("Output preview unavailable. Export creates a text file from the selected pages.", { exact: true })).toBeVisible();
    await expect(page.getByRole("img", { name: "Source page preview 1" })).toBeVisible();

    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "Extract PDF Pages", exact: true }).click();
    await expect(page.getByText("Output preview unavailable. Export creates a PDF containing the selected pages.", { exact: true })).toBeVisible();

    const rail = page.getByRole("toolbar", { name: "PDF conversion tools" });
    for (const [action, summary] of [
        ["Images to PDF", "Output preview unavailable. Export combines the selected images into one PDF in file order."],
        ["Merge PDF", "Output preview unavailable. Export merges the selected PDFs in file order."],
        ["Split PDF", "Output preview unavailable. Export creates separate PDF files using the selected split mode."],
        ["Compress PDF", "Output preview unavailable. Export creates a compressed PDF copy while preserving page geometry."],
    ] as const) {
        await rail.getByRole("button", { name: action, exact: true }).click();
        await expect(page.getByText(summary, { exact: true })).toBeVisible();
    }
});

test("PDF conversion visibly rejects retained files when an action changes format", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Images to PDF" }).click();
    await chooseFixture(page, fixturePath);

    const rail = page.getByRole("toolbar", { name: "PDF conversion tools" });
    await rail.getByRole("button", { name: "PDF to Text", exact: true }).click();
    await expect(page.getByText("1 file won't run with PDF to Text. It stays in the list for other actions.", { exact: true })).toBeVisible();
    await expect(page.getByRole("option", { name: fixtureName })).toContainText("Won't run");
    await expect(page.getByRole("button", { name: "Export PDF to Text", exact: true })).toBeDisabled();
    const inspections = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_pdf") ?? [],
    );
    expect(inspections).toHaveLength(0);
});

test("PDF page-scoped conversions send the selected pages and keep OCR gated", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Text" }).click();
    await chooseFixture(page, "/local/document.scoped.pdf");
    const pageOne = page.getByRole("checkbox", { name: "Page 1" });
    const pageThree = page.getByRole("checkbox", { name: "Page 3" });
    await expect(pageThree).toBeVisible();
    await page.getByRole("img", { name: "Source page preview 1" }).click();
    await expect(pageOne).not.toBeChecked();
    await pageThree.focus();
    await page.keyboard.press("Space");
    await expect(pageThree).not.toBeChecked();
    await page.getByRole("button", { name: "Export PDF to Text", exact: true }).click();

    const textInvocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "pdf_to_text"),
    );
    expect(textInvocation?.args).toMatchObject({ request: { paths: ["/local/document.scoped.pdf"], pages: [1] } });

    const ocrButton = page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "OCR PDF", exact: true });
    await expect(ocrButton).toBeDisabled();
    await expect(ocrButton).toHaveAttribute("title", "OCR PDF is unavailable in this build");
});

test("TIFF export waits for inspection of the current file set", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Split and Combine TIFF" }).click();
    await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = ["/local/one-page.tiff"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".media-frame-list button")).toHaveCount(1);

    await page.evaluate(() => {
        (window as TestWindow).__toolboxTiffInspectionDelayMs = 250;
        (window as TestWindow).__toolboxDialogResults = ["/local/two-page.tiff"];
    });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".workspace-primary-action")).toBeDisabled();
    await expect(page.locator(".media-frame-list button")).toHaveCount(3);
    const invocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "process_tiff_pages"),
    );
    expect(invocation).toBeUndefined();
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    await page.locator(".workspace-primary-action").click();
    const currentInvocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "process_tiff_pages"),
    );
    expect(currentInvocation?.args).toMatchObject({ request: { pages: [
        { path: "/local/one-page.tiff", page: 0 },
        { path: "/local/two-page.tiff", page: 0 },
        { path: "/local/two-page.tiff", page: 1 },
    ] } });
});

test("shared workspace ignores a stale processing completion after action changes", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("switch", { name: "Preserve original pixels" }).click();
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
    await chooseFixture(page, pdfFixturePath);
    await page.locator('input[type="password"]').fill("test-password");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
});

test("Office protection exposes DOCX and XLSX protection", async ({ page }) => {
    const utility = UtilityRegistry.find((item) => item.id === "office-protect");
    expect(utility).toBeDefined();
    expect(workspaceForTool("office-protect")?.id).toBe("file-security");
    expect(utility?.command).toBe("protect_office");
    expect(utility?.capability.acceptedExtensions).toEqual([
        ".docx", ".xlsx",
    ]);
    expect(utility?.capability.nativeAvailability).toBe("available");
    expect(utility?.status).toBe("implemented");

    await page.goto("/");
    await page.getByRole("button", { name: "Open Protect Office Files" }).click();
    await expect(page.getByRole("heading", { name: "Protect Office Files" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Choose files to process" })).toBeVisible();
    await expect(page.getByText("Add a password to selected DOCX and XLSX files. The originals stay untouched.", { exact: true })).toBeVisible();
    await expect(page.getByText("DOCX and XLSX files only", { exact: true })).toBeVisible();
    await chooseFixture(page, "/local/document.docx");
    await page.locator('input[type="password"]').fill("test-password");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
});

const succeeded = (inputPath: string, outputPaths: string[], extra: Partial<Extract<MockOutcome, { state: "succeeded" }>> = {}): MockOutcome => ({
    inputPath,
    outputPaths,
    detail: "",
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
    ...extra,
});

const failed = (inputPath: string, kind: "invalidInput" | "unavailable" | "limitExceeded" | "processing", message: string, extra: Partial<Extract<MockOutcome, { state: "failed" }>> = {}): MockOutcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind, message, pointer: null },
    reason: null,
    warnings: [],
    ...extra,
});

const runCompressImagesWith = async (page: Page, outcomes: MockOutcome[]) => {
    await page.goto("/");
    await page.evaluate((outcomes) => {
        (window as TestWindow).__toolboxProcessingResults = outcomes;
    }, outcomes);
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("switch", { name: "Preserve original pixels" }).click();
    await page.getByRole("button", { name: "Export edited images" }).click();
    return page.getByRole("list", { name: "Results" }).getByRole("listitem");
};

const resultSummaryStatus = (page: Page) => page.getByRole("status").filter({ hasText: "Done." });

const fileActionInvocations = (page: Page) => page.evaluate(() =>
    ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) =>
        command === "open_output_path" || command === "reveal_output_path"
    ),
);

test("every output exposes native file actions and keeps action errors inline", async ({ page }) => {
    const splitOutputs = ["a-01.pdf", "a-02.pdf", "a-12.pdf"].map((name) => `/local/out/${name}`);
    const folderOutput = "/local/out/brochure-pages";
    const partialOutput = "/local/out/drawings-part-1.pdf";
    const rows = await runCompressImagesWith(page, [
        succeeded(fixturePath, [mockedOutputPaths[0]], { detail: "Test output" }),
        succeeded("/local/annual-report.pdf", splitOutputs),
        succeeded("/local/brochure.pdf", [folderOutput]),
        failed("/local/drawings.pdf", "processing", "Took longer than 5 minutes and was stopped.", { outputPaths: [partialOutput] }),
    ]);
    await page.evaluate((partialOutput) => {
        (window as TestWindow).__toolboxActionFailure = {
            command: "reveal_output_path",
            path: partialOutput,
            message: "Could not reveal test output",
        };
    }, partialOutput);

    await expect(rows).toHaveCount(4);
    await expect(rows.nth(0)).toContainText("icon.png.output-one");
    await expect(rows.nth(0)).toContainText("Test output");
    await expect(rows.nth(1)).toContainText("3 files, a-01.pdf to a-12.pdf");
    await expect(rows.nth(1).getByRole("button", { name: "Open a-01.pdf", exact: true })).toHaveText("Open first");
    await expect(rows.nth(3).getByRole("button", { name: /^Open / })).toHaveCount(0);

    await rows.nth(0).getByRole("button", { name: "Open icon.png.output-one", exact: true }).click();
    await rows.nth(0).getByRole("button", { name: "Show in Finder: icon.png.output-one", exact: true }).click();
    await rows.nth(1).getByRole("button", { name: "Open a-01.pdf", exact: true }).click();
    await rows.nth(1).getByRole("button", { name: "Show in Finder: a-01.pdf", exact: true }).click();
    await rows.nth(2).getByRole("button", { name: "Show in Finder: brochure-pages", exact: true }).click();
    await rows.nth(3).getByRole("button", { name: "Show in Finder: drawings-part-1.pdf", exact: true }).click();

    expect(await fileActionInvocations(page)).toEqual([
        { command: "open_output_path", args: { path: mockedOutputPaths[0] } },
        { command: "reveal_output_path", args: { path: mockedOutputPaths[0] } },
        { command: "open_output_path", args: { path: splitOutputs[0] } },
        { command: "reveal_output_path", args: { path: splitOutputs[0] } },
        { command: "reveal_output_path", args: { path: folderOutput } },
        { command: "reveal_output_path", args: { path: partialOutput } },
    ]);
    await expect(rows.nth(3).getByRole("alert")).toHaveText("Could not reveal test output");
    await expect(page.getByRole("alert")).toHaveCount(1);
    await expect(resultSummaryStatus(page)).toHaveText("Done. 3 compressed, 1 failed");
});

test("result rows show succeeded, skipped and failed states with warnings and pointers in input order", async ({ page }) => {
    const rows = await runCompressImagesWith(page, [
        succeeded("/local/lease-agreement.pdf", ["/local/lease-agreement-protected.pdf"]),
        succeeded("/local/vendor-nda.pdf", ["/local/vendor-nda-protected.pdf"], {
            warnings: [{ code: "signatureRemoved", message: "native wording" }],
        }),
        {
            inputPath: "/local/invoice-0425.pdf",
            outputPaths: [],
            detail: "",
            state: "skipped",
            failure: null,
            reason: { code: "alreadyProtected", message: "Already password-protected. Remove the password first, then protect it again.", pointer: { kind: "tool", toolId: "pdf-unlock" } },
            warnings: [],
        },
        succeeded("/local/menu-spring.docx", ["/local/menu-spring.pdf"], {
            warnings: [
                { code: "fontSubstituted", message: "native wording", details: { requestedFamily: "Brandon Grotesque", actualFamily: "Helvetica" } },
                { code: "renamedToAvoidReplacing", message: "native wording", details: { chosenName: "menu-spring.pdf", finalName: "menu-spring-1.pdf" } },
            ],
        }),
        failed("/local/site-survey-scan.pdf", "invalidInput", "Not a readable PDF. The file may be damaged or incomplete."),
        failed("/local/temple-scan.png", "unavailable", "Recognising Gujarati text needs the Indic script pack.", {
            failure: { kind: "unavailable", message: "Recognising Gujarati text needs the Indic script pack.", pointer: { kind: "pack", packId: "script-pack" } },
        }),
    ]);

    await expect(rows).toHaveCount(6);
    const names = ["lease-agreement.pdf", "vendor-nda.pdf", "invoice-0425.pdf", "menu-spring.docx", "site-survey-scan.pdf", "temple-scan.png"];
    for (const [index, name] of names.entries()) await expect(rows.nth(index)).toContainText(name);
    await expect(rows.nth(0)).toHaveAttribute("data-tone", "ok");
    await expect(rows.nth(1)).toHaveAttribute("data-tone", "warn");
    await expect(rows.nth(2)).toHaveAttribute("data-tone", "skip");
    await expect(rows.nth(4)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(5)).toHaveAttribute("data-tone", "skip");

    await expect(rows.nth(0)).toContainText("lease-agreement-protected.pdf");
    await expect(rows.nth(0)).toContainText("Compressed");
    await expect(rows.nth(1)).toContainText("The digital signature was removed, because protecting a file changes it.");
    await expect(rows.nth(1)).not.toContainText("native wording");
    await expect(rows.nth(1)).toContainText("Compressed with a warning");
    await expect(rows.nth(3)).toContainText("The font Brandon Grotesque wasn’t available, so Helvetica was used.");
    await expect(rows.nth(3)).toContainText("Saved as menu-spring-1.pdf. Renamed to avoid replacing an existing file.");
    await expect(rows.nth(3)).toContainText("Compressed with warnings");
    await expect(rows.nth(2)).toContainText("Skipped");
    await expect(rows.nth(2)).toContainText("Already password-protected.");
    await expect(rows.nth(2).getByRole("button", { name: /^(Open|Show in)/ })).toHaveCount(1);
    await expect(rows.nth(4)).toContainText("Not a readable PDF.");
    await expect(rows.nth(4)).toContainText("Failed, the input can't be used");
    await expect(rows.nth(5)).toContainText("Unavailable");

    await expect(resultSummaryStatus(page)).toHaveAttribute("aria-live", "polite");
    await expect(resultSummaryStatus(page)).toContainText("Done. 3 compressed, 1 skipped, 2 failed");
    await expect(resultSummaryStatus(page)).toContainText("2 compressed files have warnings.");
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

    await page.evaluate(() => {
        window.addEventListener("toolbox:install-pack", (event) => {
            (window as unknown as { __installPackRequests: string[] }).__installPackRequests.push((event as CustomEvent<string>).detail);
        });
        (window as unknown as { __installPackRequests: string[] }).__installPackRequests = [];
    });
    await rows.nth(5).getByRole("button", { name: "Install script pack", exact: true }).click();
    expect(await page.evaluate(() => (window as unknown as { __installPackRequests: string[] }).__installPackRequests)).toEqual(["script-pack"]);

    await rows.nth(2).getByRole("button", { name: "Open Remove Password", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Remove Password", exact: true })).toBeVisible();
});

test("a read-only output folder failure points to the output location setting instead of Retry", async ({ page }) => {
    const rows = await runCompressImagesWith(page, [
        failed("/locked/scan.png", "processing", "The output folder is read-only. Nothing was saved.", {
            failure: { kind: "processing", message: "The output folder is read-only. Nothing was saved.", pointer: { kind: "outputFolder" } },
        }),
        succeeded(fixturePath, [mockedOutputPaths[0]]),
    ]);

    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(0)).toContainText("The output folder is read-only. Nothing was saved.");
    await expect(rows.nth(1)).toHaveAttribute("data-state", "succeeded");
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

    await rows.nth(0).getByRole("button", { name: "Choose output folder", exact: true }).click();
    const settings = page.getByRole("region", { name: "Output location" });
    await expect(settings.getByRole("radio", { name: "Next to each original" })).toBeFocused();
    await expect(page.getByRole("heading", { name: "Output location" })).toBeInViewport();
    await expect(page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Output location" })).toHaveAttribute("aria-pressed", "true");

    await page.keyboard.press("Escape");
    await expect(rows.nth(0)).toContainText("The output folder is read-only. Nothing was saved.");
});

test("Retry is offered only after processing failures and re-runs only that file with the same options", async ({ page }) => {
    const timedOut = "/local/drawings-full-set.pdf";
    const rows = await runCompressImagesWith(page, [
        succeeded(fixturePath, [mockedOutputPaths[0]]),
        failed(timedOut, "processing", "Took longer than 5 minutes and was stopped. Nothing was saved."),
        failed("/local/scan.pdf", "invalidInput", "Wrong password or unsupported encryption."),
        failed("/local/site-photos.zip", "limitExceeded", "Larger than the 4 GB limit for one archive."),
        failed("/local/temple-scan.png", "unavailable", "Needs the script pack."),
    ]);

    await expect(rows).toHaveCount(5);
    const retryButtons = page.getByRole("button", { name: /^Retry/ });
    await expect(retryButtons).toHaveCount(1);
    await expect(rows.nth(1).getByRole("button", { name: "Retry drawings-full-set.pdf", exact: true })).toBeVisible();

    await page.evaluate((timedOut) => {
        (window as TestWindow).__toolboxProcessingResults = [{
            inputPath: timedOut,
            outputPaths: [`${timedOut}.compressed.pdf`],
            detail: "",
            state: "succeeded",
            failure: null,
            reason: null,
            warnings: [],
        }];
    }, timedOut);
    await rows.nth(1).getByRole("button", { name: "Retry drawings-full-set.pdf", exact: true }).click();

    await expect(rows.nth(1)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(1)).toContainText("drawings-full-set.pdf.compressed.pdf");
    await expect(rows).toHaveCount(5);
    await expect(rows.nth(2)).toHaveAttribute("data-state", "failed");
    await expect(retryButtons).toHaveCount(0);
    await expect(resultSummaryStatus(page)).toHaveText("Done. 2 compressed, 3 failed");

    const runs = await page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "export_image_edit_plan"),
    );
    expect(runs).toHaveLength(2);
    const [first, retry] = runs.map((run) => (run.args as { request: { paths: string[]; plan: unknown } }).request);
    expect(first.paths).toEqual([fixturePath]);
    expect(retry.paths).toEqual([timedOut]);
    expect(retry.plan).toEqual(first.plan);
});

test("output action state resets and ignores stale completions", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Compress Images" }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("switch", { name: "Preserve original pixels" }).click();
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
            state: "succeeded",
            failure: null,
            reason: null,
            warnings: [],
        }];
    }, { path: mockedOutputPaths[0] });

    await page.getByRole("button", { name: "Open icon.png.output-one", exact: true }).click();
    await processButton.click();

    await expect(page.getByText("New result set", { exact: true })).toBeVisible();
    await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveCount(1);
    await page.waitForTimeout(200);
    await expect(page.getByText("Stale action failure", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open icon.png.output-one.next-output", exact: true })).toBeEnabled();
});

test("single-input actions report every selected file instead of truncating the selection", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Merge PDF" }).click();
    await chooseFixture(page, pdfFixturePath);
    await page.evaluate(() => { (window as TestWindow).__toolboxSecondPick = true; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Compress PDF", exact: true }).click();

    const exportButton = page.getByRole("button", { name: "Export Compress PDF", exact: true });
    await expect(exportButton).toBeEnabled();
    await exportButton.click();

    await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 0 compressed, 2 failed");
    const processingInvocations = await page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "compress_pdf"),
    );
    expect(processingInvocations).toHaveLength(0);
    await expect(page.getByText("This action accepts one input file, but 2 were selected.", { exact: true })).toHaveCount(2);
});

test("single-input replacement keeps the source and edit state after cancellation or the same path", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Crop PDF" }).click();
    await chooseFixture(page, pdfFixturePath);
    await drawSceneMark(page, "Crop");
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();

    await page.evaluate((path) => { (window as TestWindow).__toolboxDialogResults = [null, path]; }, pdfFixturePath);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await expect(page.getByText(pdfFixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();

    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await expect(page.getByText(pdfFixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeEnabled();
});

test("page-scoped actions disable export when the explicit selection is empty", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Text" }).click();
    await chooseFixture(page, pdfFixturePath);
    const pageCheckbox = page.getByRole("checkbox", { name: "Page 1" });
    await expect(pageCheckbox).toBeChecked();
    await pageCheckbox.click();
    await expect(page.getByRole("button", { name: "Export PDF to Text", exact: true })).toBeDisabled();
});

test("PDF replacement clears stale inspection state while inspection is pending or rejected", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Extract PDF Pages" }).click();
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeEnabled();

    await page.evaluate(() => { (window as TestWindow).__toolboxInspectionDelayMs = 250; });
    await chooseFixture(page, "/local/document.replacement.pdf");

    await expect(page.getByRole("checkbox", { name: "Page 1" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeDisabled();
    await expect(page.getByRole("checkbox", { name: "Page 2" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeEnabled();

    await page.evaluate(() => { (window as TestWindow).__toolboxInspectionError = "inspection rejected"; });
    await chooseFixture(page, "/local/document.rejected.pdf");

    await expect(page.getByRole("checkbox", { name: "Page 2" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Export Extract PDF Pages", exact: true })).toBeDisabled();
    await expect(page.getByText("inspection rejected", { exact: true })).toBeVisible();
});

test("PDF conversion resets the displayed scope when switching actions", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open PDF to Images" }).click();
    await chooseFixture(page, "/local/document.scoped.pdf");
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
    await chooseFixture(page, "/local/animation.gif");
    await expect(page.getByLabel("Tool detail").getByRole("button", { name: "Frames", exact: true })).toBeEnabled();

    const previewInvocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_image_preview") ?? []);
    expect(previewInvocations).toHaveLength(0);
});

type RunRequest = { outputLocation?: unknown; plan?: { outputLocation?: unknown } };

const storeOutputSettings = async (page: Page, settings: OutputSettings) => {
    await page.addInitScript((settings) => { (window as TestWindow).__toolboxOutputSettings = settings; }, settings);
};

const exerciseFeature = async (page: Page, utility: (typeof UtilityRegistry)[number], expectedOutputLocation: unknown = "alongsideInput") => {
    await page.goto("/");
    await page.getByRole("button", { name: `Open ${utility.title}` }).click();

    const accepts = (extension: string) => utility.capability.acceptedExtensions.some((accepted) => accepted.toLowerCase() === extension);
    const selectedFixturePath = utility.id === "office-protect"
        ? "/local/document.docx"
        : accepts(".pdf")
            ? pdfFixturePath
            : accepts(".png")
                ? fixturePath
                : `/local/fixture${utility.capability.acceptedExtensions[0]}`;
    await chooseFixture(page, selectedFixturePath);

    if (utility.id === "pdf-unlock" || utility.id === "pdf-protect" || utility.id === "office-protect") {
        await page.locator('input[type="password"]').fill("test-password");
    }
    if (workspaceForTool(utility.id)?.id === "pdf-editor") {
        if (utility.id === "pdf-page-numbers") await page.getByRole("button", { name: "Page numbers", exact: true }).click();
        else if (utility.id === "pdf-organize") await page.getByRole("button", { name: "Add blank page", exact: true }).click();
        else if (utility.id === "pdf-remove-pages") {
            await page.getByRole("button", { name: "Add blank page", exact: true }).click();
            await page.getByRole("button", { name: "Page 1", exact: true }).click();
            await page.getByRole("button", { name: "Delete selected pages", exact: true }).click();
        } else if (utility.id === "pdf-watermark") await page.getByRole("button", { name: "Add fixed watermark", exact: true }).click();
        else await drawSceneMark(page, utility.id === "pdf-sign" ? "Signature" : utility.id === "pdf-watermark" ? "Watermark" : utility.id === "pdf-crop" ? "Crop" : "Text");
    }
    if (workspaceForTool(utility.id)?.id === "image-editor") {
        if (utility.id === "heic-convert") await page.getByLabel("Target format").selectOption("jpg");
        if (utility.id === "compress") await page.getByRole("switch", { name: "Preserve original pixels" }).click();
        if (utility.id === "resize") await page.getByLabel("Width").fill("640");
        if (utility.id === "rotate") await page.getByLabel("Rotation").selectOption("90");
        if (utility.id === "crop") await page.getByLabel("Crop width").fill("320");
        if (utility.id === "image-watermark") await page.getByLabel("Watermark text").fill("Test watermark");
        if (utility.id === "image-tone") await page.getByRole("slider", { name: "Brightness" }).press("End");
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
    const run = invocations.find(({ command }) => command === composedCommand);
    expect(run).toBeDefined();
    const request = (run?.args as { request: RunRequest }).request;
    expect(composedCommand === "export_image_edit_plan" ? request.plan?.outputLocation : request.outputLocation).toEqual(expectedOutputLocation);
};

test.describe("registered feature actions", () => {
    expect(UtilityRegistry).toHaveLength(33);

    for (const utility of UtilityRegistry) {
        test(`${utility.id} accepts the fixture and runs with the Settings default`, async ({ page }) => {
            test.skip(utility.status !== "implemented", "Tool is not available in this build.");
            await exerciseFeature(page, utility);
        });
    }
});

test.describe("workspace runs send the stored output location", () => {
    const representativeTools = ["pdf-protect", "pdf-edit", "pdf-to-images", "resize", "icon-set"] as const;
    const customFolder = { customFolder: "/fake/shell/output" };

    test("one representative tool covers every workspace view", () => {
        expect(representativeTools.map((id) => workspaceForTool(id)?.id)).toEqual(["file-security", "pdf-editor", "pdf-convert", "image-editor", "media-tools"]);
    });

    for (const id of representativeTools) {
        const utility = UtilityRegistry.find((item) => item.id === id)!;
        test(`${workspaceForTool(id)?.id} sends the stored custom folder from ${id}`, async ({ page }) => {
            await storeOutputSettings(page, { mode: "customFolder", folder: customFolder.customFolder });
            await exerciseFeature(page, utility, customFolder);
        });
    }

    test("a stored folder keeps its path while the mode is alongside", async ({ page }) => {
        await storeOutputSettings(page, { mode: "alongsideInput", folder: "/fake/shell/output" });
        await exerciseFeature(page, UtilityRegistry.find((item) => item.id === "resize")!, "alongsideInput");
    });
});

const openOutputSettings = async (page: Page) => {
    await page.goto("/");
    const dialog = await openSettingsSection(page, "Output location");
    return { dialog, saveResults: dialog.getByRole("radiogroup", { name: "Save results" }) };
};

const outputSettingsWrites = (page: Page) => page.evaluate(() =>
    ((window as TestWindow).__toolboxInvocations ?? [])
        .filter(({ command }) => command === "set_output_settings")
        .map(({ args }) => (args as { settings: OutputSettings }).settings),
);

test.describe("Settings output location", () => {
    test("defaults to next to each original and shows the default folder", async ({ page }) => {
        const { dialog, saveResults } = await openOutputSettings(page);
        await expect(page.getByRole("heading", { level: 1, name: "Output location" })).toBeVisible();
        await expect(saveResults.getByRole("radio", { name: "Next to each original" })).toBeChecked();
        await expect(saveResults.getByRole("radio", { name: "Always in one folder" })).not.toBeChecked();
        await expect(dialog.getByText("~/Documents/Toolbox", { exact: true })).toBeVisible();
        await expect(dialog.getByText("~/Documents/Toolbox", { exact: true })).toHaveAttribute("title", "/Users/test/Documents/Toolbox");
        await expect(dialog.getByText("Always on", { exact: true })).toBeVisible();
        expect(await outputSettingsWrites(page)).toEqual([]);
    });

    test("saves the folder dialog result through the settings command and runs use it", async ({ page }) => {
        const { dialog, saveResults } = await openOutputSettings(page);
        await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = ["/picked/output"]; });
        await dialog.getByRole("button", { name: "Choose output folder" }).click();

        await expect(saveResults.getByRole("radio", { name: "Always in one folder" })).toBeChecked();
        await expect(dialog.getByText("/picked/output", { exact: true })).toBeVisible();
        expect(await outputSettingsWrites(page)).toEqual([{ mode: "customFolder", folder: "/picked/output" }]);
        const dialogOpen = await page.evaluate(() =>
            ((window as TestWindow).__toolboxInvocations ?? []).find(({ command }) => command === "plugin:dialog|open")?.args as { options: { directory: boolean; defaultPath?: string } },
        );
        expect(dialogOpen.options).toMatchObject({ directory: true, defaultPath: "/Users/test/Documents/Toolbox" });

        await page.keyboard.press("Escape");
        await page.getByRole("button", { name: "Open Resize Images" }).click();
        await chooseFixture(page, fixturePath);
        await page.getByLabel("Width").fill("640");
        await page.getByRole("button", { name: "Add edit to plan" }).click();
        await page.locator(".workspace-primary-action").click();
        await expect(page.getByText("Test output", { exact: true })).toBeVisible();
        const run = await page.evaluate(() =>
            ((window as TestWindow).__toolboxInvocations ?? []).find(({ command }) => command === "export_image_edit_plan")?.args as { request: RunRequest },
        );
        expect(run.request.plan?.outputLocation).toEqual({ customFolder: "/picked/output" });
    });

    test("a cancelled folder dialog saves nothing", async ({ page }) => {
        const { dialog, saveResults } = await openOutputSettings(page);
        await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = [null]; });
        await dialog.getByRole("button", { name: "Choose output folder" }).click();
        await expect.poll(() => page.evaluate(() => ((window as TestWindow).__toolboxInvocations ?? []).some(({ command }) => command === "plugin:dialog|open"))).toBe(true);
        expect(await outputSettingsWrites(page)).toEqual([]);
        await expect(saveResults.getByRole("radio", { name: "Next to each original" })).toBeChecked();
    });

    test("switching modes saves through the settings command and keeps the folder", async ({ page }) => {
        const { saveResults } = await openOutputSettings(page);
        await saveResults.getByRole("radio", { name: "Always in one folder" }).click();
        await expect(saveResults.getByRole("radio", { name: "Always in one folder" })).toBeChecked();
        await saveResults.getByRole("radio", { name: "Next to each original" }).click();
        await expect(saveResults.getByRole("radio", { name: "Next to each original" })).toBeChecked();
        expect(await outputSettingsWrites(page)).toEqual([
            { mode: "customFolder", folder: "/Users/test/Documents/Toolbox" },
            { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" },
        ]);
    });

    test("only a path inside the home folder is shortened", async ({ page }) => {
        await storeOutputSettings(page, { mode: "customFolder", folder: "/Users/tester/output" });
        const { dialog } = await openOutputSettings(page);
        await expect(dialog.getByText("/Users/tester/output", { exact: true })).toBeVisible();
    });

    test("the stored setting loads at startup", async ({ page }) => {
        await storeOutputSettings(page, { mode: "customFolder", folder: "/kept/output" });
        const { dialog, saveResults } = await openOutputSettings(page);
        await expect(saveResults.getByRole("radio", { name: "Always in one folder" })).toBeChecked();
        await expect(dialog.getByText("/kept/output", { exact: true })).toBeVisible();
    });

    test("a rejected save shows the native error and keeps the previous choice", async ({ page }) => {
        await page.addInitScript(() => {
            const internals = window.__TAURI_INTERNALS__ as { invoke: (command: string, args: unknown) => Promise<unknown> };
            const invoke = internals.invoke;
            internals.invoke = async (command, args) => {
                if (command === "set_output_settings") throw "Could not use /picked/output as the output folder: Permission denied";
                return invoke(command, args);
            };
        });
        const { dialog, saveResults } = await openOutputSettings(page);
        await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = ["/picked/output"]; });
        await dialog.getByRole("button", { name: "Choose output folder" }).click();
        await expect(dialog.getByRole("alert")).toHaveText("Could not use /picked/output as the output folder: Permission denied");
        await expect(saveResults.getByRole("radio", { name: "Next to each original" })).toBeChecked();
    });
});

const runRequests = (page: Page) => page.evaluate(() =>
    ((window as TestWindow).__toolboxInvocations ?? [])
        .filter(({ command }) => command === "export_image_edit_plan" || command === "protect_pdf")
        .map(({ command, args }) => {
            const request = (args as { request: RunRequest }).request;
            return command === "export_image_edit_plan" ? request.plan?.outputLocation : request.outputLocation;
        }));

const openResizeWithFixture = async (page: Page) => {
    await page.getByRole("button", { name: "Open Resize Images" }).click();
    await chooseFixture(page, fixturePath);
    await page.getByLabel("Width").fill("640");
    await page.getByRole("button", { name: "Add edit to plan" }).click();
};

const runAndWait = async (page: Page) => {
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByText("Test output", { exact: true }).last()).toBeVisible();
};

const changeForRun = async (page: Page, folder: string) => {
    await page.evaluate((folder) => { (window as TestWindow).__toolboxDialogResults = [folder]; }, folder);
    await page.getByRole("region", { name: "Destination" }).getByRole("button", { name: "Change for this run" }).click();
};

test.describe("per-run destination", () => {
    test("shows the default destination and an example name built from the declared suffix", async ({ page }) => {
        const resize = UtilityRegistry.find((item) => item.id === "resize")!;
        await page.goto("/");
        await page.getByRole("button", { name: "Open Resize Images" }).click();
        const destination = page.getByRole("region", { name: "Destination" });
        await expect(destination.getByTestId("run-destination-path")).toHaveText("Next to each original");
        await expect(destination.getByTestId("run-destination-example")).toHaveCount(0);
        await chooseFixture(page, fixturePath);
        await expect(destination.getByTestId("run-destination-example")).toHaveText(`icon${resize.capability.outputSuffix}.png`);
        await expect(destination.getByTestId("run-destination-example")).toHaveText("icon-resized.png");
        await expect(destination.getByText("An existing name gets -1, -2 and so on. Nothing is replaced.")).toBeVisible();
        await expect(destination.getByText("This run only")).toHaveCount(0);
    });

    test("a per-run folder in the home folder is shown with ~ and sent in full", async ({ page }) => {
        await page.goto("/");
        await openResizeWithFixture(page);
        const destination = page.getByRole("region", { name: "Destination" });
        await changeForRun(page, "/Users/test/Documents/Protected");
        await expect(destination.getByTestId("run-destination-path")).toHaveText("~/Documents/Protected");
        await runAndWait(page);
        expect(await runRequests(page)).toEqual([{ customFolder: "/Users/test/Documents/Protected" }]);
    });

    test("a per-run folder is sent in the request, marked, reset with one action, and never written to Settings", async ({ page }) => {
        await page.goto("/");
        await openResizeWithFixture(page);
        const destination = page.getByRole("region", { name: "Destination" });
        await changeForRun(page, "/picked/run");

        await expect(destination.getByText("This run only")).toBeVisible();
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/picked/run");
        await expect(destination.getByText("Your default is next to each original. This change ends when you leave Resize Images.")).toBeVisible();
        const folderDialog = await page.evaluate(() =>
            ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "plugin:dialog|open").pop()?.args as { options: { directory: boolean } },
        );
        expect(folderDialog.options).toMatchObject({ directory: true, multiple: false });

        await runAndWait(page);
        expect(await runRequests(page)).toEqual([{ customFolder: "/picked/run" }]);
        expect(await outputSettingsWrites(page)).toEqual([]);

        await destination.getByRole("button", { name: "Reset to default" }).click();
        await expect(destination.getByText("This run only")).toHaveCount(0);
        await expect(destination.getByTestId("run-destination-path")).toHaveText("Next to each original");
        await runAndWait(page);
        await expect.poll(() => runRequests(page)).toEqual([{ customFolder: "/picked/run" }, "alongsideInput"]);
        expect(await outputSettingsWrites(page)).toEqual([]);
        expect(await page.evaluate(() => (window as TestWindow).__toolboxOutputSettings)).toBeUndefined();
    });

    test("leaving and returning to the view shows the default destination again", async ({ page }) => {
        await storeOutputSettings(page, { mode: "customFolder", folder: "/kept/output" });
        await page.goto("/");
        await page.getByRole("button", { name: "Open Resize Images" }).click();
        const destination = page.getByRole("region", { name: "Destination" });
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/kept/output");
        await changeForRun(page, "/picked/run");
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/picked/run");
        await expect(destination.getByText("Your default is /kept/output. This change ends when you leave Resize Images.")).toBeVisible();

        await page.getByRole("button", { name: "← All tools" }).click();
        await expect(destination).toHaveCount(0);
        await page.getByRole("button", { name: "Open Resize Images" }).click();
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/kept/output");
        await expect(destination.getByText("This run only")).toHaveCount(0);
        expect(await outputSettingsWrites(page)).toEqual([]);
    });

    test("switching to another tool in the workspace resets the per-run folder", async ({ page }) => {
        await page.goto("/");
        await page.getByRole("button", { name: "Open Resize Images" }).click();
        const destination = page.getByRole("region", { name: "Destination" });
        await changeForRun(page, "/picked/run");
        await expect(destination.getByText("This run only")).toBeVisible();

        await page.getByRole("toolbar", { name: "Image editor tools" }).getByRole("button", { name: "Rotate and Flip Images" }).click();
        await expect(destination.getByTestId("run-destination-path")).toHaveText("Next to each original");
        await expect(destination.getByText("This run only")).toHaveCount(0);
    });

    for (const check of ["missing", "unwritable"] as const) {
        test(`${check === "missing" ? "a missing" : "an unwritable"} custom folder blocks the run and offers another folder`, async ({ page }) => {
            await storeOutputSettings(page, { mode: "customFolder", folder: "/gone/Protected" });
            await page.addInitScript((check) => { (window as TestWindow).__toolboxFolderChecks = { "/gone/Protected": check }; }, check);
            await page.goto("/");
            await openResizeWithFixture(page);
            await page.locator(".workspace-primary-action").click();

            const prompt = page.getByRole("alertdialog");
            await expect(prompt.getByRole("heading")).toHaveText(check === "missing" ? "The folder Protected is missing" : "The folder Protected can't be written to");
            await expect(prompt).toContainText("/gone/Protected");
            await expect(prompt).toContainText("Nothing has run yet.");
            expect(await runRequests(page)).toEqual([]);

            await prompt.getByRole("button", { name: "Cancel" }).click();
            await expect(prompt).toHaveCount(0);
            expect(await runRequests(page)).toEqual([]);

            await page.locator(".workspace-primary-action").click();
            await expect(prompt).toBeVisible();
            await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = ["/picked/another"]; });
            await prompt.getByRole("button", { name: "Choose another folder" }).click();
            await expect(page.getByText("Test output", { exact: true }).last()).toBeVisible();
            expect(await runRequests(page)).toEqual([{ customFolder: "/picked/another" }]);
            const destination = page.getByRole("region", { name: "Destination" });
            await expect(destination.getByTestId("run-destination-path")).toHaveText("/picked/another");
            await expect(destination.getByText("This run only")).toBeVisible();
            expect(await outputSettingsWrites(page)).toEqual([]);
        });
    }

    test("a blocked run can save next to the originals instead", async ({ page }) => {
        await storeOutputSettings(page, { mode: "customFolder", folder: "/gone/Protected" });
        await page.addInitScript(() => { (window as TestWindow).__toolboxFolderChecks = { "/gone/Protected": "missing" }; });
        await page.goto("/");
        await page.getByRole("button", { name: "Open Protect PDF" }).click();
        await chooseFixture(page, pdfFixturePath);
        await expect(page.getByRole("region", { name: "Destination" }).getByTestId("run-destination-example")).toHaveText("document-protected.pdf");
        await page.locator('input[type="password"]').fill("test-password");
        await page.locator(".workspace-primary-action").click();
        await page.getByRole("alertdialog").getByRole("button", { name: "Save next to originals" }).click();
        await expect(page.getByText("Test output", { exact: true }).last()).toBeVisible();
        expect(await runRequests(page)).toEqual(["alongsideInput"]);
        const destination = page.getByRole("region", { name: "Destination" });
        await expect(destination.getByText("This run only")).toBeVisible();
        await expect(destination.getByTestId("run-destination-path")).toHaveText("Next to each original");
        expect(await outputSettingsWrites(page)).toEqual([]);
    });
});
