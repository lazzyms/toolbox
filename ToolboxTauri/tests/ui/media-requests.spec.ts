import { expect, test, type Page } from "@playwright/test";
import type { OutputSettings } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";

// Pins the exact request each Media tool sends, so the per-tool panels can change without changing what runs.

type MediaWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
    __outputSettings: OutputSettings;
    __outcomesByPath?: Record<string, Record<string, unknown>>;
    __warnings: Record<string, Array<{ code: string; message: string }>>;
    __outcomes: Record<string, unknown[]>;
};

const customFolder: OutputSettings = { mode: "customFolder", folder: "/Users/test/Media out" };
const blankImage = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E";

const start = async (page: Page, settings: OutputSettings = { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" }) => {
    await page.addInitScript(({ settings, blankImage }) => {
        const win = window as unknown as MediaWindow;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__outputSettings = settings;
        win.__warnings = {};
        win.__outcomes = {};
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "get_quick_actions") return [];
                if (command === "get_output_settings") return win.__outputSettings;
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: blankImage };
                if (command === "inspect_tiff_pages") {
                    const { path } = (args as { request: { path: string } }).request;
                    return Array.from({ length: path.endsWith("two-page.tiff") ? 2 : 1 }, () => ({ width: 640, height: 480, dataUrl: blankImage }));
                }
                if (command.startsWith("plugin:")) return null;
                if (win.__outcomes[command]) return win.__outcomes[command];
                const { request } = (args ?? {}) as { request?: { paths?: string[] } };
                return (request?.paths ?? []).map((inputPath) => win.__outcomesByPath?.[inputPath] ?? ({
                    inputPath,
                    outputPaths: [`${inputPath}.out`],
                    detail: "",
                    state: "succeeded",
                    failure: null,
                    reason: null,
                    warnings: win.__warnings[inputPath] ?? [],
                }));
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, { settings, blankImage });
    await page.goto("/");
};

const choose = async (page: Page, files: string | string[]) => {
    await page.evaluate((files) => { (window as unknown as MediaWindow).__dialogResults.push(files); }, files);
    await page.getByRole("button", { name: "Choose files to process" }).click();
};

const invocationsOf = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as MediaWindow).__invocations.filter((invocation) => invocation.command === command).map(({ args }) => args),
command);

const runOnce = async (page: Page, command: string, save?: string) => {
    const before = (await invocationsOf(page, command)).length;
    const action = page.locator(".workspace-primary-action");
    await expect(action).toBeEnabled();
    await action.click();
    if (save !== undefined) {
        const dialog = page.getByRole("dialog");
        await expect(dialog.getByLabel("File name")).toHaveValue(save);
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect(dialog).toHaveCount(0);
    }
    await expect.poll(async () => (await invocationsOf(page, command)).length).toBe(before + 1);
    return (await invocationsOf(page, command))[before];
};

const selectRailTool = (page: Page, title: string) =>
    page.getByRole("toolbar", { name: /Media/ }).getByRole("button", { name: title, exact: true }).click();

const runId = expect.any(String);

test.describe("Media requests", () => {
    test("App Icon Set sends the preset and its sizes", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, "/local/art/logo.png");

        expect(await runOnce(page, "generate_icon_set")).toEqual({ runId, request: {
            paths: ["/local/art/logo.png"],
            preset: "macos",
            sizes: [16, 32, 128, 256, 512, 1024],
            outputLocation: "alongsideInput",
        } });

        await page.getByLabel("Icon preset").selectOption("ios");
        expect(await runOnce(page, "generate_icon_set")).toEqual({ runId, request: {
            paths: ["/local/art/logo.png"],
            preset: "ios",
            sizes: [20, 29, 40, 60, 76, 83, 1024],
            outputLocation: "alongsideInput",
        } });

        await page.getByLabel("Icon preset").selectOption("favicon");
        await page.getByRole("checkbox", { name: "Icon size 48px" }).click();
        await page.getByRole("checkbox", { name: "Icon size 16px" }).click();
        await page.getByRole("checkbox", { name: "Icon size 16px" }).click();
        expect(await runOnce(page, "generate_icon_set")).toEqual({ runId, request: {
            paths: ["/local/art/logo.png"],
            preset: "favicon",
            sizes: [16, 32, 64, 128, 256],
            outputLocation: "alongsideInput",
        } });
    });

    test("App Icon Set shows the bit-depth reduction warning on the HEIC result row", async ({ page }) => {
        await start(page);
        await page.evaluate(() => {
            (window as unknown as MediaWindow).__warnings = {
                "/local/art/widget-small.heic": [{ code: "precisionReduced", message: "10-bit HEIC saved as 8-bit sRGB." }],
            };
        });
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, "/local/art/widget-small.heic");
        await runOnce(page, "generate_icon_set");

        const row = page.locator(".result-row", { hasText: "widget-small" });
        await expect(row).toHaveAttribute("data-tone", "warn");
        await expect(row.locator(".result-row-detail").first()).toHaveText("10-bit HEIC saved as 8-bit sRGB.");
        await expect(row.locator(".result-row-state .sr-only")).toHaveText("Generated with a warning");
    });

    test("Create GIF sends frame order, delay, loop, and the saved path", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/frame-1.png", "/local/frames/frame-2.png"]);

        const strip = page.getByRole("region", { name: "Frame order" });
        await expect(strip.locator(".media-frame-list button")).toHaveCount(2);
        await expect(strip.locator(".media-frame-preview")).toHaveCount(2);
        expect(await invocationsOf(page, "inspect_image_preview")).toEqual([
            { request: { path: "/local/frames/frame-1.png" } },
            { request: { path: "/local/frames/frame-2.png" } },
        ]);

        expect(await runOnce(page, "create_gif", "frame-1-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/frame-1.png", "/local/frames/frame-2.png"],
            frameDelayMs: 100,
            loopForever: true,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/frame-1-animated.gif",
        } });

        await page.getByRole("button", { name: "Move frame-2.png up" }).click();
        await expect(strip.locator(".media-frame-list button").first()).toContainText("frame-2.png");
        await page.getByLabel("Frame delay in milliseconds").fill("250");
        await page.getByRole("switch", { name: "Loop animation" }).click();
        expect(await runOnce(page, "create_gif", "frame-2-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/frame-2.png", "/local/frames/frame-1.png"],
            frameDelayMs: 250,
            loopForever: false,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/frame-2-animated.gif",
        } });
    });

    test("Create GIF proposes the first frame's name in the resolved folder, and cancelling sends nothing", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/IMG_2210.heic", "/local/frames/IMG_2211.png"]);

        await page.locator(".workspace-primary-action").click();
        const dialog = page.getByRole("dialog");
        await expect(dialog.getByLabel("File name")).toHaveValue("IMG_2210-animated.gif");
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Media out");
        await dialog.getByRole("button", { name: "Cancel" }).click();
        await expect(dialog).toHaveCount(0);
        expect(await invocationsOf(page, "create_gif")).toEqual([]);

        expect(await runOnce(page, "create_gif", "IMG_2210-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/IMG_2210.heic", "/local/frames/IMG_2211.png"],
            frameDelayMs: 100,
            loopForever: true,
            outputLocation: { customFolder: "/Users/test/Media out" },
            outputPath: "/Users/test/Media out/IMG_2210-animated.gif",
        } });
    });

    test("Create GIF shows the renamed GIF with both names", async ({ page }) => {
        await start(page, customFolder);
        await page.evaluate(() => {
            (window as unknown as MediaWindow).__outcomes = { create_gif: [{
                inputPath: "/local/frames/loop.png",
                outputPaths: ["/Users/test/Media out/loop-animated-1.gif"],
                detail: "GIF saved",
                state: "succeeded",
                failure: null,
                reason: null,
                warnings: [{ code: "renamedToAvoidReplacing", message: "native wording", details: { chosenName: "loop-animated.gif", finalName: "loop-animated-1.gif" } }],
            }] };
        });
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/loop.png", "/local/frames/loop-2.png"]);
        await runOnce(page, "create_gif", "loop-animated.gif");

        const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(rows).toHaveCount(1);
        await expect(rows.first()).toHaveAttribute("data-tone", "warn");
        await expect(rows.first().locator(".result-row-name")).toHaveText("loop-animated-1.gif");
        await expect(rows.first()).toContainText("Saved as loop-animated-1.gif. Renamed to avoid replacing an existing file.");
        await expect(rows.first()).not.toContainText("native wording");
    });

    test("Create GIF names the unreadable frame and says nothing was saved", async ({ page }) => {
        await start(page);
        const message = "Frame 2, IMG_2212.heic, isn't a readable HEIC image. Replace or remove frame 2, then make the GIF again. Nothing was saved.";
        await page.evaluate((message) => {
            const failed = (inputPath: string, text: string) => ({ inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: text, pointer: null }, reason: null, warnings: [] });
            (window as unknown as MediaWindow).__outcomes = { create_gif: [
                failed("/local/frames/IMG_2211.png", "Aggregate operation stopped because /local/frames/IMG_2212.heic failed: " + message),
                failed("/local/frames/IMG_2212.heic", message),
            ] };
        }, message);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/IMG_2211.png", "/local/frames/IMG_2212.heic"]);
        await runOnce(page, "create_gif", "IMG_2211-animated.gif");

        const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(rows).toHaveCount(2);
        const failedFrame = rows.filter({ has: page.locator(".result-row-name", { hasText: /^IMG_2212\.heic$/ }) });
        await expect(failedFrame).toHaveCount(1);
        await expect(failedFrame).toHaveAttribute("data-tone", "fail");
        await expect(failedFrame.getByText(message, { exact: true })).toHaveCount(1);
        await expect(page.getByRole("list", { name: "Results" })).not.toContainText(".gif");
    });

    test("Create GIF keeps its options when the rail switches tools and back", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/frame-1.png", "/local/frames/frame-2.png"]);
        await page.getByLabel("Frame delay in milliseconds").fill("40");
        await page.getByRole("switch", { name: "Loop animation" }).click();

        await selectRailTool(page, "Generate App Icons");
        await expect(page.getByLabel("Icon preset")).toBeVisible();
        await page.getByLabel("Icon preset").selectOption("android");
        await selectRailTool(page, "Create GIF");
        await expect(page.getByLabel("Frame delay in milliseconds")).toHaveValue("40");
        await expect(page.getByRole("switch", { name: "Loop animation" })).not.toBeChecked();

        expect(await runOnce(page, "create_gif", "frame-1-animated.gif")).toMatchObject({ request: { frameDelayMs: 40, loopForever: false } });
        await selectRailTool(page, "Generate App Icons");
        await expect(page.getByLabel("Icon preset")).toHaveValue("android");
    });

    test("Extract GIF Frames sends only the GIF paths and output location", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Extract GIF Frames");
        await choose(page, ["/local/gifs/wave.gif", "/local/gifs/spin.gif"]);
        await expect(page.getByRole("region", { name: "Frame order" })).toHaveCount(0);

        expect(await runOnce(page, "extract_gif_frames")).toEqual({ runId, request: {
            paths: ["/local/gifs/wave.gif", "/local/gifs/spin.gif"],
            outputLocation: { customFolder: "/Users/test/Media out" },
        } });
    });

    const tiffMode = (page: Page) => page.getByRole("radiogroup", { name: "TIFF Pages mode" });

    test("TIFF Pages in Combine mode sends the mode, every inspected page in the displayed order and the saved path", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Split and Combine TIFF");
        await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
        await choose(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);

        const inspector = page.getByRole("region", { name: "Frame order" });
        const pages = inspector.locator(".media-frame-list button");
        await expect(pages).toHaveCount(3);
        await expect(pages.nth(1)).toContainText("two-page.tiff · page 2");
        await expect(inspector.locator(".media-frame-preview")).toHaveCount(3);
        expect(await invocationsOf(page, "inspect_tiff_pages")).toEqual([
            { request: { path: "/local/tiff/two-page.tiff" } },
            { request: { path: "/local/tiff/one-page.tiff" } },
        ]);
        expect(await invocationsOf(page, "inspect_image_preview")).toEqual([]);

        await pages.nth(2).click();
        await page.getByRole("button", { name: "Move selected TIFF page up" }).click();
        await expect(pages.nth(1)).toHaveAttribute("aria-pressed", "true");
        await expect(pages.nth(1)).toContainText("one-page.tiff · page 1");

        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tiff")).toEqual({ runId, request: {
            mode: "combine",
            paths: ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"],
            pages: [
                { path: "/local/tiff/two-page.tiff", page: 0 },
                { path: "/local/tiff/one-page.tiff", page: 0 },
                { path: "/local/tiff/two-page.tiff", page: 1 },
            ],
            outputLocation: { customFolder: "/Users/test/Media out" },
            outputPath: "/Users/test/Media out/two-page-combined.tiff",
        } });
    });

    test("TIFF Pages in Combine mode combines one file through the save step", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Split and Combine TIFF");
        await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
        await choose(page, "/local/tiff/two-page.tiff");
        await expect(page.locator(".media-frame-list button")).toHaveCount(2);
        await expect(page.locator(".workspace-primary-action")).toContainText("Make TIFF…");

        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tiff")).toEqual({ runId, request: {
            mode: "combine",
            paths: ["/local/tiff/two-page.tiff"],
            pages: [
                { path: "/local/tiff/two-page.tiff", page: 0 },
                { path: "/local/tiff/two-page.tiff", page: 1 },
            ],
            outputLocation: "alongsideInput",
            outputPath: "/local/tiff/two-page-combined.tiff",
        } });
    });

    test("TIFF Pages splits several files as a batch in the chosen Split mode without a save step", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Split and Combine TIFF");
        await expect(tiffMode(page).getByRole("radio", { name: "Split pages" })).toBeChecked();
        await choose(page, ["/local/tiff/two-page.tiff", "/local/tiff/scan-0043.tiff"]);
        await expect(page.getByRole("region", { name: "Frame order" })).toHaveCount(0);
        await expect(page.locator(".workspace-primary-action")).toContainText("Split 2 TIFFs");
        await expect(page.getByTestId("run-destination-example")).toContainText("two-page-page-01.tif");

        expect(await runOnce(page, "process_tiff_pages")).toEqual({ runId, request: {
            mode: "split",
            paths: ["/local/tiff/two-page.tiff", "/local/tiff/scan-0043.tiff"],
            outputLocation: { customFolder: "/Users/test/Media out" },
        } });
        await expect(page.getByRole("dialog")).toHaveCount(0);
    });

    test("TIFF Pages Split shows a skipped one-page TIFF apart from a failed one, with its reason", async ({ page }) => {
        await start(page);
        await page.addInitScript(() => {
            const failed = (inputPath: string, message: string) => ({ inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message, pointer: null }, reason: null, warnings: [] });
            (window as unknown as MediaWindow).__outcomesByPath = {
                "/local/tiff/one-page.tiff": { inputPath: "/local/tiff/one-page.tiff", outputPaths: [], detail: "", state: "skipped", failure: null, reason: { code: "noEffectiveChange", message: "Only 1 page, so there's nothing to split. No file was made.", pointer: null }, warnings: [] },
                "/local/tiff/broken.tiff": failed("/local/tiff/broken.tiff", "Could not process /local/tiff/broken.tiff: Could not read TIFF"),
            };
        });
        await page.reload();
        await openToolBySearch(page, "Split and Combine TIFF");
        await choose(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff", "/local/tiff/broken.tiff"]);
        await runOnce(page, "process_tiff_pages");

        const skipped = page.locator('.result-row[data-state="skipped"]');
        await expect(skipped).toHaveCount(1);
        await expect(skipped).toHaveAttribute("data-tone", "skip");
        await expect(skipped).toContainText("one-page.tiff");
        await expect(skipped).toContainText("Skipped");
        await expect(skipped).toContainText("Only 1 page, so there's nothing to split. No file was made.");
        await expect(skipped.getByRole("button", { name: /Retry/ })).toHaveCount(0);

        const failed = page.locator('.result-row[data-state="failed"]');
        await expect(failed).toHaveCount(1);
        await expect(failed).toHaveAttribute("data-tone", "fail");
        await expect(failed).toContainText("broken.tiff");
        await expect(failed).toContainText("Failed");
        await expect(failed).toContainText("Could not read TIFF");
        await expect(failed.getByRole("button", { name: "Retry broken.tiff" })).toBeVisible();

        await expect(page.locator('.result-row[data-state="succeeded"]')).toContainText("two-page.tiff");
    });
});
