import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import type { JobOutcome, LimitKind, OutcomePointer, OutputSettings } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";
import { ToolWorkspaceRegistry, UtilityRegistry, isPdfSceneTool, toolsForWorkspace, workspaceEntries, workspaceForTool } from "../../src/registry";
import { RetiredToolIds } from "../../src/registry/retiredToolIds";
import { displayedBoxToPdfRect } from "../../src/features/pdf-editor/geometry";

// Differs from package.json: Settings must show the version the Tauri runtime reports.
const appVersion = "9.4.2";

// Lock asks for the new password twice, so fill every password field in the options panel.
const fillPasswords = async (page: Page, password: string) => {
    for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await field.fill(password);
};

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
    __toolboxNoticesFailure?: string;
    __toolboxSecondPick?: boolean;
    __toolboxDialogResults?: Array<string | string[] | null>;
    __toolboxOutputSettings?: OutputSettings;
    __toolboxFolderChecks?: Record<string, "ready" | "missing" | "unwritable">;
    __toolboxLastSaveFolder?: string;
};

const confirmSave = async (page: Page) => {
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect(dialog).toHaveCount(0);
};

const expandViews = async (page: Page) => {
    const more = page.locator(".view-more-tools");
    while (await more.count()) await more.first().click();
};

const openFromHome = async (page: Page, title: string) => {
    if (UtilityRegistry.some((tool) => tool.title === title)) return openToolBySearch(page, title);
    await expandViews(page);
    await page.getByRole("button", { name: `Open ${title}`, exact: true }).click();
};

const defaultOutputSettings: OutputSettings ={ mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ fixturePath, pdfFixturePath, mockedOutputPaths, defaultOutputSettings, appVersion }) => {
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

                if (command === "get_quick_actions") {
                    return [
                        { id: "compress-pdf", title: "Compress PDF", enabled: true },
                        { id: "remove-background", title: "Remove Background", enabled: true },
                    ];
                }
                if (command === "set_quick_actions") return null;
                if (command === "plugin:app|version") return appVersion;
                if (command === "get_output_settings") return (window as TestWindow).__toolboxOutputSettings ?? defaultOutputSettings;
                if (command === "set_output_settings") {
                    const { settings } = args as { settings: OutputSettings };
                    (window as TestWindow).__toolboxOutputSettings = settings;
                    return settings;
                }
                if (command === "get_last_save_folder") return (window as TestWindow).__toolboxLastSaveFolder ?? "/Users/test/Documents";
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
                if (command === "open_third_party_notices") {
                    const failure = (window as TestWindow).__toolboxNoticesFailure;
                    if (failure) throw failure;
                    return null;
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
if (command === "render_pdf_source_thumbnails") return (args as { request: { pageIndices: number[] } }).request.pageIndices.map((pageIndex) => ({ pageIndex, thumbnail: { dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'/%3E", width: 247, height: 320 }, error: null }));
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
                if (command === "inspect_image_metadata") {
                    return (args as { request: { paths: string[] } }).request.paths.map((path) => ({
                        status: "report", path, format: "png", exif: true, gps: false, xmp: false, iptc: false, comments: false, icc: true, orientation: 6,
                    }));
                }
                if (command === "inspect_pdf_privacy") {
                    const none = { title: null, author: null, subject: null, keywords: null, creator: null, producer: null, creationDate: null, modificationDate: null };
                    const report = { pageCount: 1, documentInformation: none, xmp: { summary: null, conflicts: [] }, encryption: null, identifiers: { documentId: null, producer: null }, incrementalUpdates: 0, embeddedFiles: [], actions: [], annotations: [], filledFormFields: [], optionalContent: { layers: [], hidden: [] }, links: [], signatures: [], leftovers: { thumbnailPages: [], pagePieces: [] }, unsupported: [] };
                    return (args as { request: { paths: string[] } }).request.paths.map((inputPath) => ({ inputPath, outputPaths: [], detail: JSON.stringify(report), state: "succeeded", failure: null, reason: null, warnings: [] }));
                }
                if (command === "read_pdf_properties") {
                    const none = { title: null, author: null, subject: null, keywords: null, creator: null, producer: null, creationDate: null, modificationDate: null };
                    return (args as { request: { paths: string[] } }).request.paths.map((inputPath) => ({ inputPath, outputPaths: [], detail: JSON.stringify(none), state: "succeeded", failure: null, reason: null, warnings: [] }));
                }
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
    }, { fixturePath, pdfFixturePath, mockedOutputPaths, defaultOutputSettings, appVersion });
});

test("every registered feature opens its detail pane", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".topbar--home").getByRole("textbox", { name: "Search tools" })).toBeVisible();

    for (const [index, utility] of UtilityRegistry.entries()) {
        if (index > 0) {
            await page.getByRole("button", { name: "← All tools" }).click();
        }

        await openToolBySearch(page, utility.title);
        if (utility.status === "unavailable") {
            await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
            await expect(page.getByText("This tool requires an offline vision resource that is not bundled. Files will not be selected or processed.")).toBeVisible();
        } else {
            await expect(page.getByRole("heading", { name: workspaceForTool(utility.id)?.title, exact: true })).toBeVisible();
        }
        await expect(page.locator('p.sr-only[role="status"]')).toHaveText(`${workspaceForTool(utility.id)?.title ?? utility.title} workspace open.`);
    }
});

test("sidebar views pair design icons with labels and open each view", async ({ page }) => {
    await page.goto("/");
    const views = page.getByRole("navigation", { name: "Views" });
    await expect(views.getByRole("button")).toHaveText(ToolWorkspaceRegistry.map((workspace) => workspace.title));
    await expect(views.locator('.rail-link-icon[aria-hidden="true"]')).toHaveCount(ToolWorkspaceRegistry.length);

    const glyph = await views.locator(".rail-link-icon").first().evaluate((node) => node.getBoundingClientRect());
    expect(glyph.width).toBeGreaterThanOrEqual(15);
    expect(glyph.height).toBeGreaterThanOrEqual(15);

    for (const workspace of ToolWorkspaceRegistry) {
        await views.getByRole("button", { name: workspace.title, exact: true }).click();
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);
        await page.getByRole("button", { name: "← All tools" }).first().click();
    }
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
    await openFromHome(page, utility.title);

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

test("favorites navigation shows saved tools as chips and as a filtered library", async ({ page }) => {
    await page.goto("/");
    const workspaceNav = page.locator('nav[aria-label="Workspace navigation"]');
    const chips = page.getByRole("group", { name: "Favorite tools" });

    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.getByText("No favorite tools yet", { exact: true })).toBeVisible();
    await expect(workspaceNav.getByRole("button", { name: "Favorites" })).toHaveAttribute("aria-current", "page");
    await expect(chips).toHaveCount(0);

    await workspaceNav.getByRole("button", { name: "All tools" }).click();
    await openFromHome(page, "Lock & Unlock");
    await page.getByRole("button", { name: "Add Lock & Unlock to favorites" }).click();
    await page.getByRole("button", { name: "← All tools" }).click();
    await expect(chips.getByRole("button")).toHaveText(["Lock & Unlock"]);
    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator("button[data-command-result]")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Lock & Unlock" })).toBeVisible();

    await page.reload();
    await expect(chips.getByRole("button")).toHaveText(["Lock & Unlock"]);
    await chips.getByRole("button", { name: "Lock & Unlock" }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("toolbox-recent") ?? "[]"));
    expect(stored[0]).toBe("lock-unlock");
});

test("saved favorites and recents survive the startup id migration and only show registered tools", async ({ page }) => {
    await page.addInitScript(() => {
        localStorage.setItem("toolbox-favorites", JSON.stringify(["pdf-merge", "mystery-tool"]));
        localStorage.setItem("toolbox-recent", JSON.stringify(["mystery-tool", "pdf-unlock"]));
    });
    await page.goto("/");
    const workspaceNav = page.locator('nav[aria-label="Workspace navigation"]');

    await expect(page.getByRole("group", { name: "Favorite tools" }).getByRole("button")).toHaveText(["Merge PDF"]);
    await workspaceNav.getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator("button[data-command-result]")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Merge PDF" })).toBeVisible();

    const stored = await page.evaluate(() => ({
        favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null"),
        recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null"),
        marker: localStorage.getItem("toolbox-tool-id-migration"),
    }));
    expect(stored).toEqual({ favorites: ["pdf-merge"], recent: ["lock-unlock"], marker: String(RetiredToolIds.version) });
});

test("all six home view sections list their tools and open their view", async ({ page }) => {
    await page.goto("/");
    const sections = page.locator(".view-section");
    await expect(sections.locator("h2")).toHaveText(ToolWorkspaceRegistry.map((workspace) => workspace.title));
    await expandViews(page);

    for (const [index, workspace] of ToolWorkspaceRegistry.entries()) {
        const titles = workspaceEntries(workspace, toolsForWorkspace(workspace)).map((entry) =>
            entry.tools[0].capability.nativeAvailability === "unavailable" ? `${entry.title}Unavailable` : entry.title);
        await expect(sections.nth(index).locator("button[data-command-result]")).toHaveText(titles);
    }

    for (const workspace of ToolWorkspaceRegistry) {
        await page.getByRole("button", { name: `Open ${workspace.title}`, exact: true }).click();
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);
        await page.getByRole("button", { name: "← All tools" }).first().click();
    }
});

test("home sections show six tools and reveal the rest on request", async ({ page }) => {
    await page.goto("/");
    for (const [index, workspace] of ToolWorkspaceRegistry.entries()) {
        const section = page.locator(".view-section").nth(index);
        const entries = workspaceEntries(workspace, toolsForWorkspace(workspace)).length;
        const hidden = entries - Math.min(6, entries);
        await expect(section.locator("button[data-command-result]")).toHaveCount(entries - hidden);
        await expect(section.locator(".view-more-tools")).toHaveCount(hidden > 0 ? 1 : 0);
        if (hidden > 0) {
            await expect(section.locator(".view-more-tools")).toHaveText(`${hidden} more ${hidden === 1 ? "tool" : "tools"}`);
            await section.locator(".view-more-tools").click();
            await expect(section.locator("button[data-command-result]")).toHaveCount(entries);
        }
    }
});

test("sidebar brand shows the app icon instead of a letter", async ({ page }) => {
    await page.goto("/");
    const lockup = page.locator(".brand-lockup");
    await expect(lockup).toHaveText("Toolbox");
    const mark = lockup.locator("img.brand-mark");
    await expect(mark).toHaveAttribute("alt", "");
    await expect.poll(() => mark.evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth)).toBeGreaterThan(0);
});

test("home marks tools this build cannot run and keeps them out of the search hint", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("textbox", { name: "Search tools" })).toHaveAttribute("placeholder", "Find a tool, like merge, split or compress");
    await expandViews(page);

    const unavailable = UtilityRegistry.filter((tool) => tool.capability.nativeAvailability === "unavailable");
    expect(unavailable.map((tool) => tool.title)).toEqual(expect.arrayContaining(["Blur Faces", "Remove Background"]));
    expect(unavailable.map((tool) => tool.title)).not.toContain("OCR PDF");
    for (const tool of unavailable) {
        const link = page.locator(`.view-tool-link[data-command-result="${tool.id}"]`);
        await expect(link).toContainText("Unavailable");
        await expect(link).toHaveAttribute("title", `${tool.title} is unavailable in this build`);
        await expect(link).toHaveAccessibleDescription("Unavailable");
    }

    const available = page.locator('.view-tool-link[data-command-result="pdf-merge"]');
    await expect(available).not.toContainText("Unavailable");
    await expect(available).not.toHaveAttribute("aria-describedby", /.+/);

    const ocr = page.locator('.view-tool-link[data-command-result="pdf-ocr"]');
    await expect(ocr).not.toContainText("Unavailable");
    await expect(ocr).not.toHaveAttribute("aria-describedby", /.+/);

    await page.locator('.view-tool-link[data-command-result="image-blur-faces"]').click();
    await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
});

test(`search finds and opens all ${UtilityRegistry.length} registered tool actions`, async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });
    expect(UtilityRegistry).toHaveLength(37);

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
    await openFromHome(page, "Lock & Unlock");
    await page.getByRole("button", { name: "← All tools" }).click();
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
    const composingFindPrevented = await search.evaluate((input) => {
        const event = new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true, cancelable: true, isComposing: true });
        input.dispatchEvent(event);
        return event.defaultPrevented;
    });
    expect(composingFindPrevented).toBe(false);
    await expect(search).toBeFocused();
    await expect(page.locator("button[data-command-result]")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Lock & Unlock", exact: true })).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Open Protect PDF", exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open Protect Office Files", exact: true })).toHaveCount(0);

    await search.fill("Compress");
    await expect(page.locator("button[data-command-result]")).toHaveCount(2);
    const compressFiles = page.getByRole("button", { name: "Open Compress", exact: true });
    const compressImages = page.getByRole("button", { name: "Open Compress Images", exact: true });

    await page.keyboard.press("ArrowDown");
    await expect(compressFiles).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(compressImages).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(compressFiles).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(search).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await expect(compressImages).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath("phase2-command-center-search.png") });
    await page.keyboard.press("Enter");
    await expect(page.locator(".workspace-title")).toHaveText("Compress Files");

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
    await expect(page.getByRole("heading", { level: 1, name: "Tool library" })).toBeAttached();
    await page.screenshot({ path: testInfo.outputPath("home-topbar-desktop.png") });

    await openFromHome(page, "Lock & Unlock");
    await expect(page.locator(".topbar--home")).toHaveCount(0);
    await expect(page.locator(".topbar").getByText("On-device workspace", { exact: true })).toBeVisible();
    await expect(page.locator(".topbar").getByRole("textbox", { name: "Search tools" })).toHaveCount(0);

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.setViewportSize({ width: 700, height: 900 });
    await expect(page.locator(".topbar--home").getByRole("textbox", { name: "Search tools" })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("home-topbar-narrow.png") });
});

test("view sections expose an accessible open action and keep favoriting in the tool view", async ({ page }) => {
    await page.goto("/");
    const section = page.locator(".view-section").first();
    await expect(section).not.toHaveAttribute("tabindex");
    await expect(section.getByRole("button", { name: /favorites/ })).toHaveCount(0);
    const openWorkspace = section.getByRole("button", { name: "Open File Security", exact: true });
    await expect(openWorkspace).toHaveText("Open view");
    await openWorkspace.click();
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Add Lock & Unlock to favorites" })).toBeVisible();
});

test("Windows labels the reveal action for File Explorer", async ({ page }) => {
    await page.addInitScript(() => {
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" });
    });
    await page.goto("/");
    await openFromHome(page, "Compress Images");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByRole("button", { name: "Show in File Explorer: icon.png.output-one", exact: true })).toHaveText("Show in File Explorer");
    await expect(page.getByRole("button", { name: /Show in Finder/ })).toHaveCount(0);
});

test("settings checks for and installs available updates", async ({ page }) => {
    await page.goto("/");
    const updates = await openSettingsSection(page, "Updates");
    await expect(updates.getByRole("status")).toHaveText(`Check for updatesVersion ${appVersion}.`);
    await updates.getByRole("button", { name: "Check now" }).click();
    await expect(updates.getByRole("status")).toHaveText(`Toolbox is up to dateVersion ${appVersion}.`);
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

test("settings lets the user choose which quick actions appear in the file manager", async ({ page }) => {
    await page.goto("/");
    const group = (await openSettingsSection(page, "Quick actions")).getByRole("group", { name: "Quick actions" });
    const compress = group.getByRole("checkbox", { name: "Compress PDF" });
    await expect(compress).toBeChecked();
    await expect(group.getByRole("checkbox", { name: "Remove Background" })).toBeChecked();

    await compress.click();
    await expect(compress).not.toBeChecked();
    const saved = await page.evaluate(() => (window as TestWindow).__toolboxInvocations!.filter((call) => call.command === "set_quick_actions").map((call) => call.args));
    expect(saved).toEqual([{ ids: ["remove-background"] }]);
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
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    const rail = page.getByRole("toolbar", { name: "Image Editor tools" });
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

test("each view rail lists exactly its own tools and compression lives only in Compress Files", async ({ page }) => {
    for (const workspace of ToolWorkspaceRegistry.filter(({ id }) => id !== "pdf-editor")) {
        await page.goto("/");
        await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: workspace.title, exact: true }).click();
        const rail = page.getByRole("toolbar", { name: `${workspace.title} tools` });
        const entries = workspaceEntries(workspace, toolsForWorkspace(workspace));
        await expect(rail.getByRole("button")).toHaveText(entries.map(({ title }) => new RegExp(`^${title}`)));
    }

    const compress = ToolWorkspaceRegistry.find(({ id }) => id === "compress-files")!;
    expect(toolsForWorkspace(compress).map(({ title }) => title)).toEqual(["Compress", "Compress Images"]);
    for (const workspace of ToolWorkspaceRegistry.filter(({ id }) => id !== "compress-files")) {
        expect(toolsForWorkspace(workspace).map(({ id }) => id)).not.toContain("compress-files");
        expect(toolsForWorkspace(workspace).map(({ id }) => id)).not.toContain("compress");
    }
});

test("Compress Files keeps the open files across tool switches and runs each native command with its own options", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: "Compress Files", exact: true }).click();
    const rail = page.getByRole("toolbar", { name: "Compress Files tools" });
    await expect(rail.getByRole("button", { name: "Compress", exact: true })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("slider", { name: "Compression quality" })).toHaveCount(0);
    await chooseFixture(page, pdfFixturePath);

    await rail.getByRole("button", { name: "Compress Images", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Compress Images", exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await expect(page.getByText(pdfFixtureName, { exact: true })).toBeVisible();

    await rail.getByRole("button", { name: "Compress", exact: true }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await page.getByRole("button", { name: "Compress 1 file", exact: true }).click();
    await expect(page.getByText("Test output", { exact: true }).last()).toBeVisible();

    await rail.getByRole("button", { name: "Compress Images", exact: true }).click();
    await chooseFixture(page, fixturePath);
    await page.getByRole("switch", { name: "Preserve original pixels" }).click();
    await page.getByRole("button", { name: "Export Compress Images", exact: true }).click();
    await expect.poll(() => invocationCount(page, "compress_images")).toBe(1);

    const requests = await page.evaluate(() => ((window as TestWindow).__toolboxInvocations ?? [])
        .filter(({ command }) => command === "compress_files" || command === "compress_images")
        .map(({ command, args }) => ({ command, request: (args as { request: unknown }).request })));
    expect(requests).toEqual([
        { command: "compress_files", request: { paths: [pdfFixturePath], level: "medium", stripExif: false, outputLocation: "alongsideInput" } },
        { command: "compress_images", request: { paths: [fixturePath], quality: 80, lossless: true, outputLocation: "alongsideInput" } },
    ]);
});

test("Compress defaults to Medium, describes each strength and sends the paths in order with the chosen strength and Strip EXIF", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Compress");
    const strength = page.getByRole("radiogroup", { name: "Strength" });
    await expect(strength.getByRole("radio")).toHaveCount(3);
    await expect(strength.getByRole("radio", { name: /^Medium/ })).toHaveAttribute("aria-checked", "true");
    await expect(strength.getByRole("radio", { name: "Low Least saving, detail stays" })).toBeVisible();
    await expect(strength.getByRole("radio", { name: "Medium Good for sharing and email" })).toBeVisible();
    await expect(strength.getByRole("radio", { name: "High Smallest, photos look softer" })).toBeVisible();
    await expect(page.getByText("What Medium does", { exact: true })).toBeVisible();
    await expect(page.getByText("Repacked without loss. Images untouched.", { exact: true })).toBeVisible();
    await expect(page.getByText("Text, transparency and colour profiles always stay.", { exact: true })).toBeVisible();
    const exif = page.getByRole("switch", { name: "Strip EXIF" });
    await expect(exif).not.toBeChecked();
    await expect(exif).toHaveAccessibleDescription("Off: camera, date and GPS details stay in the copies.");
    await expect(page.locator(".workspace-primary-action")).toHaveAccessibleName("Compress");

    const paths = ["/local/zeta.pdf", "/local/alpha.pdf", "/local/mid.pdf"];
    await page.evaluate((paths) => { (window as TestWindow).__toolboxDialogResults = [paths]; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("3 files");
    await page.getByRole("button", { name: "Compress 3 files", exact: true }).click();
    await expect.poll(() => invocationCount(page, "compress_files")).toBe(1);

    await strength.getByRole("radio", { name: /^High/ }).click();
    await expect(page.getByText("What High does", { exact: true })).toBeVisible();
    await exif.click();
    await expect(exif).toHaveAccessibleDescription("On: removes camera, date and GPS details from JPEG, PNG and WebP. Photos stay upright. PDFs aren't changed.");
    await page.getByRole("button", { name: "Compress 3 files", exact: true }).click();
    await expect.poll(() => invocationCount(page, "compress_files")).toBe(2);

    const requests = await page.evaluate(() => ((window as TestWindow).__toolboxInvocations ?? [])
        .filter(({ command }) => command === "compress_files")
        .map(({ args }) => (args as { request: unknown }).request));
    expect(requests).toEqual([
        { paths, level: "medium", stripExif: false, outputLocation: "alongsideInput" },
        { paths, level: "high", stripExif: true, outputLocation: "alongsideInput" },
    ]);
});

for (const layout of [
    { name: "vertical", viewport: { width: 1280, height: 360 }, forward: "ArrowDown", backward: "ArrowUp", ignored: "ArrowRight" },
    { name: "narrow horizontal", viewport: { width: 900, height: 720 }, forward: "ArrowRight", backward: "ArrowLeft", ignored: "ArrowDown" },
] as const) {
    test(`a twelve-tool rail stays usable in the ${layout.name} layout with arrows, Home and End`, async ({ page }, testInfo) => {
        await page.setViewportSize(layout.viewport);
        await page.goto("/tests/ui/harness/rail.html");
        const rail = page.getByRole("toolbar", { name: "Harness tools" });
        const commands = rail.getByRole("button");
        await expect(commands).toHaveCount(12);
        await expect(rail).toHaveAttribute("aria-orientation", layout.name === "vertical" ? "vertical" : "horizontal");
        await expect(commands.nth(11)).not.toBeInViewport();

        await commands.first().focus();
        await page.keyboard.press(layout.ignored);
        await expect(commands.first()).toBeFocused();
        await page.keyboard.press(layout.forward);
        await expect(commands.nth(1)).toBeFocused();
        await page.keyboard.press(layout.backward);
        await expect(commands.first()).toBeFocused();
        await page.keyboard.press(layout.backward);
        await expect(commands.nth(11)).toBeFocused();
        await expect(commands.nth(11)).toBeInViewport();
        await page.keyboard.press("Home");
        await expect(commands.first()).toBeFocused();
        await expect(commands.first()).toBeInViewport();
        await page.keyboard.press("End");
        await expect(commands.nth(11)).toBeFocused();
        await expect(commands.nth(11)).toBeInViewport();
        await expect(commands.nth(11)).toHaveAttribute("tabindex", "0");
        await expect(commands.first()).toHaveAttribute("tabindex", "-1");

        await page.keyboard.press("Enter");
        await expect(page.getByTestId("active-tool")).toHaveText(UtilityRegistry.filter((tool) => tool.status !== "unavailable")[11].id);
        await expect(commands.nth(11)).toHaveAttribute("aria-pressed", "true");
        await page.screenshot({ path: testInfo.outputPath(`rail-harness-${layout.name.replace(" ", "-")}.png`) });
    });
}

const invocationCount = (page: Page, command: string) =>
    page.evaluate((command) => ((window as TestWindow).__toolboxInvocations ?? []).filter((invocation) => invocation.command === command).length, command);

test("window shortcuts reach search and settings from every workspace view", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });
    for (const workspace of ToolWorkspaceRegistry) {
        const utility = UtilityRegistry.find((item) => item.status !== "unavailable" && workspaceForTool(item.id)?.id === workspace.id)!;
        await page.getByRole("button", { name: `Open ${utility.title}`, exact: true }).click();
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);

        await page.locator("body").press("Meta+,");
        await expect(page.getByRole("heading", { level: 1, name: "Appearance" })).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);

        await page.locator("body").press("Meta+/");
        await expect(page.getByRole("heading", { level: 1, name: "Keyboard shortcuts" })).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(page.locator(".workspace-title")).toHaveText(workspace.title);

        await page.locator("body").press("Meta+K");
        await expect(search).toBeFocused();
        await expect(page.getByRole("heading", { name: "Tool library" })).toBeVisible();
    }
});

const runShortcutCases = [
    { tool: "Lock & Unlock", file: pdfFixturePath, command: "remove_password", ready: (page: Page) => fillPasswords(page, "test-password") },
    { tool: "Compress", file: pdfFixturePath, command: "compress_files", ready: async () => undefined },
    { tool: "Compress Images", file: fixturePath, command: "compress_images", ready: async () => undefined },
    { tool: "Image Metadata", file: fixturePath, command: "image_metadata", ready: async () => undefined },
];

for (const { tool, file, command, ready } of runShortcutCases) {
    test(`${tool} opens the picker with Cmd+O and runs with Cmd+Enter only when its run button is enabled`, async ({ page }) => {
        await page.goto("/");
        await page.getByRole("button", { name: `Open ${tool}`, exact: true }).click();
        await page.locator("body").press("Meta+Enter");
        expect(await invocationCount(page, command)).toBe(0);

        await page.evaluate((file) => { (window as TestWindow).__toolboxDialogResults = [file]; }, file);
        await page.locator("body").press("Meta+O");
        await expect(page.getByText(path.basename(file), { exact: true })).toBeVisible();
        expect(await invocationCount(page, "plugin:dialog|open")).toBe(1);

        await ready(page);
        await expect(page.locator(".workspace-primary-action")).toBeEnabled();
        await page.locator("body").press("Meta+Enter");
        await expect.poll(() => invocationCount(page, command)).toBe(1);
    });
}

test("shortcuts stay out of text fields and composition, and Run still fires from a field", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Lock & Unlock");
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Lock", exact: true }).click();
    await chooseFixture(page, pdfFixturePath);
    await fillPasswords(page, "test-password");
    const field = page.locator('.workspace-control-panel input[type="password"]').first();
    await field.focus();

    for (const key of ["Meta+O", "Meta+,", "Meta+/", "Meta+K"]) await page.keyboard.press(key);
    await expect(field).toBeFocused();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    expect(await invocationCount(page, "plugin:dialog|open")).toBe(1);

    const composingPrevented = await field.evaluate((input) => [true, false].map((isComposing) => {
        const event = new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true, isComposing });
        // Some input methods report composition only through keyCode 229.
        if (!isComposing) Object.defineProperty(event, "keyCode", { value: 229 });
        input.dispatchEvent(event);
        return event.defaultPrevented;
    }));
    expect(composingPrevented).toEqual([false, false]);
    expect(await invocationCount(page, "protect_files")).toBe(0);

    await page.keyboard.press("Meta+Enter");
    await expect.poll(() => invocationCount(page, "protect_files")).toBe(1);
});

for (const { platform, labels } of [
    { platform: "MacIntel", labels: { find: "⌘K", settings: "⌘,", help: "⌘/", open: "⌘O", run: "⌘↵", close: "⌘W", aria: "Meta+Enter", editor: ["⌘Z", "⇧⌘Z", "⌘E", "⌘D", "Delete"] } },
    { platform: "Win32", labels: { find: "Ctrl+K", settings: "Ctrl+,", help: "Ctrl+/", open: "Ctrl+O", run: "Ctrl+Enter", close: "Alt+F4", aria: "Control+Enter", editor: ["Ctrl+Z", "Ctrl+Y", "Ctrl+E", "Ctrl+D", "Delete"] } },
]) {
    test(`shortcut hints use the ${platform === "Win32" ? "Windows" : "macOS"} modifier`, async ({ page }) => {
        await page.addInitScript((platform) => {
            Object.defineProperty(window.navigator, "platform", { configurable: true, value: platform });
        }, platform);
        await page.goto("/");
        await expect(page.locator(".search-box kbd")).toHaveText(labels.find);
        const settings = page.getByRole("button", { name: "Settings", exact: true });
        await expect(settings).toContainText(labels.settings);

        await openFromHome(page, "Compress");
        await expect(page.getByRole("button", { name: "Choose files to process" })).toContainText(labels.open);
        const run = page.locator(".workspace-primary-action");
        await expect(run).toContainText(labels.run);
        await expect(run).toHaveAttribute("aria-keyshortcuts", labels.aria);

        await settings.click();
        await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Keyboard shortcuts" }).click();
        const list = page.getByRole("region", { name: "Keyboard shortcuts", exact: true });
        await expect(page.getByText("Shown for the computer you are on.")).toBeVisible();
        await expect(list.getByRole("region", { name: "Everywhere" }).locator("dd")).toHaveText([labels.find, labels.settings, labels.help, labels.open, labels.close]);
        await expect(list.getByRole("region", { name: "Batch tools" }).locator("dt")).toHaveText(["Start the run", "Cancel the run", "Select all files", "Remove selected files", "Move through the list"]);

        const editor = list.getByRole("region", { name: "PDF Editor" });
        await expect(editor.locator("dt")).toHaveText(["Undo", "Redo", "Export a copy", "Duplicate the selected mark", "Delete the selected mark"]);
        await expect(editor.locator("dd")).toHaveText(labels.editor);

        await list.getByRole("searchbox", { name: "Search shortcuts" }).fill("select all");
        await expect(list.locator("dt")).toHaveText(["Select all files"]);
        await list.getByRole("searchbox", { name: "Search shortcuts" }).fill("undo");
        await expect(list.locator("dt")).toHaveText(["Undo"]);
        await list.getByRole("searchbox", { name: "Search shortcuts" }).fill("rotate");
        await expect(list.getByText("No shortcuts match.")).toBeVisible();
    });
}

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
    await openFromHome(page, "Resize Images");
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
    await expect(sections.getByRole("button")).toHaveText(["Appearance", "Output location", "Quick actions", "Packs", "Privacy", "Updates", "About and support", "Keyboard shortcuts"]);
    await expect(page.getByText("Changes save as you make them.")).toBeVisible();

    const about = await openSettingsSection(page, "About and support", false);
    await expect(sections.getByRole("button", { name: "About and support" })).toHaveAttribute("aria-pressed", "true");
    await expect(sections.getByRole("button", { name: "Appearance" })).toHaveAttribute("aria-pressed", "false");
    await expect(about).toContainText(`Version ${appVersion} for macOS`);
    await expect(about).toContainText("Support Toolbox");
    await about.getByRole("button", { name: "Sponsor on GitHub" }).click();
    await expect.poll(() => page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "open_sponsor_page").length,
    )).toBe(1);

    await about.getByRole("button", { name: "Open notices" }).click();
    await expect.poll(() => page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "open_third_party_notices").length,
    )).toBe(1);
    await page.evaluate(() => { (window as TestWindow).__toolboxNoticesFailure = "The third-party notices are missing from this copy of Toolbox. Reinstall Toolbox to restore them."; });
    await about.getByRole("button", { name: "Open notices" }).click();
    await expect(about).toContainText("The third-party notices are missing from this copy of Toolbox.");
});

test("Find a tool on the settings page opens the tool search", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Resize Images");
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

test("view links and rail tools render their design icon masks", async ({ page }) => {
    await page.goto("/");
    const masks = (selector: string) => page.locator(selector).evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).maskImage));
    const viewMasks = await masks('nav[aria-label="Views"] .rail-link-icon');
    expect(viewMasks).toHaveLength(ToolWorkspaceRegistry.length);
    expect(viewMasks.every((mask) => mask !== "none")).toBe(true);

    const railMasks: string[] = [];
    for (const workspace of ToolWorkspaceRegistry.filter((item) => item.id !== "pdf-editor")) {
        await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: workspace.title, exact: true }).click();
        const icons = await masks(".workspace-command-icon > span");
        expect(icons).toHaveLength(workspaceEntries(workspace, toolsForWorkspace(workspace)).length);
        railMasks.push(...icons);
        await page.getByRole("button", { name: "← All tools" }).click();
    }
    expect(railMasks.every((mask) => mask !== "none")).toBe(true);
    expect(new Set(railMasks).size).toBeGreaterThan(1);
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
    await expect(page.locator(".command-center")).toBeVisible();
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

const drawRedactionBox = async (page: Page, from = { x: .25, y: .1 }, to = { x: .75, y: .3 }) => {
    const area = page.getByRole("region", { name: "Redaction area" });
    await expect(area).toBeVisible();
    await area.evaluate((element) => element.scrollIntoView({ block: "start" }));
    const box = (await area.boundingBox())!;
    await page.mouse.move(box.x + box.width * from.x, box.y + box.height * from.y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * to.x, box.y + box.height * to.y, { steps: 5 });
    await page.mouse.up();
};

const lastInvocation = (page: Page, command: string) => page.evaluate((command) =>
    ((window as TestWindow).__toolboxInvocations ?? []).filter((invocation) => invocation.command === command).at(-1), command);

const chooseFixture = async (page: Page, selectedPath: string) => {
    await page.evaluate((fixture) => {
        (window as TestWindow).__toolboxDialogResults = [fixture];
    }, selectedPath);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(path.basename(selectedPath), { exact: true }).first()).toBeVisible();
};

test("image metadata inspection shows labelled categories for each file", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Image Metadata");
    await chooseFixture(page, fixturePath);
    await page.getByRole("radio", { name: "Inspect metadata" }).click();
    await page.getByRole("button", { name: "Inspect metadata", exact: true }).click();

    expect((await lastInvocation(page, "inspect_image_metadata"))?.args).toMatchObject({ request: { paths: [fixturePath] } });
    const report = page.getByRole("region", { name: `Metadata in ${fixtureName}` });
    await expect(report.locator(".image-metadata-row").filter({ hasText: "Camera" })).toContainText("Removed");
    await expect(report.locator(".image-metadata-row").filter({ hasText: "Orientation" })).toContainText("Rotate 90° clockwise to show upright");
    await expect(report).not.toContainText('"status"');
    expect(await invocationCount(page, "image_metadata")).toBe(0);
});

test("crop is applied visually and exported as part of the scene", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Edit PDF");
    await chooseFixture(page, pdfFixturePath);
    await drawSceneMark(page, "Crop");
    await page.getByRole("button", { name: "Export PDF", exact: true }).click();
    const invocation = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.find(c => c.command === "export_pdf_scene"));
    expect(invocation?.args).toMatchObject({ request: { scene: { pages: [{ crop: { x: expect.any(Number), width: expect.any(Number) } }] } } });
});

test("pdf editor renders page previews", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Sign PDF");
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await expect(page.getByRole("img", { name: "Thumbnail of page 1" })).toBeVisible();
});

test("editor command rails keep one document session while changing tools", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("img", { name: `Preview of ${fixtureName}` })).toBeVisible();

    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Rotate", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Rotate and Flip Images", exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Adjust", exact: true }).click();
    await expect(page.getByRole("slider", { name: "Contrast" })).toBeVisible();

    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open PDF Editor", exact: true }).click();
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Crop", exact: true }).click();
    await expect(page.getByText("Drag a rectangle on the page to crop.", { exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
});

test("vision tools explain unavailable resources before file selection", async ({ page }) => {
    await page.goto("/");
    for (const [index, id] of ["image-blur-faces", "image-remove-bg"].entries()) {
        const utility = UtilityRegistry.find((item) => item.id === id);
        expect(utility).toBeDefined();
        if (!utility) continue;
        if (index > 0) await page.getByRole("button", { name: "← All tools" }).click();
        await openFromHome(page, utility.title);
        await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
        await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveCount(0);
    }
});

test("PDF editor protects the last page and supports unified history", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Remove PDF Pages");
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("button", { name: "Delete selected pages" })).toBeDisabled();
    await drawSceneMark(page, "Text");
    await expect(page.locator(".scene-object-hit")).toHaveCount(1);
    await page.getByRole("button", { name: /^Undo \(/ }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(0);
    await page.getByRole("button", { name: /^Redo \(/ }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(1);
    await page.getByRole("button", { name: "Reset…" }).click();
    await page.getByRole("alertdialog", { name: "Reset all page changes?" }).getByRole("button", { name: "Reset", exact: true }).click();
    await expect(page.locator(".scene-object-hit")).toHaveCount(0);
});

test("Image editor previews a reversible edit stack and exports one combined plan", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("img", { name: `Preview of ${fixtureName}` })).toBeVisible();
    await expect(page.getByRole("button", { name: "Undo" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Redo" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Reset edits" })).toBeDisabled();
    await expect(page.getByText("Edits are applied in order. Undo removes one edit at a time.", { exact: true })).toBeVisible();
    await expect(page.getByRole("tablist", { name: "Edit Image" })).toBeVisible();
    await expect(page.getByRole("toolbar", { name: "Image Editor tools" })).toBeVisible();
    await expect(page.locator(".workspace-primary-action")).toHaveCount(1);

    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Resize", exact: true }).click();
    await page.getByLabel("Width").fill("320");
    await expect.poll(async () =>
        (await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []))
            .some(({ command }) => command === "inspect_image_edit_preview"),
    ).toBe(true);
    await page.getByRole("button", { name: "Add edit to plan" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Rotate", exact: true }).click();
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
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "Pixel rectangle" }).click();
    await page.getByLabel("Width, pixels").fill("160");
    await page.getByLabel("Height, pixels").fill("120");
    await page.getByLabel("Left edge, pixels").fill("80");
    await page.getByLabel("Top edge, pixels").fill("60");

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
    await expect(page.getByLabel("Left edge, pixels")).toHaveValue("240");
    await expect(page.getByLabel("Top edge, pixels")).toHaveValue("180");
    await expect(overlay).toHaveAttribute("style", /left: 37\.5%/);
    await expect(overlay).toHaveAttribute("style", /top: 37\.5%/);
});

test("Image crop offers aspect presets with a rule-of-thirds grid", async ({ page }, testInfo) => {
    await page.goto("/");
    await openFromHome(page, "Resize Images");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "1:1" }).click();

    const overlay = page.getByLabel("Crop selection");
    await expect(overlay).toHaveAttribute("style", /left: 12\.5%/);
    await expect(overlay).toHaveAttribute("style", /width: 75%/);
    await expect(overlay).toHaveAttribute("style", /height: 100%/);
    await expect(page.locator(".image-editor-crop-grid > span")).toHaveCount(4);
    await expect(page.getByRole("region", { name: "Crop result preview" })).toContainText("480 × 480px");
    await page.screenshot({ path: testInfo.outputPath("image-crop-presets.png") });

    const previewInvocation = await page.evaluate(() =>
        [...(window as TestWindow).__toolboxInvocations ?? []]
            .reverse()
            .find(({ command }) => command === "inspect_image_edit_preview"),
    );
    expect(previewInvocation?.args).toMatchObject({
        request: { plan: { edits: [{ kind: "crop", mode: "aspectRatio", aspectWidth: 1, aspectHeight: 1 }] } },
    });
});

test("Image crop cancels a global pointer interruption before a later move", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "Pixel rectangle" }).click();
    await page.getByLabel("Width, pixels").fill("160");
    await page.getByLabel("Height, pixels").fill("120");
    await page.getByLabel("Left edge, pixels").fill("80");
    await page.getByLabel("Top edge, pixels").fill("60");
    await expect(page.getByLabel("Crop selection")).toBeVisible();

    const stage = (await page.locator(".image-editor-preview-stage").boundingBox())!;
    const before = { left: await page.getByLabel("Left edge, pixels").inputValue(), top: await page.getByLabel("Top edge, pixels").inputValue() };
    await page.locator(".image-editor-preview-stage").dispatchEvent("pointerdown", {
        pointerId: 77, button: 0, clientX: stage.x + stage.width * .25, clientY: stage.y + stage.height * .25,
    });
    await page.evaluate(() => window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 77, bubbles: true })));
    await page.locator(".image-editor-preview-stage").dispatchEvent("pointermove", {
        pointerId: 77, clientX: stage.x + stage.width * .75, clientY: stage.y + stage.height * .75,
    });
    await expect(page.getByLabel("Left edge, pixels")).toHaveValue(before.left);
    await expect(page.getByLabel("Top edge, pixels")).toHaveValue(before.top);
});

test("Image aspect crop uses the anchored maximum-fit geometry for overlay and result", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "1:1" }).click();
    await page.getByLabel("Keep", { exact: true }).selectOption("end");

    const overlay = page.getByLabel("Crop selection");
    await expect(overlay).toHaveAttribute("style", /left: 25%/);
    await expect(overlay).toHaveAttribute("style", /top: 0%/);
    await expect(overlay).toHaveAttribute("style", /width: 75%/);
    await expect(overlay).toHaveAttribute("style", /height: 100%/);
    await expect(page.getByRole("region", { name: "Crop result preview" })).toContainText("480 × 480px");
});

test("Image aspect crop can move while keeping its preset ratio", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "1:1" }).click();
    await expect(page.getByLabel("Crop selection")).toHaveAttribute("style", /width: 75%/);

    await page.locator(".image-editor-preview-stage").scrollIntoViewIfNeeded();
    const stage = (await page.locator(".image-editor-preview-stage").boundingBox())!;
    await page.mouse.move(stage.x + stage.width * 0.5, stage.y + stage.height * 0.5);
    await page.mouse.down();
    await page.mouse.move(stage.x + stage.width * 0.6, stage.y + stage.height * 0.5, { steps: 3 });
    await page.mouse.up();

    const overlay = page.getByLabel("Crop selection");
    await expect(page.getByRole("radio", { name: "1:1" })).toHaveAttribute("aria-checked", "true");
    await expect(page.getByLabel("Keep", { exact: true })).toHaveValue("custom");
    await expect(overlay).toHaveAttribute("style", /left: 22\.5%/);
    await expect(overlay).toHaveAttribute("style", /width: 75%/);
    await expect.poll(async () => page.evaluate(() =>
        [...(window as TestWindow).__toolboxInvocations ?? []]
            .reverse()
            .find(({ command }) => command === "inspect_image_edit_preview")?.args,
    )).toMatchObject({
        request: { plan: { edits: [{ kind: "crop", mode: "aspectRatio", x: 144, y: 0, aspectWidth: 1, aspectHeight: 1, anchor: "custom" }] } },
    });
});

test("Image crop clears an old result and disables Run for an empty rectangle", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Crop", exact: true }).click();
    await page.getByRole("radio", { name: "Pixel rectangle" }).click();
    await page.getByLabel("Width, pixels").fill("160");
    await page.getByLabel("Height, pixels").fill("120");
    await expect(page.getByRole("region", { name: "Crop result preview" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeEnabled();

    await page.getByLabel("Width, pixels").fill("0");

    await expect(page.getByRole("region", { name: "Crop result preview" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Run on 1 image" })).toBeDisabled();
});

test("Image editor history can undo, redo, and reset the composed plan", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Resize", exact: true }).click();
    await page.getByLabel("Width").fill("320");
    await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Rotate", exact: true }).click();
    await page.getByLabel("Rotation").selectOption("90");
    await page.getByRole("button", { name: "Add edit to plan" }).click();

    await expect(page.getByRole("button", { name: "Undo" })).toBeEnabled();
    await expect(page.getByText("2 committed edits", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(page.getByText("1 committed edit", { exact: true })).toBeVisible();
    await expect(page.locator(".image-edit-timeline li")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "Redo" })).toBeEnabled();

    await page.getByRole("button", { name: "Redo" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeEnabled();
    await expect(page.getByText("2 committed edits", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Reset edits" }).click();
    await expect(page.getByRole("button", { name: "Export edited images" })).toBeDisabled();
    await expect(page.getByText("0 committed edits", { exact: true })).toBeVisible();
});

test("PDF to Images identifies source selection and summarizes its non-previewable output", async ({ page }) => {
  await page.goto("/");
  await openFromHome(page, "PDF to Images");
  await chooseFixture(page, pdfFixturePath);
  const pageCard = page.locator(".conversion-page-card");
  await expect(pageCard).toHaveAttribute("data-slot", "card");
  await expect(pageCard.locator('[data-slot="card-content"]')).toHaveCount(1);
  await expect(page.getByRole("region", { name: "Source page selection" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Conversion preview" })).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
  await expect(page.getByRole("radiogroup", { name: "Resolution" })).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Format" })).toBeVisible();
  await expect(page.getByText("Pick pages here to replace the range for this PDF only.", { exact: true })).toBeVisible();
  await expect(page.getByText("Output preview unavailable. Export renders each PDF's pages at 150 dpi as PNG image files.", { exact: true })).toBeVisible();

  await page.getByRole("radiogroup", { name: "Resolution" }).getByRole("radio", { name: "300 dpi" }).click();
  await page.getByRole("radiogroup", { name: "Format" }).getByRole("radio", { name: "WebP" }).click();
  await expect(page.getByText("Output preview unavailable. Export renders each PDF's pages at 300 dpi as WebP image files.", { exact: true })).toBeVisible();

  await page.getByRole("checkbox", { name: "Page 1" }).click();
  await expect(page.getByText("No pages picked. Pick at least one page, or use the range instead.", { exact: true })).toBeVisible();
  await expect(page.locator(".workspace-primary-action")).toBeDisabled();
});

test("PDF range splitting rejects a blank range and accepts a range", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Split PDF");
    await chooseFixture(page, pdfFixturePath);

    const splitBy = page.getByRole("radiogroup", { name: "Split by" });
    await expect(splitBy.getByRole("radio", { name: "Page ranges" })).toBeChecked();
    const ranges = page.getByRole("textbox", { name: "Page ranges" });
    const exportSplit = page.getByRole("button", { name: "Export Split PDF" });
    await expect(page.getByRole("spinbutton", { name: "Pages per file" })).toHaveCount(0);
    await expect(page.getByText("Each range becomes one file. 8- runs to the last page.", { exact: true })).toBeVisible();

    await expect(page.getByRole("alert")).toHaveText("Enter at least one page range to split.");
    await expect(exportSplit).toBeDisabled();

    for (const [typed, message] of [
        ["3-1", "3-1 runs backwards (write 1-3)."],
        ["1,,3", "Entry 2 is empty."],
        ["1-3,", "Entry 2 is empty."],
        ["x", "x isn't a page."],
        ["0", "0 isn't a page. Pages start at 1."],
        ["0-2", "0-2 isn't a range. Pages start at 1."],
        ["1-2-3", "1-2-3 isn't a range."],
        ["-5", "-5 isn't a range."],
        ["3-1, 5, , x", "3-1 runs backwards (write 1-3). Entry 3 is empty. x isn't a page."],
        ["   ", "Enter at least one page range to split."],
    ] as const) {
        await ranges.fill(typed);
        await expect(page.getByRole("alert"), typed).toHaveText(message);
        await expect(ranges, typed).toHaveAttribute("aria-invalid", "true");
        await expect(exportSplit, typed).toBeDisabled();
    }

    const names = page.getByRole("group", { name: "File names" });
    await expect(names.getByText("document-pages-1-3.pdf", { exact: true })).toBeVisible();
    await expect(names.getByText("document-page-5.pdf", { exact: true })).toBeVisible();

    await ranges.fill("1-3, 5, 8-");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(ranges).not.toHaveAttribute("aria-invalid", "true");
    await expect(exportSplit).toBeEnabled();
    await exportSplit.click();
    await expect.poll(async () => ((await lastInvocation(page, "split_pdf"))?.args as { request?: unknown } | undefined)?.request).toEqual({
        paths: [pdfFixturePath],
        mode: { kind: "ranges", ranges: "1-3, 5, 8-" },
        outputLocation: "alongsideInput",
    });
});

test("PDF splitting every N pages sends only the page count", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Split PDF");
    await chooseFixture(page, pdfFixturePath);

    await page.getByRole("radiogroup", { name: "Split by" }).getByRole("radio", { name: "Every N pages" }).click();
    const pagesPerFile = page.getByRole("spinbutton", { name: "Pages per file" });
    const exportSplit = page.getByRole("button", { name: "Export Split PDF" });
    await expect(page.getByRole("textbox", { name: "Page ranges" })).toHaveCount(0);
    await expect(page.getByRole("group", { name: "File names" })).toHaveCount(0);
    await expect(pagesPerFile).toHaveValue("1");
    await expect(page.getByText("1 makes one file per page. The last file can be shorter.", { exact: true })).toBeVisible();
    await expect(exportSplit).toBeEnabled();

    for (const typed of ["0", ""]) {
        await pagesPerFile.fill(typed);
        await expect(page.getByRole("alert"), typed).toHaveText("Enter 1 or more pages per file.");
        await expect(exportSplit, typed).toBeDisabled();
    }

    await pagesPerFile.fill("4");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await exportSplit.click();
    await expect.poll(async () => ((await lastInvocation(page, "split_pdf"))?.args as { request?: unknown } | undefined)?.request).toEqual({
        paths: [pdfFixturePath],
        mode: { kind: "everyN", pages: 4 },
        outputLocation: "alongsideInput",
    });
});

test("media and security workspaces expose ordered inputs and format boundaries", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Create GIF");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("region", { name: "Frame order" })).toBeVisible();
    await expect(page.locator(".media-frame-preview")).toBeVisible();
    await expect.poll(async () => page.evaluate(() => (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_image_preview").length ?? 0)).toBe(1);
    await expect(page.getByRole("button", { name: `Move ${fixtureName} up` })).toBeDisabled();
    await expect(page.getByRole("button", { name: `Move ${fixtureName} down` })).toBeDisabled();

    await page.getByRole("button", { name: "← All tools" }).click();
    await openFromHome(page, "Lock & Unlock");
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();
});

test("TIFF Combine expands internal pages and sends the displayed order", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Split and Combine TIFF");
    await page.getByRole("radiogroup", { name: "TIFF Pages mode" }).getByRole("radio", { name: "Combine into one TIFF" }).click();
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
    await confirmSave(page);
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
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, pdfFixturePath);
    await expect(page.getByRole("region", { name: "Source page selection" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Conversion preview" })).toHaveCount(0);
    await expect(page.getByText("Output preview unavailable. Export renders each PDF's pages at 150 dpi as PNG image files.", { exact: true })).toBeVisible();
    await expect(page.getByRole("img", { name: "Source page preview 1" })).toBeVisible();

    const conversionRail = page.getByRole("toolbar", { name: "PDF Conversion tools" });
    await conversionRail.getByRole("button", { name: "Images to PDF", exact: true }).click();
    await expect(page.getByText("Output preview unavailable. Export combines the selected images into one PDF in file order.", { exact: true })).toBeVisible();

    await page.goto("/");
    await openFromHome(page, "Merge PDF");
    await chooseFixture(page, pdfFixturePath);
    const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
    for (const [action, summary] of [
        ["Merge PDF", "Output preview unavailable. Export merges the selected PDFs in file order."],
        ["Extract PDF Pages", "Output preview unavailable. Export creates a PDF containing the selected pages."],
        ["Split PDF", "Output preview unavailable. Export creates one PDF for each range, or for every N pages."],
    ] as const) {
        await rail.getByRole("button", { name: action, exact: true }).click();
        await expect(page.getByText(summary, { exact: true })).toBeVisible();
    }
});


test("PDF conversion visibly rejects retained files when an action changes format", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Images to PDF");
    await chooseFixture(page, fixturePath);

    const rail = page.getByRole("toolbar", { name: "PDF Conversion tools" });
    await rail.getByRole("button", { name: "Extract from PDF", exact: true }).click();
    await expect(page.getByText("1 file won't run with Extract from PDF. It stays in the list for other actions.", { exact: true })).toBeVisible();
    await expect(page.getByRole("option", { name: fixtureName })).toContainText("Won't run");
    await expect(page.getByRole("button", { name: "Extract", exact: true })).toBeDisabled();
    const inspections = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_pdf") ?? [],
    );
    expect(inspections).toHaveLength(0);
});

test("PDF page-scoped conversions send the selected pages and reach OCR from the rail", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, "/local/document.scoped.pdf");
    const pageOne = page.getByRole("checkbox", { name: "Page 1" });
    const pageThree = page.getByRole("checkbox", { name: "Page 3" });
    await expect(pageThree).toBeVisible();
    await page.getByRole("img", { name: "Source page preview 1" }).click();
    await expect(pageOne).not.toBeChecked();
    await pageThree.focus();
    await page.keyboard.press("Space");
    await expect(pageThree).not.toBeChecked();
    await page.getByRole("button", { name: "Export PDF to Images", exact: true }).click();

    const imagesInvocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "pdf_to_images"),
    );
    expect(imagesInvocation?.args).toMatchObject({
        request: { paths: ["/local/document.scoped.pdf"], pageRange: null, picks: [{ path: "/local/document.scoped.pdf", pages: [1] }] },
    });

    const ocrButton = page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "OCR PDF", exact: true });
    await expect(ocrButton).toBeEnabled();
    await expect(ocrButton).not.toHaveAttribute("title", /unavailable/);
    await ocrButton.click();
    await expect(page.getByRole("button", { name: "Export OCR PDF", exact: true })).toBeVisible();
    await expect(page.getByText("Unavailable in this build.", { exact: true })).toHaveCount(0);
    await expect(page.getByText("Output preview unavailable. Export recognizes the text on each page and saves it as a text file.", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Export OCR PDF", exact: true }).click();
    expect((await lastInvocation(page, "ocr_pdf"))?.args).toMatchObject({ request: { paths: ["/local/document.scoped.pdf"], pageRange: null, mode: "text" } });
    await page.getByRole("img", { name: "Source page preview 1" }).click();
    await expect(pageOne).not.toBeChecked();
    await page.getByRole("button", { name: "Export OCR PDF", exact: true }).click();
    expect((await lastInvocation(page, "ocr_pdf"))?.args).toMatchObject({ request: { paths: ["/local/document.scoped.pdf"], pageRange: "2-3", mode: "text" } });
});

test("OCR sends scans as a text-file request to the folder chosen in Settings", async ({ page }) => {
    await storeOutputSettings(page, { mode: "customFolder", folder: "/fake/shell/output" });
    await page.goto("/");
    await openFromHome(page, "Extract from PDF");
    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "OCR PDF", exact: true }).click();
    await page.evaluate((fixtures) => { (window as TestWindow).__toolboxDialogResults = [fixtures]; }, [pdfFixturePath, "/local/second-scan.pdf"]);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText("second-scan.pdf", { exact: true }).first()).toBeVisible();
    await page.getByRole("button", { name: "Export OCR PDF", exact: true }).click();

    const invocation = await lastInvocation(page, "ocr_pdf");
    expect((invocation?.args as { request?: unknown } | undefined)?.request).toEqual({
        paths: [pdfFixturePath, "/local/second-scan.pdf"],
        pageRange: null,
        mode: "text",
        force: false,
        languages: ["eng"],
        outputLocation: { customFolder: "/fake/shell/output" },
    });
});

test("OCR shows a missing engine, a locked scan and a blank scan beside a saved text file", async ({ page }) => {
    const textFile = "/fake/shell/output/document-ocr.txt";
    const locked = "It's password-protected. Remove its password in File Security, then run OCR again.";
    await page.goto("/");
    await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
        succeeded(pdfFixturePath, [textFile], { detail: "1 page recognized. English." }),
        failed("/local/no-engine.pdf", "unavailable", "The text recognition engine is missing. Reinstall Toolbox."),
        failed("/local/locked.pdf", "invalidInput", locked, { failure: { kind: "invalidInput", message: locked, pointer: { kind: "tool", toolId: "lock-unlock" } } }),
        failed("/local/blank.pdf", "processing", "OCR completed but found no readable text."),
    ]);
    await openFromHome(page, "Extract from PDF");
    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "OCR PDF", exact: true }).click();
    await chooseFixture(page, pdfFixturePath);
    await page.getByRole("button", { name: "Export OCR PDF", exact: true }).click();

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows).toHaveCount(4);
    await expect(rows.nth(0)).toContainText("document-ocr.txt");
    await expect(rows.nth(0)).toContainText("1 page recognized. English.");
    await rows.nth(0).getByRole("button", { name: "Open document-ocr.txt", exact: true }).click();
    await rows.nth(0).getByRole("button", { name: "Show in Finder: document-ocr.txt", exact: true }).click();
    expect(await fileActionInvocations(page)).toEqual([
        { command: "open_output_path", args: { path: textFile } },
        { command: "reveal_output_path", args: { path: textFile } },
    ]);
    await expect(rows.nth(1)).toContainText("Unavailable");
    await expect(rows.nth(1)).toContainText("The text recognition engine is missing. Reinstall Toolbox.");
    await expect(rows.nth(2)).toContainText(locked);
    await expect(rows.nth(3)).toContainText("OCR completed but found no readable text.");
    for (const index of [1, 2, 3]) await expect(rows.nth(index).getByRole("button", { name: /^Show in Finder|^Open .*\.txt$/ })).toHaveCount(0);

    await rows.nth(2).getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
});

test.describe("Extract from PDF", () => {
    const pdfs = ["/local/report.pdf", "/local/site-survey-scan.pdf"];
    const extractRequest = async (page: Page) => {
        const runs = await page.evaluate(() =>
            ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "extract_from_pdf"),
        );
        expect(runs).toHaveLength(1);
        return (runs[0].args as { request: unknown }).request;
    };
    const openWithPdfs = async (page: Page, paths = pdfs) => {
        await page.goto("/");
        await openFromHome(page, "Extract from PDF");
        await page.evaluate((paths) => { (window as TestWindow).__toolboxDialogResults = [paths]; }, paths);
        await page.getByRole("button", { name: "Choose files to process" }).click();
        await expect(page.getByText(paths[paths.length - 1].split("/").pop()!, { exact: true }).first()).toBeVisible();
    };
    const extractModes = (page: Page) => page.getByRole("radiogroup", { name: "Extract", exact: true });
    const textOptions = (page: Page) => page.getByRole("checkbox", { name: "Preserve layout" });
    const imageOptions = (page: Page) => page.getByRole("checkbox", { name: "Skip small images" });

    test("Both is the default and sends one request for every PDF with both option groups", async ({ page }) => {
        await openWithPdfs(page);
        await expect(extractModes(page).getByRole("radio", { name: "Both", exact: true })).toHaveAttribute("aria-checked", "true");
        await expect(page.getByText("A text file and a folder of images for each PDF.", { exact: true })).toBeVisible();
        await expect(textOptions(page)).toBeVisible();
        await expect(imageOptions(page)).toBeVisible();
        await expect(page.getByRole("textbox", { name: "Page range for every PDF" })).toHaveCount(0);
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        const request = await extractRequest(page);
        expect(request).toEqual({
            paths: pdfs,
            mode: "both",
            pageRange: null,
            outputLocation: "alongsideInput",
            preserveLayout: false,
        });
        expect(request).not.toHaveProperty("minImageSize");
    });

    test("Text shows only the text options and sends the layout choice and page range", async ({ page }) => {
        await openWithPdfs(page);
        await extractModes(page).getByRole("radio", { name: "Text", exact: true }).click();
        await expect(page.getByText("One text file for each PDF, in reading order.", { exact: true })).toBeVisible();
        await expect(textOptions(page)).toBeVisible();
        await expect(imageOptions(page)).toHaveCount(0);
        await expect(page.getByRole("spinbutton", { name: "Smaller than" })).toHaveCount(0);
        await textOptions(page).click();
        await page.getByRole("radio", { name: "Range", exact: true }).click();
        const range = page.getByRole("textbox", { name: "Page range for every PDF" });
        const run = page.getByRole("button", { name: "Extract from 2 PDFs", exact: true });
        await range.fill("1-3, 9-8");
        await expect(page.getByRole("alert")).toHaveText("9-8 runs backwards. Write it as 8-9.");
        await expect(range).toHaveAttribute("aria-invalid", "true");
        await expect(run).toBeDisabled();
        await range.fill("1-3, 5, 8-");
        await expect(page.getByRole("alert")).toHaveCount(0);
        await run.click();

        const request = await extractRequest(page);
        expect(request).toEqual({
            paths: pdfs,
            mode: "text",
            pageRange: "1-3, 5, 8-",
            outputLocation: "alongsideInput",
            preserveLayout: true,
        });
        expect(request).not.toHaveProperty("minImageSize");
    });

    test("Images shows only the image options and sends the minimum image size", async ({ page }) => {
        await openWithPdfs(page);
        await textOptions(page).click();
        await extractModes(page).getByRole("radio", { name: "Images", exact: true }).click();
        await expect(page.getByText("One folder of images for each PDF.", { exact: true })).toBeVisible();
        await expect(textOptions(page)).toHaveCount(0);
        await expect(imageOptions(page)).toBeVisible();
        await expect(page.getByRole("spinbutton", { name: "Smaller than" })).toHaveCount(0);
        await expect(page.getByText("Off saves every image at its original size.", { exact: true })).toBeVisible();
        await imageOptions(page).click();
        await expect(page.getByText("Leaves out bullets, lines and spacers. The result counts what was left out.", { exact: true })).toBeVisible();
        const minSize = page.getByRole("spinbutton", { name: "Smaller than" });
        await expect(minSize).toHaveValue("64");
        await minSize.fill("120");
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        expect(await extractRequest(page)).toEqual({
            paths: pdfs,
            mode: "images",
            pageRange: null,
            outputLocation: "alongsideInput",
            preserveLayout: false,
            minImageSize: 120,
        });
    });

    test("a Both run shows one row per PDF with the text file and images folder together and the missing part named", async ({ page }) => {
        await openWithPdfs(page);
        await page.evaluate(() => {
            (window as TestWindow).__toolboxProcessingResults = [
                {
                    inputPath: "/local/report.pdf",
                    outputPaths: ["/local/report-text.txt", "/local/report-images"],
                    detail: "Saved the text and 2 images.",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: [],
                },
                {
                    inputPath: "/local/site-survey-scan.pdf",
                    outputPaths: ["/local/site-survey-scan-images"],
                    detail: "No selectable text, so no text file was made. OCR can read scanned pages.",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: [],
                },
            ];
        });
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        const rows = page.locator(".result-row");
        await expect(rows).toHaveCount(2);
        await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(0)).toContainText("2 files, report-text.txt to report-images");
        await expect(rows.nth(0)).toContainText("Saved the text and 2 images.");
        await expect(rows.nth(0).getByRole("button", { name: "Open report-text.txt", exact: true })).toBeVisible();
        await expect(rows.nth(1)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(1)).toContainText("site-survey-scan-images");
        await expect(rows.nth(1)).toContainText("No selectable text, so no text file was made. OCR can read scanned pages.");
        await rows.nth(1).getByRole("button", { name: "Show in Finder: site-survey-scan-images", exact: true }).click();
        expect(await fileActionInvocations(page)).toEqual([{ command: "reveal_output_path", args: { path: "/local/site-survey-scan-images" } }]);
        expect(((await extractRequest(page)) as { mode: string }).mode).toBe("both");
    });

    test("Both sends the layout choice with the image options", async ({ page }) => {
        await openWithPdfs(page);
        await textOptions(page).click();
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        const request = await extractRequest(page);
        expect(request).toMatchObject({ mode: "both", preserveLayout: true });
        expect(request).not.toHaveProperty("minImageSize");
    });

    test("Both sends the minimum image size only while Skip small images is on", async ({ page }) => {
        await openWithPdfs(page);
        await imageOptions(page).click();
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();
        expect(await extractRequest(page)).toMatchObject({ mode: "both", minImageSize: 64 });
    });

    test("turning Skip small images off again leaves the minimum image size out of the request", async ({ page }) => {
        await openWithPdfs(page);
        await imageOptions(page).click();
        await page.getByRole("spinbutton", { name: "Smaller than" }).fill("200");
        await imageOptions(page).click();
        await expect(page.getByRole("spinbutton", { name: "Smaller than" })).toHaveCount(0);
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();
        expect(await extractRequest(page)).not.toHaveProperty("minImageSize");
    });

    test("the images folder row shows the extracted, duplicate, small and undecodable counts", async ({ page }) => {
        const damaged = "Page 7, image 2 — The image data is damaged. Not saved.";
        await openWithPdfs(page, ["/local/report.pdf", "/local/scan-archive-2019.pdf"]);
        await extractModes(page).getByRole("radio", { name: "Images", exact: true }).click();
        await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
            succeeded("/local/report.pdf", ["/local/report-images"], { detail: "Saved 17 images · 4 duplicates · 6 small left out." }),
            succeeded("/local/scan-archive-2019.pdf", ["/local/scan-archive-2019-images"], {
                detail: "Saved 45 images · 2 duplicates · 3 small left out · 1 couldn't be saved.",
                warnings: [{ code: "imageDamaged", message: damaged, details: { page: 7, image: 2 } }],
            }),
        ]);
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        const rows = page.locator(".result-row");
        const details = (row: number) => rows.nth(row).locator(".result-row-detail");
        await expect(rows).toHaveCount(2);
        await expect(details(0)).toHaveText(["report-images", "Saved 17 images · 4 duplicates · 6 small left out."]);
        await expect(rows.nth(0).getByRole("button", { name: "Show in Finder: report-images", exact: true })).toBeVisible();
        await expect(rows.nth(1)).toHaveAttribute("data-tone", "warn");
        await expect(details(1)).toHaveText([damaged, "Saved 45 images · 2 duplicates · 3 small left out · 1 couldn't be saved."]);
    });

    test("a locked PDF and a missing text helper fail only their own rows, and the lock row points to Lock & Unlock", async ({ page }) => {
        const locked = "This PDF is password-protected. Remove the password, then extract again.";
        const missing = "Not read: the text helper is missing. Reinstall Toolbox to restore it.";
        await openWithPdfs(page, ["/local/report.pdf", "/local/payroll-locked.pdf", "/local/site-survey-scan.pdf"]);
        await extractModes(page).getByRole("radio", { name: "Text", exact: true }).click();
        await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
            succeeded("/local/report.pdf", ["/local/report-text.txt"], { detail: "PDF text extracted in reading order" }),
            failed("/local/payroll-locked.pdf", "invalidInput", locked, {
                failure: { kind: "invalidInput", message: locked, pointer: { kind: "tool", toolId: "lock-unlock" } },
            }),
            failed("/local/site-survey-scan.pdf", "unavailable", missing),
        ]);
        await page.getByRole("button", { name: "Extract from 3 PDFs", exact: true }).click();

        const rows = page.locator(".result-row");
        await expect(rows).toHaveCount(3);
        await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(0).getByRole("button", { name: "Open report-text.txt", exact: true })).toBeVisible();
        await expect(rows.nth(1)).toHaveAttribute("data-state", "failed");
        await expect(rows.nth(1)).toContainText(locked);
        await expect(rows.nth(2)).toHaveAttribute("data-state", "failed");
        await expect(rows.nth(2).locator(".result-row-state")).toContainText("Unavailable");
        await expect(rows.nth(2)).toContainText(missing);
        await expect(rows.nth(2).getByRole("button", { name: "Open Lock & Unlock", exact: true })).toHaveCount(0);

        await rows.nth(1).getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
        await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
    });

    test("a Text run hands a scan that needs OCR to OCR with only that file queued", async ({ page }) => {
        await openWithPdfs(page);
        await page.evaluate((pdfs) => {
            (window as TestWindow).__toolboxProcessingResults = [
                {
                    inputPath: pdfs[0],
                    outputPaths: ["/local/report-text.txt"],
                    detail: "PDF text extracted in page order",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: [],
                },
                {
                    inputPath: pdfs[1],
                    outputPaths: [],
                    detail: "",
                    state: "skipped",
                    failure: null,
                    reason: { code: "needsOcr", message: "This PDF has no text layer. Run OCR to read its text.", pointer: { kind: "tool", toolId: "pdf-ocr" } },
                    warnings: [],
                },
            ];
        }, pdfs);
        await extractModes(page).getByRole("radio", { name: "Text", exact: true }).click();
        await page.getByRole("button", { name: "Extract from 2 PDFs", exact: true }).click();

        const rows = page.locator(".result-row");
        await expect(rows).toHaveCount(2);
        await expect(rows.nth(1)).toHaveAttribute("data-state", "skipped");
        await expect(rows.nth(1)).toContainText("This PDF has no text layer. Run OCR to read its text.");
        await expect(rows.nth(0).getByRole("button", { name: "Open OCR PDF", exact: true })).toHaveCount(0);
        await rows.nth(1).getByRole("button", { name: "Open OCR PDF", exact: true }).click();

        await expect(page.getByRole("button", { name: "Export OCR PDF", exact: true })).toBeVisible();
        await expect(page.getByText("site-survey-scan.pdf", { exact: true }).first()).toBeVisible();
        await expect(page.getByText("report.pdf", { exact: true })).toHaveCount(0);
        await page.getByRole("button", { name: "Export OCR PDF", exact: true }).click();
        expect((await lastInvocation(page, "ocr_pdf"))?.args).toMatchObject({ request: { paths: [pdfs[1]], mode: "text" } });
    });

    test("images that weren't saved are listed with their page, colour warnings stay on saved rows, and a PDF with no decodable image fails", async ({ page }) => {
        const unsupported = "Page 7, image 2 — JBIG2 isn't supported in this version. Not saved.";
        const damaged = "Page 19, image 1 — The image data is damaged. Not saved.";
        const tooLarge = "Page 41, image 1 — Would be 61,000 × 48,000 px when decoded, over the size limit. Not saved.";
        const inverted = "Page 22 — 1 inverted JPEG, copied as it is. Some viewers show it in negative.";
        const cmyk = "Pages 2, 5, 9 — 6 JPEGs use CMYK colour and were copied as they are. Some viewers show them with the wrong colours.";
        const noneDecoded = "None of the 12 images could be decoded, so nothing was saved.";
        const faxImage = "Page 3, image 1 — JBIG2 isn't supported in this version. Not saved.";
        await openWithPdfs(page, ["/local/scan-archive-2019.pdf", "/local/brochure-spring.pdf", "/local/fax-bundle.pdf"]);
        await extractModes(page).getByRole("radio", { name: "Images", exact: true }).click();
        await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
            succeeded("/local/scan-archive-2019.pdf", ["/local/scan-archive-2019-images"], {
                detail: "Saved 44 images · 3 couldn't be saved.",
                warnings: [
                    { code: "imageUnsupported", message: unsupported, details: { page: 7, image: 2, feature: "JBIG2" } },
                    { code: "imageDamaged", message: damaged, details: { page: 19, image: 1 } },
                    { code: "imageTooLarge", message: tooLarge, details: { page: 41, image: 1, width: 61000, height: 48000 } },
                    { code: "jpegColourKept", message: inverted, details: { colour: "inverted", pages: [22], images: 1 } },
                ],
            }),
            succeeded("/local/brochure-spring.pdf", ["/local/brochure-spring-images"], {
                detail: "Saved 38 images.",
                warnings: [{ code: "jpegColourKept", message: cmyk, details: { colour: "cmyk", pages: [2, 5, 9], images: 6 } }],
            }),
            failed("/local/fax-bundle.pdf", "processing", noneDecoded, {
                warnings: [{ code: "imageUnsupported", message: faxImage, details: { page: 3, image: 1, feature: "JBIG2" } }],
            }),
        ]);
        await page.getByRole("button", { name: "Extract from 3 PDFs", exact: true }).click();

        const rows = page.locator(".result-row");
        await expect(rows).toHaveCount(3);
        const details = (row: number) => rows.nth(row).locator(".result-row-detail");
        await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(0)).toHaveAttribute("data-tone", "warn");
        await expect(details(0)).toHaveText([unsupported, damaged, tooLarge, inverted, "Saved 44 images · 3 couldn't be saved."]);
        await expect(rows.nth(0).getByRole("button", { name: "Show in Finder: scan-archive-2019-images", exact: true })).toBeVisible();
        await expect(rows.nth(1)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(1)).toHaveAttribute("data-tone", "warn");
        await expect(details(1)).toHaveText([cmyk, "Saved 38 images."]);
        await expect(rows.nth(2)).toHaveAttribute("data-state", "failed");
        await expect(rows.nth(2)).toHaveAttribute("data-tone", "fail");
        await expect(details(2)).toHaveText([noneDecoded, faxImage]);
        await expect(rows.nth(2).getByRole("button", { name: "Open Lock & Unlock", exact: true })).toHaveCount(0);
    });

    test("one range for a batch skips short and scanned PDFs and notes pages without text", async ({ page }) => {
        // Extract from PDF sends one range for every PDF; the native side clamps it per file and skips what has nothing to extract.
        const scanned = "This PDF has no text layer. Run OCR to read its text.";
        const outOfRange = "No pages in 3-5, 8- are in this PDF, which has 2 pages.";
        const blankPages = "Pages 30-33 have no selectable text. OCR can read them.";
        const timedOut = "Reading text took longer than 3 minutes, so it was stopped. The other PDFs weren't affected.";
        const extractPaths = ["/local/scan-archive-2019.pdf", "/local/site-survey-scan.pdf", "/local/memo-oct.pdf", "/local/annual-accounts.pdf"];
        const skipped = (inputPath: string, code: "needsOcr" | "rangeEmptyAfterClamping", message: string, pointer: OutcomePointer | null): MockOutcome => ({
            inputPath, outputPaths: [], detail: "", state: "skipped", failure: null, reason: { code, message, pointer }, warnings: [],
        });
        await page.goto("/");
        await openFromHome(page, "Extract from PDF");
        await page.evaluate((paths) => { (window as TestWindow).__toolboxDialogResults = [paths]; }, extractPaths);
        await page.getByRole("button", { name: "Choose files to process" }).click();
        await page.getByRole("radiogroup", { name: "Extract", exact: true }).getByRole("radio", { name: "Text", exact: true }).click();
        await page.getByRole("radio", { name: "Range", exact: true }).click();
        await page.getByRole("textbox", { name: "Page range for every PDF" }).fill("3-5, 8-");
        await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
            succeeded("/local/scan-archive-2019.pdf", ["/local/scan-archive-2019-text.txt"], {
                detail: "PDF text extracted in reading order",
                warnings: [{ code: "pagesWithoutText", message: blankPages, details: { pages: [30, 31, 32, 33] } }],
            }),
            skipped("/local/site-survey-scan.pdf", "needsOcr", scanned, { kind: "tool", toolId: "pdf-ocr" }),
            skipped("/local/memo-oct.pdf", "rangeEmptyAfterClamping", outOfRange, null),
            failed("/local/annual-accounts.pdf", "processing", timedOut),
        ]);
        await page.getByRole("button", { name: "Extract from 4 PDFs", exact: true }).click();

        const extractInvocation = await page.evaluate(() =>
            (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "extract_from_pdf"),
        );
        expect(extractInvocation?.args).toMatchObject({ request: { paths: extractPaths, mode: "text", pageRange: "3-5, 8-" } });

        const rows = page.locator(".result-row");
        const details = (row: number) => rows.nth(row).locator(".result-row-detail");
        const ocrPointer = (row: number) => rows.nth(row).getByRole("button", { name: "Open OCR PDF", exact: true });
        await expect(rows).toHaveCount(4);
        await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
        await expect(rows.nth(0)).toHaveAttribute("data-tone", "note");
        await expect(details(0)).toHaveText([blankPages, "PDF text extracted in reading order"]);
        await expect(rows.nth(0).getByRole("button", { name: "Open scan-archive-2019-text.txt", exact: true })).toBeVisible();
        await expect(ocrPointer(0)).toBeVisible();
        for (const [row, reason] of [[1, scanned], [2, outOfRange]] as const) {
            await expect(rows.nth(row)).toHaveAttribute("data-state", "skipped");
            await expect(rows.nth(row)).toHaveAttribute("data-tone", "skip");
            await expect(rows.nth(row).locator(".result-row-state")).toContainText("Skipped");
            await expect(details(row)).toHaveText([reason]);
            await expect(rows.nth(row).getByRole("button", { name: /^(Show in|Retry)/ })).toHaveCount(0);
        }
        await expect(ocrPointer(1)).toBeVisible();
        await expect(ocrPointer(2)).toHaveCount(0);
        await expect(rows.nth(3)).toHaveAttribute("data-state", "failed");
        await expect(rows.nth(3)).toHaveAttribute("data-tone", "fail");
        await expect(rows.nth(3).locator(".result-row-state")).toContainText("Failed");
        await expect(details(3)).toHaveText([timedOut]);
        await expect(ocrPointer(3)).toHaveCount(0);
        await expect(page.getByText("1 extracted, 2 skipped, 1 failed", { exact: false }).first()).toBeVisible();

        await ocrPointer(1).click();
        const gatedOcr = page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "OCR PDF", exact: true });
        await expect(gatedOcr).toHaveAttribute("aria-pressed", "true");
    });
});

test("rotate PDF sends the chosen direction and zero-based pages", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Rotate PDFs");
    await chooseFixture(page, "/local/document.scoped.pdf");
    await page.getByLabel("Rotation direction").selectOption("270");
    await page.locator(".workspace-primary-action").click();
    expect((await lastInvocation(page, "rotate_pdf"))?.args).toMatchObject({ request: { paths: ["/local/document.scoped.pdf"], degrees: 270, pages: null } });

    await page.getByRole("checkbox", { name: "Page 2" }).click();
    await page.locator(".workspace-primary-action").click();
    expect((await lastInvocation(page, "rotate_pdf"))?.args).toMatchObject({ request: { degrees: 270, pages: [0, 2] } });
});

test("compare PDF needs exactly two files and can swap the original", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Compare PDFs");
    await chooseFixture(page, "/local/original.pdf");
    const exportCompare = page.locator(".workspace-primary-action");
    await expect(page.getByText("Choose exactly two PDFs: the original and the new version.", { exact: true })).toBeVisible();
    await expect(exportCompare).toBeDisabled();

    await chooseFixture(page, "/local/revised.pdf");
    await expect(page.getByText("Original: original.pdf", { exact: true })).toBeVisible();
    await expect(page.getByText("New version: revised.pdf", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Move original.pdf down" }).click();
    await expect(page.getByText("Original: revised.pdf", { exact: true })).toBeVisible();
    await exportCompare.click();
    await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
    expect((await lastInvocation(page, "compare_pdfs"))?.args).toMatchObject({ request: { paths: ["/local/revised.pdf", "/local/original.pdf"] } });
});

test("redact PDF waits for a box and sends it in PDF points", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Redact PDF");
    await chooseFixture(page, pdfFixturePath);
    const exportRedact = page.locator(".workspace-primary-action");
    await expect(exportRedact).toBeDisabled();
    await drawRedactionBox(page);
    await expect(page.getByLabel("Redaction box")).toBeVisible();
    await exportRedact.click();
    const request = ((await lastInvocation(page, "redact_pdf"))?.args as { request: { rectangle: { x: number; y: number; width: number; height: number }; scope: unknown } }).request;
    expect(request.scope).toBe("all");
    expect(request.rectangle.x).toBeCloseTo(153, -1);
    expect(request.rectangle.width).toBeCloseTo(306, -1);
    expect(request.rectangle.y).toBeCloseTo(554, -1);
    expect(request.rectangle.height).toBeCloseTo(158, -1);
});

test("redaction boxes drawn on rotated pages map back to unrotated PDF points", () => {
    const page = { index: 0, width: 600, height: 800, pageBox: [0, 0, 600, 800] as [number, number, number, number] };
    const topLeftCorner = { left: 0, top: 0, right: .5, bottom: .25 };
    expect(displayedBoxToPdfRect(topLeftCorner, { ...page, rotation: 0 })).toEqual({ x: 0, y: 600, width: 300, height: 200 });
    expect(displayedBoxToPdfRect(topLeftCorner, { ...page, rotation: 90 })).toEqual({ x: 0, y: 0, width: 150, height: 400 });
    expect(displayedBoxToPdfRect(topLeftCorner, { ...page, rotation: 180 })).toEqual({ x: 300, y: 0, width: 300, height: 200 });
    expect(displayedBoxToPdfRect(topLeftCorner, { ...page, rotation: 270 })).toEqual({ x: 450, y: 400, width: 150, height: 400 });
});

test("redact PDF only touches the first page unless more pages are checked", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Redact PDF");
    await chooseFixture(page, "/local/document.scoped.pdf");
    await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeChecked();
    await expect(page.getByRole("checkbox", { name: "Page 2" })).not.toBeChecked();
    await drawRedactionBox(page);
    await page.locator(".workspace-primary-action").click();
    expect((await lastInvocation(page, "redact_pdf"))?.args).toMatchObject({ request: { scope: { selected: { pages: [0] } } } });

    await page.getByRole("checkbox", { name: "Page 3" }).click();
    await page.locator(".workspace-primary-action").click();
    expect((await lastInvocation(page, "redact_pdf"))?.args).toMatchObject({ request: { scope: { selected: { pages: [0, 2] } } } });
});

test("TIFF Combine waits for inspection of the current file set", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Split and Combine TIFF");
    await page.getByRole("radiogroup", { name: "TIFF Pages mode" }).getByRole("radio", { name: "Combine into one TIFF" }).click();
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
    await confirmSave(page);
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
    await openFromHome(page, "Compress Images");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.evaluate(() => { (window as TestWindow).__toolboxProcessingDelayMs = 150; });
    await page.getByRole("button", { name: "Export Compress Images" }).click();
    await page.getByRole("toolbar", { name: "Compress Files tools" }).getByRole("button", { name: "Compress", exact: true }).click();
    await page.waitForTimeout(220);
    await expect(page.getByText("Test output", { exact: true })).toHaveCount(0);
});

test("command K focuses the tool search", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });

    await page.locator("body").press("Meta+K");

    await expect(search).toBeFocused();
});

test("Lock & Unlock exposes one cross-format document tool", async ({ page }) => {
    const utility = UtilityRegistry.find((item) => item.id === "lock-unlock");
    expect(utility).toBeDefined();
    expect(utility?.title).toBe("Lock & Unlock");
    expect(utility?.category).toBe("Documents");
    expect(utility?.command).toBe("remove_password");

    await page.goto("/");
    await openFromHome(page, "Lock & Unlock");
    await chooseFixture(page, pdfFixturePath);
    await fillPasswords(page, "test-password");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
});

test("Lock & Unlock locks DOCX and XLSX files in Lock mode now that Protect Office Files is retired", async ({ page }) => {
    expect(UtilityRegistry.find((item) => item.id === "office-protect")).toBeUndefined();
    expect(workspaceForTool("office-protect")).toBeUndefined();
    expect(ToolWorkspaceRegistry.find((workspace) => workspace.id === "file-security")?.toolIds).toEqual(["lock-unlock"]);

    await page.goto("/");
    await expect(page.getByRole("button", { name: "Open Protect Office Files" })).toHaveCount(0);
    await openFromHome(page, "Lock & Unlock");
    await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();
    await chooseFixture(page, "/local/document.docx");
    await fillPasswords(page, "test-password");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
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

const failed = (inputPath: string, kind: "invalidInput" | "unavailable" | "limitExceeded" | "processing" | "wrongPassword", message: string, extra: Partial<Extract<MockOutcome, { state: "failed" }>> = {}): MockOutcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind, message, pointer: null },
    reason: null,
    warnings: [],
    ...extra,
});

const overLimit = (inputPath: string, limit: LimitKind, message: string, extra: Partial<Extract<MockOutcome, { state: "failed" }>> = {}): MockOutcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind: "limitExceeded", details: { limit }, message, pointer: null },
    reason: null,
    warnings: [],
    ...extra,
});

const runCompressImagesWith = async (page: Page, outcomes: MockOutcome[]) => {
    await page.goto("/");
    await page.evaluate((outcomes) => {
        (window as TestWindow).__toolboxProcessingResults = outcomes;
    }, outcomes);
    await openFromHome(page, "Compress Images");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Export Compress Images" }).click();
    return page.getByRole("list", { name: "Results" }).getByRole("listitem");
};

const resultSummaryStatus = (page: Page) => page.getByRole("status").filter({ hasText: "Done." });

const fileActionInvocations = (page: Page) => page.evaluate(() =>
    ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) =>
        command === "open_output_path" || command === "reveal_output_path"
    ),
);

test("Compress rows show sizes and the saving, skips keep both sizes without file actions, and the batch total counts only compressed files", async ({ page }) => {
    await page.goto("/");
    const paths = ["/local/site-survey-scan.pdf", "/local/board-minutes-q3.pdf", "/local/site-notes.pdf", "/local/tax-return-2025.pdf", "/local/hero.pdf"];
    await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
        succeeded(paths[0], ["/local/site-survey-scan-compressed.pdf"], { detail: "PDF repacked without loss", originalBytes: 14_600_000, outputBytes: 3_800_000 }),
        {
            inputPath: paths[1],
            outputPaths: [],
            detail: "",
            state: "skipped",
            failure: null,
            reason: { code: "noSavings", message: "No savings: already well compressed", pointer: null },
            warnings: [],
            originalBytes: 640_000,
            outputBytes: 652_000,
        },
        failed(paths[2], "invalidInput", "Not a readable PDF. The file may be damaged or incomplete."),
        failed(paths[3], "invalidInput", "This PDF is password-protected. Unlock it first, then compress the unlocked copy.", {
            failure: { kind: "invalidInput", message: "This PDF is password-protected. Unlock it first, then compress the unlocked copy.", pointer: { kind: "tool", toolId: "lock-unlock" } },
        }),
        succeeded(paths[4], ["/local/hero-compressed.pdf"], { detail: "PDF repacked without loss", originalBytes: 10_800_000, outputBytes: 4_500_000 }),
    ] satisfies MockOutcome[]);
    await openFromHome(page, "Compress");
    await page.evaluate((paths) => { (window as TestWindow).__toolboxDialogResults = [paths]; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Compress 5 files", exact: true }).click();

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows).toHaveText([/^site-survey-scan\.pdf/, /^board-minutes-q3\.pdf/, /^site-notes\.pdf/, /^tax-return-2025\.pdf/, /^hero\.pdf/]);
    await expect(rows.nth(0)).toContainText("14.6 MB → 3.8 MB, 74% smaller");
    await expect(rows.nth(0)).toContainText("site-survey-scan-compressed.pdf");
    await expect(rows.nth(0).getByRole("button", { name: "Open site-survey-scan-compressed.pdf" })).toBeVisible();

    const skipped = rows.nth(1);
    await expect(skipped).toContainText("Skipped");
    await expect(skipped).toContainText("640 KB → 652 KB, no savings");
    await expect(skipped).toContainText("No savings: already well compressed");
    await expect(skipped.getByRole("button")).toHaveCount(0);

    await expect(rows.nth(2)).toContainText("Failed");
    await expect(rows.nth(2)).not.toContainText("→");
    await expect(rows.nth(3).getByRole("button", { name: "Open Lock & Unlock" })).toBeVisible();
    await expect(rows.nth(4)).toContainText("10.8 MB → 4.5 MB, 58% smaller");

    const summary = resultSummaryStatus(page);
    await expect(summary.locator("strong")).toHaveText("Done. 2 compressed, 1 skipped, 2 failed");
    await expect(summary).toContainText("Saved 17.1 MB, 67% smaller");
    await expect(summary).toContainText("25.4 MB → 8.3 MB across 2 compressed files");
    await expect(summary).toContainText("Counts only compressed files. Skipped and failed files kept their size and got no copy.");
});

test("every output exposes native file actions and keeps action errors inline", async ({ page }) => {
    const splitOutputs = ["a-01.pdf", "a-02.pdf", "a-12.pdf"].map((name) => `/local/out/${name}`);
    const folderOutput = "/local/out/brochure-pages";
    const partialOutput = "/local/out/drawings-part-1.pdf";
    const rows = await runCompressImagesWith(page, [
        succeeded(fixturePath, [mockedOutputPaths[0]], { detail: "Test output" }),
        succeeded("/local/annual-report.pdf", splitOutputs),
        succeeded("/local/brochure.pdf", [folderOutput]),
        overLimit("/local/drawings.pdf", "time", "Took longer than 5 minutes and was stopped. 1 part was saved before it stopped.", { outputPaths: [partialOutput] }),
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
            reason: { code: "alreadyProtected", message: "Already password-protected. Remove the password first, then protect it again.", pointer: { kind: "tool", toolId: "lock-unlock" } },
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
            failure: { kind: "unavailable", message: "Recognising Gujarati text needs the Indic script pack.", pointer: { kind: "pack", packId: "script-pack", name: "Indic script", reason: "notInstalled" } },
        }),
    ]);

    await expect(rows).toHaveCount(6);
    const names = ["lease-agreement.pdf", "vendor-nda.pdf", "invoice-0425.pdf", "menu-spring.docx", "site-survey-scan.pdf", "temple-scan.png"];
    for (const [index, name] of names.entries()) await expect(rows.nth(index)).toContainText(name);
    await expect(rows.nth(0)).toHaveAttribute("data-tone", "ok");
    await expect(rows.nth(1)).toHaveAttribute("data-tone", "warn");
    await expect(rows.nth(2)).toHaveAttribute("data-tone", "skip");
    await expect(rows.nth(4)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(5)).toHaveAttribute("data-tone", "fail");

    await expect(rows.nth(0)).toContainText("lease-agreement-protected.pdf");
    await expect(rows.nth(0)).toContainText("Compressed");
    await expect(rows.nth(1)).toContainText("Saved. The digital signature was removed, because changing the file breaks it.");
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
    await expect(rows.nth(5)).toContainText("Needs a pack");

    await expect(resultSummaryStatus(page)).toHaveAttribute("aria-live", "polite");
    await expect(resultSummaryStatus(page)).toContainText("Done. 3 compressed, 1 skipped, 1 failed, 1 needs a pack");
    await expect(resultSummaryStatus(page)).toContainText("2 compressed files have warnings.");
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

    await rows.nth(5).getByRole("button", { name: "Install Indic script pack for temple-scan.png", exact: true }).click();
    await expect.poll(() => page.evaluate(() => ((window as TestWindow).__toolboxInvocations ?? [])
        .filter(({ command }) => command === "install_pack")
        .map(({ args }) => (args as { packId: string }).packId))).toEqual(["script-pack"]);

    await rows.nth(2).getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
});

test("PDF to Images fails a missing renderer, protected and damaged PDFs per file beside the rendered ones", async ({ page }) => {
    const rendererUnavailable = "Not converted. The page renderer is unavailable. It's a bundled helper that ships inside Toolbox, so this is a problem with this copy of the app, not with your PDF. Reinstall Toolbox to restore it.";
    const passwordProtected = "It's password-protected. Remove its password, then convert it again.";
    await page.goto("/");
    await page.evaluate((outcomes) => {
        (window as TestWindow).__toolboxProcessingResults = outcomes;
    }, [
        succeeded("/local/brochure.pdf", ["/local/out/brochure-page-01.png"], { detail: "1 image, page 1" }),
        failed("/local/catalog.pdf", "unavailable", rendererUnavailable),
        failed("/local/payroll.pdf", "invalidInput", passwordProtected, {
            failure: { kind: "invalidInput", message: passwordProtected, pointer: { kind: "tool", toolId: "lock-unlock" } },
        }),
        failed("/local/truncated.pdf", "processing", "Not a readable PDF. The file may be damaged or incomplete. Nothing was saved."),
        succeeded("/local/report.pdf", ["/local/out/report-page-01.png"], { detail: "1 image, page 1" }),
    ]);
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, pdfFixturePath);
    await page.getByRole("button", { name: "Export PDF to Images", exact: true }).click();
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");

    await expect(rows).toHaveCount(5);
    await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(0)).toContainText("brochure.pdf");
    await expect(rows.nth(0)).toContainText("1 image, page 1");
    await expect(rows.nth(4)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(4)).toContainText("report.pdf");
    await expect(rows.nth(4)).toContainText("1 image, page 1");

    await expect(rows.nth(1)).toHaveAttribute("data-tone", "skip");
    await expect(rows.nth(1)).toContainText("Unavailable");
    await expect(rows.nth(1)).toContainText("bundled helper that ships inside Toolbox, so this is a problem with this copy of the app, not with your PDF");

    await expect(rows.nth(3)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(3)).toContainText("Not a readable PDF.");

    await expect(rows.nth(2)).toHaveAttribute("data-tone", "fail");
    await expect(rows.nth(2)).toContainText(passwordProtected);
    await expect(page.getByRole("button", { name: "Open Lock & Unlock", exact: true })).toHaveCount(1);
    await rows.nth(2).getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
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

test("Retry is offered only after processing failures and time limits and re-runs only that file with the same options", async ({ page }) => {
    const timedOut = "/local/drawings-full-set.pdf";
    const rows = await runCompressImagesWith(page, [
        succeeded(fixturePath, [mockedOutputPaths[0]]),
        overLimit(timedOut, "time", "Took longer than 5 minutes and was stopped. Nothing was saved."),
        failed("/local/scan.pdf", "invalidInput", "Wrong password or unsupported encryption."),
        overLimit("/local/site-plan.png", "fileSize", "Larger than the 250 MB limit for one image. Reduce its size and try again."),
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
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "compress_images"),
    );
    expect(runs).toHaveLength(2);
    const [first, retry] = runs.map((run) => (run.args as { request: { paths: string[] } }).request);
    expect(first.paths).toEqual([fixturePath]);
    expect(retry).toEqual({ ...first, paths: [timedOut] });
});

test("files over a limit fail alone with the limit message while the other rows render normally", async ({ page }) => {
    const limits: Array<[string, LimitKind, string]> = [
        ["/local/site-plan.png", "fileSize", "Larger than the 250 MB limit for one image. Reduce its size and try again."],
        ["/local/poster.png", "decompression", "This image expands to more than 500 MB when opened, which is over the limit. It may be damaged or unsafe to open."],
        ["/local/animation.gif", "pageCount", "More than 1,000 frames, over the limit for one GIF. Use fewer frames and try again."],
    ];
    const rows = await runCompressImagesWith(page, [
        succeeded(fixturePath, [mockedOutputPaths[0]], { detail: "Test output" }),
        ...limits.map(([path, limit, message]) => overLimit(path, limit, message)),
        failed("/local/broken.png", "processing", "The image could not be read."),
    ]);

    await expect(rows).toHaveCount(5);
    await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(0)).toContainText("icon.png.output-one");
    await expect(rows.nth(0)).toContainText("Test output");
    await expect(rows.nth(0).getByRole("button", { name: "Open icon.png.output-one", exact: true })).toBeVisible();
    for (const [index, [path, , message]] of limits.entries()) {
        const row = rows.nth(index + 1);
        await expect(row).toHaveAttribute("data-state", "failed");
        await expect(row).toContainText(path.split("/").pop()!);
        await expect(row).toContainText(message);
        await expect(row).toContainText("Failed, over the limit");
        await expect(row.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
    }
    await expect(rows.nth(4)).toContainText("The image could not be read.");
    await expect(rows.nth(4).getByRole("button", { name: "Retry broken.png", exact: true })).toBeVisible();
    await expect(resultSummaryStatus(page)).toHaveText("Done. 1 compressed, 4 failed");
});

test("a merge over the page limit names the input that crossed it and every other row says the merge stopped there", async ({ page }) => {
    const files = ["/local/leases/lease-agreement.pdf", "/local/leases/lease-addendum.pdf", "/local/leases/lease-schedule.pdf"];
    const limitMessage = "Adding lease-addendum.pdf takes the merged PDF past 5,000 pages, over the limit for one PDF. Merge fewer pages and try again.";
    const stopped = `Aggregate operation stopped because ${files[1]} failed: ${limitMessage}`;
    await page.goto("/");
    await page.getByRole("button", { name: "Open Merge PDF" }).click();
    await page.evaluate((files) => { (window as TestWindow).__toolboxDialogResults = [[...files]]; }, files);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
        failed(files[0], "processing", stopped),
        overLimit(files[1], "pageCount", limitMessage),
        failed(files[2], "processing", stopped),
    ]);
    await page.locator(".workspace-primary-action").click();
    await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(1)).toHaveAttribute("data-state", "failed");
    await expect(rows.nth(1)).toContainText("lease-addendum.pdf");
    await expect(rows.nth(1)).toContainText(limitMessage);
    await expect(rows.nth(1)).toContainText("Failed, over the limit");
    await expect(rows.nth(1).getByRole("button", { name: /^Retry/ })).toHaveCount(0);
    for (const index of [0, 2]) {
        await expect(rows.nth(index)).toHaveAttribute("data-state", "failed");
        await expect(rows.nth(index)).toContainText(`stopped because ${files[1]} failed`);
        await expect(rows.nth(index)).not.toContainText("Failed, over the limit");
    }
    await expect(page.getByRole("button", { name: /^Open .*merged/ })).toHaveCount(0);
});

test("a split over the page limit fails alone and the next PDF in the batch splits normally", async ({ page }) => {
    const limitMessage = "More than 5,000 pages, over the limit for one PDF. Use fewer pages and try again.";
    const longFile = "/local/split/drawings-full-set.pdf";
    const goodFile = "/local/split/annual-report.pdf";
    const goodOutputs = ["/local/split/annual-report-split-page-1.pdf", "/local/split/annual-report-split-page-2.pdf"];
    await page.goto("/");
    await openFromHome(page, "Split PDF");
    await page.getByRole("radiogroup", { name: "Split by" }).getByRole("radio", { name: "Every N pages" }).click();
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");

    await chooseFixture(page, longFile);
    await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [overLimit(longFile, "pageCount", limitMessage)]);
    await page.getByRole("button", { name: "Export Split PDF" }).click();
    await expect(rows).toHaveCount(1);
    await expect(rows.nth(0)).toHaveAttribute("data-state", "failed");
    await expect(rows.nth(0)).toContainText("drawings-full-set.pdf");
    await expect(rows.nth(0)).toContainText(limitMessage);
    await expect(rows.nth(0)).toContainText("Failed, over the limit");

    await chooseFixture(page, goodFile);
    await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [succeeded(goodFile, goodOutputs)]);
    await page.getByRole("button", { name: "Export Split PDF" }).click();
    await expect(rows).toHaveCount(1);
    await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(0)).not.toContainText(limitMessage);
    await expect(rows.nth(0)).toContainText("2 files, annual-report-split-page-1.pdf to annual-report-split-page-2.pdf");
    await expect(rows.nth(0).getByRole("button", { name: "Open annual-report-split-page-1.pdf", exact: true })).toBeVisible();
    await expect(resultSummaryStatus(page)).toHaveText("Done. 1 split");
});

test("output action state resets and ignores stale completions", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Compress Images");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    const processButton = page.getByRole("button", { name: "Export Compress Images" });
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
    await openFromHome(page, "Merge PDF");
    await chooseFixture(page, pdfFixturePath);
    await page.evaluate(() => { (window as TestWindow).__toolboxSecondPick = true; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("toolbar", { name: "PDF Editor tools" }).getByRole("button", { name: "Split PDF", exact: true }).click();
    await page.getByRole("textbox", { name: "Page ranges" }).fill("1");

    const exportButton = page.getByRole("button", { name: "Export Split PDF", exact: true });
    await expect(exportButton).toBeEnabled();
    await exportButton.click();

    await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 0 split, 2 failed");
    const processingInvocations = await page.evaluate(() =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "split_pdf"),
    );
    expect(processingInvocations).toHaveLength(0);
    await expect(page.getByText("This action accepts one input file, but 2 were selected.", { exact: true })).toHaveCount(2);
});

for (const batch of [
    { title: "Compress Images", command: "compress_images", paths: ["/local/beach.png", "/local/forest.jpg"] },
    { title: "Compress", command: "compress_files", paths: ["/local/lease.pdf", "/local/invoice.pdf"] },
    { title: "Extract GIF Frames", command: "extract_gif_frames", paths: ["/local/wave.gif", "/local/spin.gif"] },
]) {
    test(`${batch.title} runs every selected file in one native batch`, async ({ page }) => {
        await page.goto("/");
        await openFromHome(page, batch.title);
        await page.evaluate((paths) => { (window as TestWindow).__toolboxDialogResults = [paths]; }, batch.paths);
        await page.getByRole("button", { name: "Choose files to process" }).click();
        await expect(page.locator(".file-selection-count")).toHaveText("2 files");

        await page.locator(".workspace-primary-action").click();
        await expect.poll(() => invocationCount(page, batch.command)).toBe(1);
        const request = await page.evaluate((command) => ((window as TestWindow).__toolboxInvocations ?? [])
            .find((invocation) => invocation.command === command)?.args as { request: { paths: string[] } }, batch.command);
        expect(request.request.paths).toEqual(batch.paths);
        await expect(page.getByText(/This action accepts one input file/)).toHaveCount(0);
    });
}

test("single-input replacement keeps the source and edit state after cancellation or the same path", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Edit PDF");
    await chooseFixture(page, pdfFixturePath);
    await drawSceneMark(page, "Crop");
    await expect(page.getByRole("button", { name: "Reset…" })).toBeEnabled();

    await page.evaluate((path) => { (window as TestWindow).__toolboxDialogResults = [null, path]; }, pdfFixturePath);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await expect(page.getByText(pdfFixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset…" })).toBeEnabled();

    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file");
    await expect(page.getByText(pdfFixtureName, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reset…" })).toBeEnabled();
});

test("page-scoped actions disable export when the explicit selection is empty", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, pdfFixturePath);
    const pageCheckbox = page.getByRole("checkbox", { name: "Page 1" });
    await expect(pageCheckbox).toBeChecked();
    await pageCheckbox.click();
    await expect(page.getByRole("button", { name: "Export PDF to Images", exact: true })).toBeDisabled();
});

test("PDF replacement clears stale inspection state while inspection is pending or rejected", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Extract PDF Pages");
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
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, "/local/document.scoped.pdf");
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeVisible();
    await page.getByRole("radiogroup", { name: "Pages" }).getByRole("radio", { name: "Range" }).click();
    await page.getByRole("textbox", { name: "Page range for every PDF" }).fill("1");
    await expect(page.getByRole("checkbox", { name: "Page 2" })).not.toBeChecked();

    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "Extract from PDF", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toHaveCount(0);
    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "PDF to Images", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeVisible();

    await page.getByRole("radiogroup", { name: "Pages" }).getByRole("radio", { name: "Range" }).click();
    await expect(page.getByRole("textbox", { name: "Page range for every PDF" })).toHaveValue("");
    await page.getByRole("radiogroup", { name: "Pages" }).getByRole("radio", { name: "All pages" }).click();
    await expect(page.getByRole("checkbox", { name: "Page 2" })).toBeChecked();
    await page.getByRole("button", { name: "Export PDF to Images", exact: true }).click();

    const invocation = await page.evaluate(() =>
        (window as TestWindow).__toolboxInvocations?.find(({ command }) => command === "pdf_to_images"),
    );
    expect(invocation?.args).toMatchObject({ request: { picks: [], pageRange: null } });
});

test("PDF page ranges follow the native grammar and messages", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "PDF to Images");
    await chooseFixture(page, "/local/document.scoped.pdf");
    await page.getByRole("radiogroup", { name: "Pages" }).getByRole("radio", { name: "Range", exact: true }).click();
    const range = page.getByRole("textbox", { name: "Page range for every PDF" });
    const exportButton = page.getByRole("button", { name: "Export PDF to Images", exact: true });

    await range.fill("1-3,,2");
    await expect(page.getByText("Entry 2 of the page range is empty. Remove the extra comma.", { exact: true })).toBeVisible();
    await expect(exportButton).toBeDisabled();

    // One range applies to every PDF, so pages past a PDF's last page are left out instead of rejected.
    await range.fill("4");
    await expect(page.getByText("Range 4 applied to each PDF. 1 will be skipped.", { exact: true })).toBeVisible();

    await range.fill("2-");
    await expect(page.getByText("Range 2- applied to each PDF.", { exact: true })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: "Page 1" })).not.toBeChecked();
    await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeChecked();
    await exportButton.click();
    expect((await lastInvocation(page, "pdf_to_images"))?.args).toMatchObject({ request: { pageRange: "2-", picks: [] } });
});

test("GIF extraction does not request a source preview", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Extract GIF Frames");
    await chooseFixture(page, "/local/animation.gif");
    await expect(page.getByLabel("Tool detail").getByRole("button", { name: "Frames", exact: true })).toBeEnabled();

    const previewInvocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations?.filter(({ command }) => command === "inspect_image_preview") ?? []);
    expect(previewInvocations).toHaveLength(0);
});

type RunRequest = { outputLocation?: unknown; plan?: { outputLocation?: unknown; edits?: unknown[] } };

const storeOutputSettings = async (page: Page, settings: OutputSettings) => {
    await page.addInitScript((settings) => { (window as TestWindow).__toolboxOutputSettings = settings; }, settings);
};

const exerciseFeature = async (page: Page, utility: (typeof UtilityRegistry)[number], expectedOutputLocation: unknown = "alongsideInput") => {
    await page.goto("/");
    await openFromHome(page, utility.title);
    // Image Metadata and Convert Image Format live in the Image Editor but run on their own screen, without an edit plan.
    // Crop runs through the shared edit stack, which Crop fills with its default step.
    const usesImageEditPlan = workspaceForTool(utility.id)?.id === "image-editor" && utility.id !== "image-metadata" && utility.id !== "heic-convert" && utility.id !== "crop";

    const accepts = (extension: string) => utility.capability.acceptedExtensions.some((accepted) => accepted.toLowerCase() === extension);
    const selectedFixturePath = utility.id === "convert-to-pdf"
        ? "/local/notes.txt"
        : accepts(".pdf")
        ? pdfFixturePath
        : accepts(".png")
            ? fixturePath
            : `/local/fixture${utility.capability.acceptedExtensions[0]}`;
    await chooseFixture(page, selectedFixturePath);

    if (utility.id === "lock-unlock") {
        await fillPasswords(page, "test-password");
    }
    if (isPdfSceneTool(utility.id)) {
        if (utility.id === "pdf-page-numbers") await page.getByRole("button", { name: "Add page numbers", exact: true }).click();
        else if (utility.id === "pdf-organize") {
            await page.getByRole("button", { name: "Open in page view", exact: true }).click();
            await page.getByRole("button", { name: "Rotate selected pages", exact: true }).click();
        } else if (utility.id === "pdf-remove-pages") await page.getByRole("button", { name: "Rotate selected pages", exact: true }).click(); else if (utility.id === "pdf-watermark") await page.getByRole("button", { name: "Add watermark", exact: true }).click();
        else await drawSceneMark(page, utility.id === "pdf-sign" ? "Signature" : utility.id === "pdf-watermark" ? "Watermark" : "Text");
    }
    if (usesImageEditPlan) {
        if (utility.id === "resize") await page.getByLabel("Width").fill("640");
        if (utility.id === "rotate") await page.getByLabel("Rotation").selectOption("90");
        if (utility.id === "image-watermark") await page.getByLabel("Watermark text").fill("Test watermark");
        if (utility.id === "image-tone") await page.getByRole("slider", { name: "Brightness" }).press("End");
    }
    if (utility.id === "compress") await page.getByRole("switch", { name: "Preserve original pixels" }).click();
    if (utility.id === "pdf-redact") await drawRedactionBox(page);
    if (utility.id === "pdf-properties") await page.getByRole("group", { name: "Title", exact: true }).getByRole("radio", { name: "Clear", exact: true }).click();
    if (utility.id === "pdf-split") await page.getByRole("textbox", { name: "Page ranges" }).fill("1-");
    if (utility.id === "pdf-merge" || utility.id === "pdf-compare") {
        await page.evaluate(() => { (window as TestWindow).__toolboxSecondPick = true; });
        await page.getByRole("button", { name: "Choose files to process" }).click();
    }
    if (usesImageEditPlan) {
        await page.getByRole("button", { name: "Add edit to plan" }).click();
    }

    const action = page.locator(".workspace-primary-action");
    await expect(action).toBeEnabled();
    await action.click();
    // A read-only report tool sends only the paths and shows a report instead of saved results.
    // (Convert to PDF also has no suffix, since the output changes extension, but it saves files.)
    if (utility.capability.outputSuffix === "" && !utility.capability.formatFamilies) {
        await expect(page.getByRole("region", { name: pdfFixtureName, exact: true }).getByText("None found", { exact: true })).toHaveCount(14);
        const reportRuns = await page.evaluate((command) => ((window as TestWindow).__toolboxInvocations ?? []).filter((call) => call.command === command), utility.command);
        expect(reportRuns.map(({ args }) => (args as { request: unknown }).request)).toEqual([{ paths: [selectedFixturePath], password: "", passwordOverrides: {} }]);
        return;
    }
    if (["pdf-merge", "pdf-compare", "images-to-pdf", "gif-create"].includes(utility.id)) await confirmSave(page);

    await expect(page.getByText("Test output", { exact: true })).toBeVisible();
    const invocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []);
    const composedCommand = usesImageEditPlan
        ? "export_image_edit_plan"
        : isPdfSceneTool(utility.id)
            ? "export_pdf_scene"
            : utility.command;
    const run = invocations.find(({ command }) => command === composedCommand);
    expect(run).toBeDefined();
    const request = (run?.args as { request: RunRequest }).request;
    expect(composedCommand === "export_image_edit_plan" ? request.plan?.outputLocation : request.outputLocation).toEqual(expectedOutputLocation);
    if (utility.id === "image-watermark") {
        expect(request.plan?.edits).toContainEqual({ kind: "watermark", text: "Test watermark", opacity: 20, position: { x: 0, y: 0 } });
        // Edit PDF watermarks send the same shared option set (tests/ui/pdf-scene.spec.ts).
        const watermarkEdit = request.plan?.edits?.find((edit: { kind: string }) => edit.kind === "watermark");
        expect(Object.keys(watermarkEdit ?? {}).filter((key) => key !== "kind").every((key) => ["text", "logoPath", "size", "opacity", "color", "position", "rotation"].includes(key))).toBe(true);
    }
};

test.describe("registered feature actions", () => {
    expect(UtilityRegistry).toHaveLength(37);

    for (const utility of UtilityRegistry) {
        test(`${utility.id} accepts the fixture and runs with the Settings default`, async ({ page }) => {
            test.skip(utility.status !== "implemented", "Tool is not available in this build.");
            await exerciseFeature(page, utility);
        });
    }
});

test.describe("workspace runs send the stored output location", () => {
    const representativeTools = ["lock-unlock", "pdf-edit", "pdf-to-images", "resize", "compress-files", "icon-set"] as const;
    const customFolder = { customFolder: "/fake/shell/output" };

    test("one representative tool covers every workspace view", () => {
        expect(representativeTools.map((id) => workspaceForTool(id)?.id)).toEqual(["file-security", "pdf-editor", "pdf-convert", "image-editor", "compress-files", "media-tools"]);
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
        await openFromHome(page, "Resize Images");
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
        .filter(({ command }) => command === "export_image_edit_plan" || command === "protect_files")
        .map(({ command, args }) => {
            const request = (args as { request: RunRequest }).request;
            return command === "export_image_edit_plan" ? request.plan?.outputLocation : request.outputLocation;
        }));

const openResizeWithFixture = async (page: Page) => {
    await openFromHome(page, "Resize Images");
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
    test("example names use the extension each tool writes", async ({ page }) => {
        await page.goto("/");
        const example = page.getByRole("region", { name: "Destination" }).getByTestId("run-destination-example");

        await openFromHome(page, "Convert Image Format");
        await chooseFixture(page, fixturePath);
        await expect(example).toHaveText("icon-converted.jpg");
        await page.getByRole("radiogroup", { name: "Convert to" }).getByRole("radio", { name: "WebP", exact: true }).click();
        await expect(example).toHaveText("icon-converted.webp");

        await page.goto("/");
        await openFromHome(page, "Extract from PDF");
        await chooseFixture(page, pdfFixturePath);
        await expect(example).toHaveText("document-text.txt");
        await page.getByRole("radio", { name: "Images", exact: true }).click();
        await expect(example).toHaveText("document-images");
        await expect(page.getByText("JPEG and JPEG 2000 are copied as they are; others become lossless PNG. An existing folder is never reused: the new one gets -1, -2 and so on.", { exact: true })).toBeVisible();
        await page.getByRole("radio", { name: "Both", exact: true }).click();
        await expect(page.getByText("An existing file or folder is never replaced: the new one gets -1, -2 and so on.", { exact: true })).toBeVisible();

        await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "PDF to Images", exact: true }).click();
        await expect(example).toHaveText("document-page-01.png");
        await page.getByRole("radiogroup", { name: "Format" }).getByRole("radio", { name: "JPEG" }).click();
        await expect(example).toHaveText("document-page-01.jpg");
    });

    test("shows the default destination and the name the image editor export writes", async ({ page }) => {
        await page.goto("/");
        await openFromHome(page, "Resize Images");
        const destination = page.getByRole("region", { name: "Destination" });
        await expect(destination.getByTestId("run-destination-path")).toHaveText("Next to each original");
        await expect(destination.getByTestId("run-destination-example")).toHaveCount(0);
        await chooseFixture(page, fixturePath);
        await expect(destination.getByTestId("run-destination-example")).toHaveText("icon-edited.png");
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
        await openFromHome(page, "Resize Images");
        const destination = page.getByRole("region", { name: "Destination" });
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/kept/output");
        await changeForRun(page, "/picked/run");
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/picked/run");
        await expect(destination.getByText("Your default is /kept/output. This change ends when you leave Resize Images.")).toBeVisible();

        await page.getByRole("button", { name: "← All tools" }).click();
        await expect(destination).toHaveCount(0);
        await openFromHome(page, "Resize Images");
        await expect(destination.getByTestId("run-destination-path")).toHaveText("/kept/output");
        await expect(destination.getByText("This run only")).toHaveCount(0);
        expect(await outputSettingsWrites(page)).toEqual([]);
    });

    test("switching to another tool in the workspace resets the per-run folder", async ({ page }) => {
        await page.goto("/");
        await openFromHome(page, "Resize Images");
        const destination = page.getByRole("region", { name: "Destination" });
        await changeForRun(page, "/picked/run");
        await expect(destination.getByText("This run only")).toBeVisible();

        await page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Rotate", exact: true }).click();
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
        await openFromHome(page, "Lock & Unlock");
        await page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Lock", exact: true }).click();
        await chooseFixture(page, pdfFixturePath);
        await expect(page.getByRole("region", { name: "Destination" }).getByTestId("run-destination-example")).toHaveText("document-protected.pdf");
        await fillPasswords(page, "test-password");
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

test.describe("many-to-one save step", () => {
    const saveStepCases = [
        { tool: "Merge PDF", command: "merge_pdfs", files: ["/local/leases/lease-agreement.pdf", "/local/leases/lease-addendum.pdf"], title: "Save the merged PDF", name: "lease-agreement-merged.pdf", summary: "2 files, joined top to bottom.", runLabel: "Merge…" },
        { tool: "Images to PDF", command: "images_to_pdf", files: ["/local/scans/IMG_4012.jpg", "/local/scans/receipt-scan.png"], title: "Save the combined PDF", name: "IMG_4012-combined.pdf", summary: "2 images, one per page.", runLabel: "Make PDF…" },
        { tool: "Create GIF", command: "create_gif", files: ["/local/frames/frame-1.png", "/local/frames/frame-2.png"], title: "Save the GIF", name: "frame-1-animated.gif", summary: "2 frames, in the order shown.", runLabel: "Make GIF…" },
        { tool: "Split and Combine TIFF", command: "process_tiff_pages", files: ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"], title: "Save the combined TIFF", name: "two-page-combined.tiff", summary: "2 TIFF files, combined in the order shown.", runLabel: "Make TIFF…", mode: "Combine into one TIFF" },
    ] as const;
    const merge = saveStepCases[0];

    const openWithFiles = async (page: Page, tool: string, files: readonly string[], mode?: string) => {
        await page.goto("/");
        await page.getByRole("button", { name: `Open ${tool}` }).click();
        if (mode) await page.getByRole("radio", { name: mode }).click();
        await page.evaluate((files) => { (window as TestWindow).__toolboxDialogResults = [[...files]]; }, files);
        await page.getByRole("button", { name: "Choose files to process" }).click();
    };
    const startRun = async (page: Page) => {
        const action = page.locator(".workspace-primary-action");
        await expect(action).toBeEnabled();
        await action.click();
        return page.getByRole("dialog");
    };
    const requestsFor = (page: Page, command: string) => page.evaluate((command) =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter((invocation) => invocation.command === command).map(({ args }) => (args as { request: Record<string, unknown> }).request),
    command);
    const commandCount = (page: Page, command: string) => page.evaluate((command) =>
        ((window as TestWindow).__toolboxInvocations ?? []).filter((invocation) => invocation.command === command).length,
    command);

    for (const testCase of saveStepCases) {
        test(`${testCase.command} asks for a name and folder in the app before it runs`, async ({ page }) => {
            await openWithFiles(page, testCase.tool, testCase.files, "mode" in testCase ? testCase.mode : undefined);
            const destination = page.getByRole("region", { name: "Destination" });
            await expect(destination).toContainText("You choose a name and folder when you start, because this makes one file.");
            await expect(destination.getByTestId("run-destination-example")).toHaveCount(0);
            await expect(destination.getByTestId("run-destination-path")).toHaveCount(0);
            await expect(destination.getByRole("button", { name: "Change for this run" })).toHaveCount(0);
            await expect(page.locator(".workspace-primary-action")).toHaveText(`${testCase.runLabel}⌘↵`);

            const dialog = await startRun(page);
            const folder = testCase.files[0].slice(0, testCase.files[0].lastIndexOf("/"));
            await expect(dialog.getByRole("heading", { name: testCase.title })).toBeVisible();
            await expect(dialog.getByLabel("File name")).toHaveValue(testCase.name);
            await expect(dialog.getByTestId("save-folder")).toHaveText(folder);
            await expect(dialog).toHaveAccessibleDescription(testCase.summary);
            await expect(page.getByText(/Processing \d+ of \d+ files/)).toHaveCount(0);
            expect(await requestsFor(page, testCase.command)).toEqual([]);

            await dialog.getByRole("button", { name: "Save" }).click();
            await expect(dialog).toHaveCount(0);
            await expect.poll(() => requestsFor(page, testCase.command)).toHaveLength(1);
            const [request] = await requestsFor(page, testCase.command);
            expect(request).toMatchObject({ paths: testCase.files, outputLocation: "alongsideInput", outputPath: `${folder}/${testCase.name}` });
            expect(await commandCount(page, "plugin:dialog|save")).toBe(0);
        });
    }

    for (const theme of ["light", "dark"] as const) {
        test(`the proposed folder follows the custom output location and reads from home in the ${theme} theme`, async ({ page }) => {
            await page.setViewportSize({ width: 1280, height: 800 });
            await page.addInitScript((theme) => { localStorage.setItem("toolbox-theme", theme); }, theme);
            await storeOutputSettings(page, { mode: "customFolder", folder: "/Users/test/Documents/Leases" });
            await openWithFiles(page, merge.tool, merge.files);
            const dialog = await startRun(page);
            await expect(dialog.getByTestId("save-folder")).toHaveText("~/Documents/Leases");
            await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
            const screenshotDir = process.env.SAVE_STEP_SCREENSHOT_DIR;
            if (screenshotDir) await page.screenshot({ path: path.join(screenshotDir, `save-dialog-${theme}.png`), animations: "disabled" });

            await dialog.getByRole("button", { name: "Save" }).click();
            await expect.poll(() => requestsFor(page, "merge_pdfs")).toHaveLength(1);
            expect((await requestsFor(page, "merge_pdfs"))[0]).toMatchObject({ outputLocation: { customFolder: "/Users/test/Documents/Leases" }, outputPath: "/Users/test/Documents/Leases/lease-agreement-merged.pdf" });
        });
    }

    for (const testCase of saveStepCases) {
        test(`cancelling the ${testCase.command} save dialog sends no request`, async ({ page }) => {
            await openWithFiles(page, testCase.tool, testCase.files, "mode" in testCase ? testCase.mode : undefined);
            const dialog = await startRun(page);
            await dialog.getByRole("button", { name: "Cancel" }).click();
            await expect(dialog).toHaveCount(0);
            await expect(page.locator(".workspace-primary-action")).toBeEnabled();
            expect(await requestsFor(page, testCase.command)).toEqual([]);
            await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveCount(0);
        });
    }

    test("Escape closes the save dialog without a request", async ({ page }) => {
        await openWithFiles(page, merge.tool, merge.files);
        const dialog = await startRun(page);
        await expect(dialog.getByLabel("File name")).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(dialog).toHaveCount(0);
        expect(await requestsFor(page, "merge_pdfs")).toEqual([]);
    });

    test("Change… picks another folder with the native folder dialog", async ({ page }) => {
        await openWithFiles(page, merge.tool, merge.files);
        const dialog = await startRun(page);
        await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = ["/Users/test/Desktop/Signed"]; });
        await dialog.getByRole("button", { name: "Change…" }).click();
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Desktop/Signed");
        const folderDialogs = await page.evaluate(() =>
            ((window as TestWindow).__toolboxInvocations ?? []).filter(({ command }) => command === "plugin:dialog|open").map(({ args }) => (args as { options: unknown }).options),
        );
        expect(folderDialogs.at(-1)).toMatchObject({ directory: true, multiple: false, defaultPath: "/local/leases" });

        await page.evaluate(() => { (window as TestWindow).__toolboxDialogResults = [null]; });
        await dialog.getByRole("button", { name: "Change…" }).click();
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Desktop/Signed");

        await dialog.getByRole("button", { name: "Save" }).click();
        await expect.poll(() => requestsFor(page, "merge_pdfs")).toHaveLength(1);
        expect((await requestsFor(page, "merge_pdfs"))[0]).toMatchObject({ outputPath: "/Users/test/Desktop/Signed/lease-agreement-merged.pdf" });
    });

    test("the file name must be present, stay in the folder, and gets the tool's extension", async ({ page }) => {
        await openWithFiles(page, merge.tool, merge.files);
        const dialog = await startRun(page);
        const name = dialog.getByLabel("File name");
        const save = dialog.getByRole("button", { name: "Save" });

        await name.fill("   ");
        await expect(dialog.getByRole("alert")).toHaveText("Enter a file name.");
        await expect(name).toHaveAttribute("aria-invalid", "true");
        await expect(save).toBeDisabled();
        await name.press("Enter");

        await name.fill("../lease");
        await expect(dialog.getByRole("alert")).toHaveText("A file name can’t contain / or \\.");
        await expect(save).toBeDisabled();
        await name.fill("folder\\lease.pdf");
        await expect(save).toBeDisabled();
        expect(await requestsFor(page, "merge_pdfs")).toEqual([]);

        await name.fill("Lease pack");
        await expect(dialog.getByRole("alert")).toHaveCount(0);
        await name.press("Enter");
        await expect.poll(() => requestsFor(page, "merge_pdfs")).toHaveLength(1);
        expect((await requestsFor(page, "merge_pdfs"))[0]).toMatchObject({ outputPath: "/local/leases/Lease pack.pdf" });
    });

    test("a name that already has the extension keeps it once", async ({ page }) => {
        await openWithFiles(page, merge.tool, merge.files);
        const dialog = await startRun(page);
        await dialog.getByLabel("File name").fill("Lease Pack.PDF");
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect.poll(() => requestsFor(page, "merge_pdfs")).toHaveLength(1);
        expect((await requestsFor(page, "merge_pdfs"))[0]).toMatchObject({ outputPath: "/local/leases/Lease Pack.PDF" });
    });

    test("TIFF Split mode runs without the save step for one file or several", async ({ page }) => {
        await openWithFiles(page, "Split and Combine TIFF", ["/local/tiff/two-page.tiff"]);
        await expect(page.locator(".workspace-primary-action")).toHaveText("Split 1 TIFF⌘↵");
        await page.locator(".workspace-primary-action").click();
        await expect.poll(() => requestsFor(page, "process_tiff_pages")).toHaveLength(1);
        await expect(page.getByRole("dialog")).toHaveCount(0);
        const [request] = await requestsFor(page, "process_tiff_pages");
        expect(request).toMatchObject({ mode: "split", paths: ["/local/tiff/two-page.tiff"] });
        expect(request).not.toHaveProperty("outputPath");
        expect(request).not.toHaveProperty("pages");
    });

    test("TIFF Combine mode opens the save step even for one file", async ({ page }) => {
        await openWithFiles(page, "Split and Combine TIFF", ["/local/tiff/two-page.tiff"], "Combine into one TIFF");
        const dialog = await startRun(page);
        await expect(dialog.getByLabel("File name")).toHaveValue("two-page-combined.tiff");
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect.poll(() => requestsFor(page, "process_tiff_pages")).toHaveLength(1);
        expect((await requestsFor(page, "process_tiff_pages"))[0]).toMatchObject({ mode: "combine", outputPath: "/local/tiff/two-page-combined.tiff" });
    });

    for (const theme of ["light", "dark"] as const) {
        test(`a renamed save shows one row named by the output with the warning once in the ${theme} theme`, async ({ page }) => {
            await page.setViewportSize({ width: 1280, height: 800 });
            await page.addInitScript((theme) => { localStorage.setItem("toolbox-theme", theme); }, theme);
            await openWithFiles(page, merge.tool, ["/local/statements/statement-q3.pdf", "/local/statements/statement-q4.pdf"]);
            const renamed = { code: "renamedToAvoidReplacing" as const, message: "native wording", details: { chosenName: "statement-q3-merged.pdf", finalName: "statement-q3-merged-1.pdf" } };
            await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
                succeeded("/local/statements/statement-q3.pdf", ["/local/statements/statement-q3-merged-1.pdf"], { warnings: [renamed] }),
            ]);
            const dialog = await startRun(page);
            await dialog.getByRole("button", { name: "Save" }).click();
            const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
            await expect(rows).toHaveCount(1);
            await expect(rows.nth(0)).toHaveAttribute("data-tone", "warn");
            const rowName = rows.nth(0).locator(".result-row-name");
            await expect(rowName).toHaveText("statement-q3-merged-1.pdf");
            await expect(rowName).toHaveAttribute("title", "/local/statements/statement-q3-merged-1.pdf");
            await expect(rows.nth(0).getByText("Saved as statement-q3-merged-1.pdf. Renamed to avoid replacing an existing file.")).toHaveCount(1);
            await expect(rows.nth(0)).not.toContainText("native wording");
            await expect(resultSummaryStatus(page)).toContainText("Done. 1 merged");
            await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
            const screenshotDir = process.env.SAVE_STEP_SCREENSHOT_DIR;
            if (screenshotDir) {
                await rows.nth(0).scrollIntoViewIfNeeded();
                await page.screenshot({ path: path.join(screenshotDir, `save-result-${theme}.png`), animations: "disabled" });
            }
        });
    }

    test("a combined result without a warning is named by the output and drops the repeated output line", async ({ page }) => {
        await openWithFiles(page, merge.tool, merge.files);
        await page.evaluate((outcomes) => { (window as TestWindow).__toolboxProcessingResults = outcomes; }, [
            succeeded("/local/leases/lease-agreement.pdf", ["/local/leases/lease-agreement-merged.pdf"]),
        ]);
        const dialog = await startRun(page);
        await dialog.getByRole("button", { name: "Save" }).click();
        const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(rows).toHaveCount(1);
        await expect(rows.nth(0)).toHaveAttribute("data-tone", "ok");
        await expect(rows.nth(0).locator(".result-row-name")).toHaveText("lease-agreement-merged.pdf");
        await expect(rows.nth(0).locator(".result-row-detail")).toHaveCount(0);
        await expect(resultSummaryStatus(page)).toContainText("Done. 1 merged");
    });

    test("a created-document save opens in the last-used folder, or else Documents", async ({ page }) => {
        await page.goto("/");
        const chooseCreated = () => page.evaluate(`import("/src/components/saveStep.ts").then((step) => step.chooseCreatedDocumentSavePath({ title: "Save the new PDF", extension: "pdf" }, "Untitled.pdf"))`);
        const dialog = page.getByRole("dialog", { name: "Save the new PDF" });

        const first = chooseCreated();
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Documents");
        await expect(dialog.getByLabel("File name")).toHaveValue("Untitled.pdf");
        await dialog.getByRole("button", { name: "Save" }).click();
        expect(await first).toBe("/Users/test/Documents/Untitled.pdf");

        await page.evaluate(() => { (window as TestWindow).__toolboxLastSaveFolder = "/Users/test/Leases"; });
        const second = chooseCreated();
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Leases");
        await dialog.getByRole("button", { name: "Save" }).click();
        expect(await second).toBe("/Users/test/Leases/Untitled.pdf");

        const cancelled = chooseCreated();
        await dialog.getByRole("button", { name: "Cancel" }).click();
        expect(await cancelled).toBeNull();
        expect(await commandCount(page, "plugin:dialog|save")).toBe(0);
    });
});

test("PDF Editor rail lists Details and More tools after their labels and keeps files across switches", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Rotate PDFs");
    const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
    await expect(rail.getByRole("button")).toHaveText(["Edit PDF", "Organize PDF", "Merge PDF", "Split PDF", "Crop and Resize PDF", "Add PDF Page Numbers", "Redact PDF", "Privacy Check", "Edit Properties", "Rotate PDFs", "Repair PDF", "Compare PDFs", "Extract PDF Pages"].map((title) => new RegExp(`^${title}`)));
    await expect(rail.locator(".workspace-command-group")).toHaveText(["Details", "More"]);
    await expect(rail.locator(".workspace-command-group + button")).toHaveText([/^Privacy Check/, /^Rotate PDFs/]);
    await expect(rail.getByRole("button", { name: "Rotate PDFs" })).toHaveAttribute("aria-pressed", "true");

    await chooseFixture(page, "/local/document.scoped.pdf");
    await rail.getByRole("button", { name: "Repair PDF" }).click();
    await expect(rail.getByRole("button", { name: "Repair PDF" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("document.scoped.pdf", { exact: true })).toBeVisible();
    await page.locator(".workspace-primary-action").click();
    expect((await lastInvocation(page, "repair_pdf"))?.args).toMatchObject({ request: { paths: ["/local/document.scoped.pdf"] } });
});

test("Edit Image tabs switch with arrow keys and keep the files and the edit plan", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Image Editor", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    const tabs = page.getByRole("tablist", { name: "Edit Image" });
    await expect(tabs.getByRole("tab")).toHaveText(["Resize", "Rotate", "Crop", "Watermark", "Adjust"]);
    await tabs.getByRole("tab", { name: "Resize", exact: true }).click();
    await page.getByLabel("Width").fill("320");

    await tabs.getByRole("tab", { name: "Resize", exact: true }).press("ArrowRight");
    const rotate = tabs.getByRole("tab", { name: "Rotate", exact: true });
    await expect(rotate).toHaveAttribute("aria-selected", "true");
    await expect(rotate).toBeFocused();
    await expect(page.getByLabel("Rotation")).toBeVisible();
    await expect(page.getByRole("img", { name: `Preview of ${fixtureName}` })).toBeVisible();
    await expect(page.getByText("1 committed edit", { exact: true })).toBeVisible();

    await rotate.press("ArrowLeft");
    await expect(tabs.getByRole("tab", { name: "Resize", exact: true })).toBeFocused();
    await tabs.getByRole("tab", { name: "Resize", exact: true }).press("ArrowLeft");
    await expect(tabs.getByRole("tab", { name: "Adjust", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("toolbar", { name: "Image Editor tools" }).getByRole("button", { name: "Edit Image" })).toHaveAttribute("aria-pressed", "true");
});

test("Redact in the Edit PDF toolbar marks areas in the editor alongside other edits", async ({ page }) => {
    await page.goto("/");
    await openFromHome(page, "Edit PDF");
    await chooseFixture(page, pdfFixturePath);
    const redact = page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Redact", exact: true });

    await drawSceneMark(page, "Text");
    await expect(redact).toBeEnabled();
    await redact.click();
    await expect(redact).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("Drag over anything you want removed. Marks are only previews until you export.")).toBeVisible();
    await expect(page.getByRole("region", { name: "Redaction area" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Add Redact PDF to favorites" })).toHaveCount(0);
});

test("search and favorites list folded tools by their own names and open them", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });
    await expect(page.getByRole("button", { name: "Open Watermark PDF", exact: true })).toHaveCount(0);
    for (const [title, id] of [["Merge PDF", "pdf-merge"], ["Split PDF", "pdf-split"], ["Crop and Resize PDF", "pdf-crop"], ["Watermark PDF", "pdf-watermark"], ["Rotate and Flip Images", "rotate"]] as const) {
        await search.fill(title);
        await expect(page.locator(`[data-command-result="${id}"]`)).toHaveAccessibleName(`Open ${title}`);
    }
    await page.locator('[data-command-result="rotate"]').click();
    await expect(page.getByRole("tablist", { name: "Edit Image" }).getByRole("tab", { name: "Rotate", exact: true })).toHaveAttribute("aria-selected", "true");

    await page.getByRole("button", { name: "Add Rotate and Flip Images to favorites" }).click();
    await page.getByRole("button", { name: "← All tools" }).click();
    await openToolBySearch(page, "Watermark PDF");
    await page.getByRole("button", { name: "Add Watermark PDF to favorites" }).click();
    await page.getByRole("button", { name: "← All tools" }).click();
    await expect(page.getByRole("group", { name: "Favorite tools" }).getByRole("button")).toHaveText(["Rotate and Flip Images", "Watermark PDF"]);
    await page.locator('nav[aria-label="Workspace navigation"]').getByRole("button", { name: "Favorites" }).click();
    await expect(page.locator("button[data-command-result]")).toHaveText(["Watermark PDF", "Rotate and Flip Images"]);
    await page.getByRole("button", { name: "Open Watermark PDF", exact: true }).click();
    await expect(page.getByRole("button", { name: "Remove Watermark PDF from favorites" })).toBeVisible();
});
