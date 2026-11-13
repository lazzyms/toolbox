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
                if (command === "inspect_image_preview") {
                    // Icon masters say their shape in their name, so the square preview has real dimensions to work with.
                    const { path } = (args as { request: { path: string } }).request;
                    if (path.includes("wide")) return { width: 1600, height: 900, dataUrl: blankImage };
                    if (path.includes("tall")) return { width: 300, height: 1400, dataUrl: blankImage };
                    if (path.includes("square")) return { width: 1024, height: 1024, dataUrl: blankImage };
                    if (path.includes("small")) return { width: 512, height: 512, dataUrl: blankImage };
                    if (path.includes("2mp")) return { width: 2000, height: 1000, dataUrl: blankImage };
                    return { width: 640, height: 480, dataUrl: blankImage };
                }
                if (command === "inspect_tiff_pages") {
                    const { path } = (args as { request: { path: string } }).request;
                    if (path.includes("unreadable")) throw "This TIFF can't be read.";
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
    test("App Icon Set sends the ticked platforms with only their ticked sizes and containers", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, "/local/art/logo.png");

        const platforms = page.getByRole("group", { name: "Make icons for" });
        for (const [name, checked, summary] of [
            ["macOS app", true, "7 sizes, ICNS, .appiconset"],
            ["iPhone and iPad", true, "13 sizes, .appiconset"],
            ["Android", false, "6 sizes"],
            ["Website", true, "6 sizes, favicon.ico"],
            ["Custom sizes", false, "None yet"],
        ] as const) {
            const box = platforms.getByRole("checkbox", { name, exact: true });
            await expect(box).toHaveAttribute("aria-checked", String(checked));
            await expect(box.locator("xpath=..")).toContainText(summary);
            await expect(platforms.getByRole("button", { name: `Sizes for ${name}` })).toBeVisible();
        }
        // macOS opens first, with each of its sizes a labelled checkbox.
        await expect(platforms.getByRole("button", { name: "Sizes for macOS app" })).toHaveAttribute("aria-expanded", "true");
        const sizes = platforms.getByRole("group", { name: "Sizes, pixels" });
        await expect(sizes.getByRole("checkbox")).toHaveCount(7);
        for (const size of [16, 32, 64, 128, 256, 512, 1024]) await expect(sizes.getByRole("checkbox", { name: String(size), exact: true })).toBeChecked();
        // Its containers are ticked, and the ICNS is named after the image.
        const macosContainers = platforms.getByRole("group", { name: "macOS app containers" });
        const appiconset = macosContainers.getByRole("checkbox", { name: "AppIcon.appiconset, with Contents.json" });
        const icns = macosContainers.getByRole("checkbox", { name: "logo.icns" });
        for (const box of [appiconset, icns]) {
            await expect(box).toBeChecked();
            await expect(box).toBeEnabled();
        }

        expect(await runOnce(page, "generate_icon_set")).toEqual({ runId, request: {
            paths: ["/local/art/logo.png"],
            platforms: [
                { preset: "macos", sizes: [16, 32, 64, 128, 256, 512, 1024], containers: ["appiconset", "icns"] },
                { preset: "ios", sizes: [20, 29, 40, 58, 60, 76, 80, 87, 120, 152, 167, 180, 1024], containers: ["appiconset"] },
                { preset: "favicon", sizes: [16, 32, 48, 64, 128, 256], containers: ["ico"] },
            ],
            squareRule: "pad",
            outputLocation: "alongsideInput",
        } });

        // Unticking a size removes exactly that size, and a partial macOS set has no asset catalog or ICNS.
        await sizes.getByRole("checkbox", { name: "64", exact: true }).click();
        await expect(platforms.getByRole("checkbox", { name: "macOS app" }).locator("xpath=..")).toContainText("6 sizes, PNGs only");
        await expect(platforms).toContainText("AppIcon.appiconset and the ICNS need every macOS size, so they're off. Tick 64 to turn them back on.");
        for (const box of [appiconset, icns]) {
            await expect(box).not.toBeChecked();
            await expect(box).toBeDisabled();
        }
        // iPhone and iPad icons are made opaque, and the design says so beside the asset catalog.
        await platforms.getByRole("button", { name: "Sizes for iPhone and iPad" }).click();
        await expect(platforms.getByRole("group", { name: "iPhone and iPad containers" })).toContainText("iPhone and iPad icons can't be transparent. Transparent areas are filled with white.");
        await platforms.getByRole("checkbox", { name: "iPhone and iPad" }).click();
        await platforms.getByRole("checkbox", { name: "Android" }).click();
        await platforms.getByRole("button", { name: "Sizes for Android" }).click();
        await expect(platforms.getByRole("button", { name: "Sizes for macOS app" })).toHaveAttribute("aria-expanded", "false");
        await expect(platforms).toContainText("Launcher sizes from mdpi to xxxhdpi, plus the 512 store icon.");
        await sizes.getByRole("checkbox", { name: "512", exact: true }).click();
        await platforms.getByRole("button", { name: "Sizes for Website" }).click();
        await sizes.getByRole("checkbox", { name: "48", exact: true }).click();
        // The favicon.ico holds whichever sizes are ticked, so it stays on.
        const ico = platforms.getByRole("group", { name: "Website containers" }).getByRole("checkbox", { name: "favicon.ico, with the ticked sizes up to 256" });
        await expect(ico).toBeChecked();
        await expect(ico).toBeEnabled();
        await expect(platforms.getByRole("checkbox", { name: "Website" }).locator("xpath=..")).toContainText("5 sizes, favicon.ico");
        expect(await runOnce(page, "generate_icon_set")).toEqual({ runId, request: {
            paths: ["/local/art/logo.png"],
            platforms: [
                { preset: "macos", sizes: [16, 32, 128, 256, 512, 1024], containers: [] },
                { preset: "android", sizes: [48, 72, 96, 144, 192], containers: [] },
                { preset: "favicon", sizes: [16, 32, 64, 128, 256], containers: ["ico"] },
            ],
            squareRule: "pad",
            outputLocation: "alongsideInput",
        } });

        await platforms.getByRole("button", { name: "Sizes for macOS app" }).click();
        await platforms.getByRole("button", { name: "Tick all macOS sizes" }).click();
        await expect(platforms.getByRole("checkbox", { name: "macOS app" }).locator("xpath=..")).toContainText("7 sizes, ICNS, .appiconset");
        await expect(platforms.getByRole("button", { name: "Tick all macOS sizes" })).toHaveCount(0);
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ request: { platforms: [
            { preset: "macos", sizes: [16, 32, 64, 128, 256, 512, 1024], containers: ["appiconset", "icns"] },
            { preset: "android", sizes: [48, 72, 96, 144, 192] },
            { preset: "favicon", sizes: [16, 32, 64, 128, 256] },
        ] } });

        // Unticking a container leaves its PNGs, and the summary says what is left.
        await appiconset.click();
        await expect(platforms.getByRole("checkbox", { name: "macOS app" }).locator("xpath=..")).toContainText("7 sizes, ICNS");
        await icns.click();
        await expect(platforms.getByRole("checkbox", { name: "macOS app" }).locator("xpath=..")).toContainText("7 sizes, PNGs only");
        await platforms.getByRole("button", { name: "Sizes for Website" }).click();
        await ico.click();
        await expect(platforms.getByRole("checkbox", { name: "Website" }).locator("xpath=..")).toContainText(/5 sizes$/);
        await platforms.getByRole("button", { name: "Sizes for macOS app" }).click();
        await icns.click();
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ request: { platforms: [
            { preset: "macos", containers: ["icns"] },
            { preset: "android", containers: [] },
            { preset: "favicon", containers: [] },
        ] } });
    });

    test("App Icon Set asks for every iPhone and iPad size before making the asset catalog", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, "/local/art/logo.png");
        const platforms = page.getByRole("group", { name: "Make icons for" });
        await platforms.getByRole("button", { name: "Sizes for iPhone and iPad" }).click();
        await platforms.getByRole("group", { name: "Sizes, pixels" }).getByRole("checkbox", { name: "87", exact: true }).click();
        const appiconset = platforms.getByRole("group", { name: "iPhone and iPad containers" }).getByRole("checkbox", { name: "AppIcon.appiconset, with Contents.json" });
        await expect(appiconset).toBeDisabled();
        await expect(platforms.getByRole("checkbox", { name: "iPhone and iPad" }).locator("xpath=..")).toContainText("12 sizes, PNGs only");
        await expect(platforms).toContainText("AppIcon.appiconset needs every iPhone and iPad size, so it's off. Tick 87 to turn it back on.");
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ request: { platforms: [
            { preset: "macos" }, { preset: "ios", containers: [] }, { preset: "favicon" },
        ] } });
        await platforms.getByRole("button", { name: "Tick all iPhone and iPad sizes" }).click();
        await expect(appiconset).toBeChecked();
        await expect(platforms.getByRole("checkbox", { name: "iPhone and iPad" }).locator("xpath=..")).toContainText("13 sizes, .appiconset");
    });

    test("App Icon Set takes custom sizes from 1 to 4096 and blocks the run until the choice is complete", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, "/local/art/logo.png");
        const platforms = page.getByRole("group", { name: "Make icons for" });
        const action = page.locator(".workspace-primary-action");

        await platforms.getByRole("checkbox", { name: "Custom sizes" }).click();
        await expect(platforms.getByRole("alert")).toHaveText("Tick at least one size for Custom sizes, or untick it.");
        await expect(action).toBeDisabled();

        await platforms.getByRole("button", { name: "Sizes for Custom sizes" }).click();
        const field = platforms.getByRole("textbox", { name: "Add a size in pixels" });
        await expect(field).toHaveAttribute("placeholder", "1 to 4096");
        const add = platforms.getByRole("button", { name: "Add size" });
        for (const [typed, message] of [
            ["5000", "5000 pixels is outside the range. Use 1 to 4096 pixels."],
            ["0", "0 pixels is outside the range. Use 1 to 4096 pixels."],
            ["12.5", "Type a whole number of pixels from 1 to 4096."],
            ["big", "Type a whole number of pixels from 1 to 4096."],
        ]) {
            await field.fill(typed);
            await add.click();
            await expect(platforms.getByRole("alert").first()).toHaveText(message);
            await expect(field).toHaveAttribute("aria-invalid", "true");
            await expect(field).toHaveAccessibleDescription(message);
            await expect(action).toBeDisabled();
        }
        await field.fill("300");
        await field.press("Enter");
        await field.fill("4096");
        await add.click();
        await field.fill("1");
        await add.click();
        await expect(field).toHaveValue("");
        await expect(field).not.toHaveAttribute("aria-invalid", "true");
        await field.fill("300");
        await add.click();
        await expect(platforms.getByRole("alert")).toHaveText("300 pixels is already in the list.");
        await field.fill("");
        await expect(platforms.getByRole("alert")).toHaveCount(0);
        await expect(field).not.toHaveAttribute("aria-invalid", "true");
        await expect(platforms.getByRole("checkbox", { name: "Custom sizes" }).locator("xpath=..")).toContainText("3 sizes");
        await expect(platforms.getByRole("list", { name: "Custom sizes added" }).getByRole("listitem")).toHaveText(["1", "300", "4096"]);
        await platforms.getByRole("button", { name: "Remove 4096" }).click();
        await expect(action).toBeEnabled();
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ request: { platforms: [
            { preset: "macos" }, { preset: "ios" }, { preset: "favicon" }, { preset: "custom", sizes: [1, 300] },
        ] } });

        // With no platform ticked, nothing can run.
        for (const name of ["macOS app", "iPhone and iPad", "Website", "Custom sizes"]) await platforms.getByRole("checkbox", { name }).click();
        await expect(platforms.getByRole("alert")).toHaveText("Tick at least one platform to make icons for.");
        await expect(action).toBeDisabled();
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

    test("App Icon Set takes several masters and sends them all in one run", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await expect(page.locator(".workspace-primary-action")).toHaveText(/^Make icon sets/);
        await choose(page, ["/local/art/app-icon.png", "/local/art/brand-mark.png"]);
        await choose(page, "/local/art/launcher.webp");

        await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(3);
        await expect(page.locator(".workspace-primary-action")).toHaveText(/^Make 3 icon sets/);
        await expect(page.getByTestId("run-destination-example")).toHaveText("app-icon-icons");
        await expect(page.getByText("Each image's icons are in one folder, with a folder per platform inside. A set is saved only when every size worked, so you never get half a set.")).toBeVisible();
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ runId, request: {
            paths: ["/local/art/app-icon.png", "/local/art/brand-mark.png", "/local/art/launcher.webp"],
            squareRule: "pad",
            outputLocation: "alongsideInput",
        } });
    });

    test("App Icon Set previews the square rule on the focused master and sends the rule", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, ["/local/art/brand-mark-wide.png", "/local/art/poster-tall.png", "/local/art/app-icon-square.png"]);

        const preview = page.getByRole("region", { name: "Preview" });
        const rule = page.getByRole("radiogroup", { name: "Non-square images" });
        const pad = rule.getByRole("radio", { name: "Pad with transparency" });
        const crop = rule.getByRole("radio", { name: "Crop to centre" });
        const square = preview.getByRole("img", { name: /^brand-mark-wide\.png (padded|cropped)/ });
        await expect(preview).toContainText("brand-mark-wide.png");
        await expect(pad).toHaveAttribute("aria-checked", "true");
        await expect(square).toHaveAccessibleName("brand-mark-wide.png padded to a transparent 1600-pixel square");
        await expect(square).toHaveAttribute("data-square-rule", "pad");
        await expect(square.locator("img")).toHaveCSS("object-fit", "contain");
        await expect(rule).toHaveAccessibleDescription("The whole image is centred on a transparent square of 1600 pixels. Applies to every non-square image in this run. Images are never stretched.");
        await expect(preview.getByRole("status")).toHaveCount(0);
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ request: { squareRule: "pad" } });

        await crop.click();
        await expect(crop).toHaveAttribute("aria-checked", "true");
        await expect(pad).toHaveAttribute("aria-checked", "false");
        await expect(square).toHaveAccessibleName("brand-mark-wide.png cropped to its centred 900-pixel square");
        await expect(square).toHaveAttribute("data-square-rule", "crop");
        await expect(square.locator("img")).toHaveCSS("object-fit", "cover");
        await expect(rule).toHaveAccessibleDescription("The centred 900-pixel square is kept. The left and right edges are cut off. Applies to every non-square image in this run. Images are never stretched.");
        await expect(preview.getByRole("status")).toHaveText("The crop is 900 pixels, so the 1024 size will be upscaled.");
        expect(await runOnce(page, "generate_icon_set")).toMatchObject({ runId, request: {
            paths: ["/local/art/brand-mark-wide.png", "/local/art/poster-tall.png", "/local/art/app-icon-square.png"],
            squareRule: "crop",
            outputLocation: "alongsideInput",
        } });

        // Focusing another master previews the same rule on it.
        await page.getByRole("listbox").getByRole("option", { name: /poster-tall\.png/ }).click();
        const tall = preview.getByRole("img", { name: /^poster-tall\.png (padded|cropped)/ });
        await expect(tall).toHaveAccessibleName("poster-tall.png cropped to its centred 300-pixel square");
        await expect(rule).toHaveAccessibleDescription("The centred 300-pixel square is kept. The top and bottom edges are cut off. Applies to every non-square image in this run. Images are never stretched.");
        await expect(preview.getByRole("status")).toHaveText("The crop is 300 pixels, so sizes above 300 will be upscaled.");
        await pad.click();
        await expect(tall).toHaveAccessibleName("poster-tall.png padded to a transparent 1400-pixel square");
        await expect(tall.locator("img")).toHaveCSS("object-fit", "contain");
        await expect(preview.getByRole("status")).toHaveCount(0);

        // A square master is used as it is, whichever rule is chosen.
        await page.getByRole("listbox").getByRole("option", { name: /app-icon-square\.png/ }).click();
        const already = preview.getByRole("img", { name: /^app-icon-square\.png, already/ });
        await expect(already).toHaveAccessibleName("app-icon-square.png, already square");
        await crop.click();
        await expect(already).toHaveAccessibleName("app-icon-square.png, already square");
        await expect(rule).toHaveAccessibleDescription("app-icon-square.png is already square, so it is used as it is. The rule applies to every non-square image in this run. Images are never stretched.");
        await expect(preview.getByRole("status")).toHaveCount(0);

        expect(await invocationsOf(page, "inspect_image_preview")).toEqual(expect.arrayContaining([
            { request: { path: "/local/art/brand-mark-wide.png" } },
            { request: { path: "/local/art/poster-tall.png" } },
            { request: { path: "/local/art/app-icon-square.png" } },
        ]));
    });

    test("App Icon Set warns when a master is smaller than the largest size, and shows a contact sheet of the ticked sizes", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, ["/local/art/widget-small.png", "/local/art/brand-mark-wide.png"]);
        const preview = page.getByRole("region", { name: "Preview" });
        const platforms = page.getByRole("group", { name: "Make icons for" });

        // A square 512 px master is upscaled to 1024 whichever rule is chosen.
        await expect(preview.getByRole("status")).toHaveText("widget-small.png is 512 pixels, so the 1024 size will be upscaled.");
        await expect(preview).toContainText("Contact sheet, macOS sizes");
        await expect(preview).toContainText("Actual pixels up to 64. Larger sizes are shown at 96.");
        const sheet = preview.getByRole("img", { name: "widget-small.png at 16, 32, 64, 128, 256, 512 and 1024 pixels" });
        await expect(sheet.locator(".icon-contact-size")).toHaveText(["16", "32", "64", "128", "256", "512", "1024 ↑"]);
        await expect(sheet.locator('[data-size="32"] .icon-contact-image')).toHaveCSS("width", "32px");
        await expect(sheet.locator('[data-size="1024"] .icon-contact-image')).toHaveCSS("width", "96px");

        // The sheet follows the open platform, and only its ticked sizes.
        await platforms.getByRole("button", { name: "Sizes for Website" }).click();
        await platforms.getByRole("group", { name: "Sizes, pixels" }).getByRole("checkbox", { name: "48", exact: true }).click();
        await expect(preview).toContainText("Contact sheet, Website sizes");
        await expect(preview.getByRole("img", { name: "widget-small.png at 16, 32, 64, 128 and 256 pixels" })).toBeVisible();

        // Padding a wide master keeps every size within its 1600 px square; cropping it to 900 upscales 1024.
        await page.getByRole("listbox").getByRole("option", { name: /brand-mark-wide\.png/ }).click();
        await expect(preview.getByRole("status")).toHaveCount(0);
        await page.getByRole("radiogroup", { name: "Non-square images" }).getByRole("radio", { name: "Crop to centre" }).click();
        await expect(preview.getByRole("status")).toHaveText("The crop is 900 pixels, so the 1024 size will be upscaled.");
        await platforms.getByRole("checkbox", { name: "macOS app" }).click();
        await platforms.getByRole("checkbox", { name: "iPhone and iPad" }).click();
        await expect(preview.getByRole("status")).toHaveCount(0);
    });

    test("App Icon Set shows each master's own result, with its folder or the size that failed", async ({ page }) => {
        await start(page);
        await page.addInitScript(() => {
            (window as unknown as MediaWindow).__outcomes = { generate_icon_set: [
                { inputPath: "/local/art/app-icon.png", outputPaths: ["/local/art/app-icon-icons"], detail: "7 files", state: "succeeded", failure: null, reason: null, warnings: [] },
                { inputPath: "/local/art/brand-mark.png", outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: "The macOS 1024 size failed: the image data is damaged. Nothing was saved for this image.", pointer: null }, reason: null, warnings: [] },
                { inputPath: "/local/art/launcher.webp", outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: "Not a readable WebP image. The file may be damaged. Nothing was saved.", pointer: null }, reason: null, warnings: [] },
                { inputPath: "/local/art/badge.png", outputPaths: ["/local/art/badge-icons-1"], detail: "7 files", state: "succeeded", failure: null, reason: null, warnings: [{ code: "renamedToAvoidReplacing", message: "", details: { chosenName: "badge-icons", finalName: "badge-icons-1" } }] },
                { inputPath: "/local/art/widget-small.png", outputPaths: ["/local/art/widget-small-icons"], detail: "8 files", state: "succeeded", failure: null, reason: null, warnings: [{ code: "upscaled", message: "Upscaled to 1024 from 512 pixels.", details: { from: 512, to: 1024 } }] },
            ] };
        });
        await page.reload();
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, ["/local/art/app-icon.png", "/local/art/brand-mark.png", "/local/art/launcher.webp", "/local/art/badge.png", "/local/art/widget-small.png"]);
        await runOnce(page, "generate_icon_set");

        const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(results).toHaveCount(5);
        await expect(results.nth(0)).toContainText("app-icon-icons");
        await expect(results.nth(0)).toContainText("7 files");
        await expect(results.nth(1)).toContainText("The macOS 1024 size failed: the image data is damaged. Nothing was saved for this image.");
        await expect(results.nth(1)).toContainText("Failed");
        await expect(results.nth(1).getByRole("button", { name: /^Open|Show in Finder/ })).toHaveCount(0);
        await expect(results.nth(2)).toContainText("Not a readable WebP image. The file may be damaged. Nothing was saved.");
        await expect(results.nth(3)).toContainText("Saved as badge-icons-1.");
        // An upscaled master is still made, and its row says which size was enlarged from what.
        await expect(results.nth(4)).toContainText("Upscaled to 1024 from 512 pixels.");
        await expect(results.nth(4)).toContainText("8 files");
        await expect(results.nth(4)).not.toContainText("Failed");

        await results.nth(0).getByRole("button", { name: "Show in Finder: app-icon-icons" }).click();
        await expect.poll(() => invocationsOf(page, "reveal_output_path")).toEqual([{ path: "/local/art/app-icon-icons" }]);
        await results.nth(3).getByRole("button", { name: "Open badge-icons-1" }).click();
        await expect.poll(() => invocationsOf(page, "open_output_path")).toEqual([{ path: "/local/art/badge-icons-1" }]);
    });

    test("App Icon Set shows a set with its containers as one icon folder, or the container that failed", async ({ page }) => {
        await start(page);
        await page.addInitScript(() => {
            (window as unknown as MediaWindow).__outcomes = { generate_icon_set: [
                // 7 macOS sizes, AppIcon.appiconset (10 images and Contents.json) and the ICNS; 13 iOS sizes and their 19-file asset catalog; 6 Website sizes and favicon.ico.
                { inputPath: "/local/art/app-icon.png", outputPaths: ["/local/art/app-icon-icons"], detail: "58 files", state: "succeeded", failure: null, reason: null, warnings: [] },
                { inputPath: "/local/art/brand-mark.png", outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: "The macOS ICNS failed: the disk is full. Nothing was saved for this image.", pointer: null }, reason: null, warnings: [] },
            ] };
        });
        await page.reload();
        await openToolBySearch(page, "Generate App Icons");
        await choose(page, ["/local/art/app-icon.png", "/local/art/brand-mark.png"]);
        await runOnce(page, "generate_icon_set");

        const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(results).toHaveCount(2);
        await expect(results.nth(0)).toContainText("app-icon-icons");
        await expect(results.nth(0)).toContainText("58 files");
        await expect(results.nth(0)).not.toContainText("ICO container is unavailable");
        await expect(results.nth(0)).not.toContainText("Failed");
        await expect(results.nth(1)).toContainText("The macOS ICNS failed: the disk is full. Nothing was saved for this image.");
        await expect(results.nth(1).getByRole("button", { name: /^Open|Show in Finder/ })).toHaveCount(0);
        await results.nth(0).getByRole("button", { name: "Show in Finder: app-icon-icons" }).click();
        await expect.poll(() => invocationsOf(page, "reveal_output_path")).toEqual([{ path: "/local/art/app-icon-icons" }]);
    });

    test("Create GIF sends frame order, delay, loop, and the saved path", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/frame-1.png", "/local/frames/frame-2.png"]);

        const strip = page.getByRole("listbox", { name: "Frames, in play order" });
        await expect(strip.getByRole("option")).toHaveCount(2);
        await expect(strip.locator(".media-frame-picture img")).toHaveCount(2);
        expect(await invocationsOf(page, "inspect_image_preview")).toEqual([
            { request: { path: "/local/frames/frame-1.png" } },
            { request: { path: "/local/frames/frame-2.png" } },
        ]);

        expect(await runOnce(page, "create_gif", "frame-1-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/frame-1.png", "/local/frames/frame-2.png"],
            frameDelayMs: 100,
            frameDelayOverrides: [],
            loopForever: true,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/frame-1-animated.gif",
        } });

        await page.getByRole("button", { name: "Move frame-2.png up" }).click();
        await expect(strip.getByRole("option").first()).toHaveAccessibleName("Frame 1, frame-2.png, 100 ms");
        await page.getByLabel("Delay for every frame").fill("250");
        await page.getByRole("switch", { name: "Loop forever" }).click();
        expect(await runOnce(page, "create_gif", "frame-2-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/frame-2.png", "/local/frames/frame-1.png"],
            frameDelayMs: 250,
            frameDelayOverrides: [],
            loopForever: false,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/frame-2-animated.gif",
        } });
    });

    test("Create GIF frame grid reorders by drag and keyboard, and own delays move with their frames", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/a.png", "/local/frames/b.png", "/local/frames/c.png"]);

        const grid = page.getByRole("listbox", { name: "Frames, in play order" });
        const frames = grid.getByRole("option");
        const status = page.getByRole("status").filter({ hasText: /^(Frame|Dragging) / });
        const names = () => frames.evaluateAll((options) => options.map((option) => option.getAttribute("aria-label")));
        await expect(frames).toHaveCount(3);
        await expect(frames.nth(0)).toHaveAccessibleName("Frame 1, a.png, 100 ms");
        await expect(frames.nth(0)).toContainText("Sets size");
        await expect(page.getByText("Frames play in this order. Drag a frame, or select it and press ⌘+Arrow to move it. Delete removes it.")).toBeVisible();
        await expect(page.getByText("300 ms per loop")).toHaveCount(0);
        await expect(page.getByText("0.3 s per loop")).toBeVisible();

        // Frame 2 gets its own delay; the global delay changes every other frame.
        await grid.getByRole("button", { name: "Frame 2, b.png, 100 ms" }).click();
        await expect(status).toHaveText("Frame 2, b.png, selected.");
        const frameSection = page.getByRole("group", { name: "Frame 2, b.png" });
        await expect(frameSection).toContainText("Uses 100 ms, the delay for every frame.");
        await frameSection.getByRole("checkbox", { name: "Own delay" }).click();
        const ownDelay = page.getByLabel("Delay for frame 2, milliseconds");
        await expect(ownDelay).toBeFocused();
        await expect(ownDelay).toHaveValue("100");
        await ownDelay.fill("1500");
        await expect(frameSection).toContainText("Clear Own delay to use the delay for every frame again.");
        await page.getByLabel("Delay for every frame").fill("70");
        expect(await names()).toEqual(["Frame 1, a.png, 70 ms", "Frame 2, b.png, 1500 ms, own delay", "Frame 3, c.png, 70 ms"]);
        await expect(page.getByText("1.64 s per loop")).toBeVisible();
        await expect(status).toHaveText("Frame 2, b.png, selected. Own delay 1500 ms.");

        // Drag c.png onto the first tile: it lands at position 1 and b.png's own delay follows b.png.
        await grid.getByRole("button", { name: "Frame 3, c.png, 70 ms" }).hover();
        await page.mouse.down();
        await grid.getByRole("button", { name: "Frame 1, a.png, 70 ms" }).hover();
        await expect(status).toHaveText("Dragging c.png. Drop it at position 1.");
        await expect(frames.nth(2)).toHaveAttribute("data-dragging", "true");
        await expect(frames.nth(0)).toHaveAttribute("data-drop-target", "before");
        await page.mouse.up();
        expect(await names()).toEqual(["Frame 1, c.png, 70 ms", "Frame 2, a.png, 70 ms", "Frame 3, b.png, 1500 ms, own delay"]);
        await expect(status).toHaveText("Frame 1, c.png, selected.");
        expect(await runOnce(page, "create_gif", "c-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/c.png", "/local/frames/a.png", "/local/frames/b.png"],
            frameDelayMs: 70,
            frameDelayOverrides: [{ index: 2, delayMs: 1500 }],
            loopForever: true,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/c-animated.gif",
        } });

        // The keyboard moves the selected frame to the front, keeping focus on it.
        const moved = grid.getByRole("button", { name: "Frame 3, b.png, 1500 ms, own delay" });
        await moved.click();
        await page.keyboard.press("Meta+ArrowLeft");
        await page.keyboard.press("Meta+ArrowUp");
        expect(await names()).toEqual(["Frame 1, b.png, 1500 ms, own delay", "Frame 2, c.png, 70 ms", "Frame 3, a.png, 70 ms"]);
        await expect(grid.getByRole("button", { name: "Frame 1, b.png, 1500 ms, own delay" })).toBeFocused();
        await expect(status).toHaveText("Frame 1, b.png, selected. Own delay 1500 ms.");
        await page.keyboard.press("ArrowRight");
        await expect(grid.getByRole("button", { name: "Frame 2, c.png, 70 ms" })).toBeFocused();
        await expect(page.getByRole("group", { name: "Frame 2, c.png" })).toContainText("Uses 70 ms, the delay for every frame. Frame 1 has its own delay.");
        await page.getByRole("switch", { name: "Loop forever" }).click();
        expect(await runOnce(page, "create_gif", "b-animated.gif")).toEqual({ runId, request: {
            paths: ["/local/frames/b.png", "/local/frames/c.png", "/local/frames/a.png"],
            frameDelayMs: 70,
            frameDelayOverrides: [{ index: 0, delayMs: 1500 }],
            loopForever: false,
            outputLocation: "alongsideInput",
            outputPath: "/local/frames/b-animated.gif",
        } });

        // Delete removes the selected frame from the list, not from disk.
        await grid.getByRole("button", { name: "Frame 2, c.png, 70 ms" }).click();
        await page.keyboard.press("Delete");
        expect(await names()).toEqual(["Frame 1, b.png, 1500 ms, own delay", "Frame 2, a.png, 70 ms"]);
    });

    test("Create GIF gives one copy of a repeated frame its own delay", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/a.png", "/local/frames/b.png", "/local/frames/a.png"]);

        const grid = page.getByRole("listbox", { name: "Frames, in play order" });
        await grid.getByRole("button", { name: "Frame 3, a.png, 100 ms" }).click();
        await page.getByRole("checkbox", { name: "Own delay" }).click();
        await page.getByLabel("Delay for frame 3, milliseconds").fill("500");
        await expect(grid.getByRole("option")).toHaveText([/^1\s*100 ms/, /^2\s*100 ms/, /^3\s*500 ms/]);
        expect(await runOnce(page, "create_gif", "a-animated.gif")).toMatchObject({ request: {
            paths: ["/local/frames/a.png", "/local/frames/b.png", "/local/frames/a.png"],
            frameDelayMs: 100,
            frameDelayOverrides: [{ index: 2, delayMs: 500 }],
        } });
    });

    test("Create GIF refuses delays outside 1 to 60000 ms with a message and no run", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/a.png", "/local/frames/b.png"]);
        const action = page.locator(".workspace-primary-action");
        const everyFrame = page.getByLabel("Delay for every frame");
        await expect(page.getByText("ms, 1 to 60000")).toBeVisible();

        for (const value of ["0", "60001", "", "2.5"]) {
            await everyFrame.fill(value);
            await expect(page.getByRole("alert")).toHaveText("The delay for every frame must be a whole number from 1 to 60000 ms.");
            await expect(everyFrame).toHaveAttribute("aria-invalid", "true");
            await expect(action).toBeDisabled();
        }
        await everyFrame.fill("60000");
        await expect(page.getByRole("alert")).toHaveCount(0);
        await expect(everyFrame).toHaveAttribute("aria-invalid", "false");
        await expect(action).toBeEnabled();

        await page.getByRole("checkbox", { name: "Own delay" }).click();
        const ownDelay = page.getByLabel("Delay for frame 1, milliseconds");
        await expect(ownDelay).toHaveValue("60000");
        await ownDelay.fill("0");
        await expect(page.getByRole("alert")).toHaveText("Frame 1's own delay must be a whole number from 1 to 60000 ms.");
        await expect(ownDelay).toHaveAttribute("aria-invalid", "true");
        await expect(page.getByRole("option", { name: "Frame 1, a.png, Check delay, own delay" })).toBeVisible();
        await expect(page.getByText("s per loop")).toHaveCount(0);
        await expect(action).toBeDisabled();
        // The message stays while another frame is selected.
        await page.getByRole("button", { name: "Frame 2, b.png, 60000 ms" }).click();
        await expect(page.getByRole("alert")).toHaveText("Frame 1's own delay must be a whole number from 1 to 60000 ms.");
        await expect(action).toBeDisabled();
        await page.getByRole("button", { name: "Frame 1, a.png, Check delay, own delay" }).click();
        await ownDelay.fill("1");
        await expect(page.getByRole("alert")).toHaveCount(0);
        expect(await runOnce(page, "create_gif", "a-animated.gif")).toMatchObject({ request: { frameDelayMs: 60000, frameDelayOverrides: [{ index: 0, delayMs: 1 }] } });
    });

    test("Create GIF preview plays the current order with the delays the GIF stores, its loop, and follows a reorder", async ({ page }) => {
        await page.clock.install();
        // Reduce motion is on: the preview never starts by itself, and Play still plays it.
        await page.emulateMedia({ reducedMotion: "reduce" });
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, ["/local/frames/a.png", "/local/frames/b.png", "/local/frames/c.png"]);

        const grid = page.getByRole("listbox", { name: "Frames, in play order" });
        const canvas = page.getByRole("img", { name: /^Preview, / });
        const play = page.getByRole("button", { name: "Play", exact: true });
        const pause = page.getByRole("button", { name: "Pause", exact: true });
        const shows = (frame: number, name: string) => expect(canvas).toHaveAccessibleName(`Preview, Frame ${frame} of 3, ${name}`);
        await shows(1, "a.png");
        await expect(page.getByText("Frame 1 of 3", { exact: true })).toBeVisible();
        await expect(page.getByText("Shows the GIF at 640 × 480 with your delays and loop. It doesn't play on its own when Reduce motion is on.")).toBeVisible();

        // b.png gets its own 44 ms, which the GIF stores as 40 ms; selecting a frame shows it.
        await grid.getByRole("button", { name: "Frame 2, b.png, 100 ms" }).click();
        await page.getByRole("checkbox", { name: "Own delay" }).click();
        await page.getByLabel("Delay for frame 2, milliseconds").fill("44");
        await shows(2, "b.png");
        await grid.getByRole("button", { name: "Frame 1, a.png, 100 ms" }).click();
        await shows(1, "a.png");

        // From here time moves only when the test moves it.
        await page.clock.pauseAt(Date.now() + 60_000);
        await page.clock.runFor(5_000);
        await shows(1, "a.png");
        await play.click();
        await expect(pause).toBeVisible();
        await page.clock.runFor(99);
        await shows(1, "a.png");
        await page.clock.runFor(1);
        await shows(2, "b.png");
        await page.clock.runFor(39);
        await shows(2, "b.png");
        await page.clock.runFor(1);
        await shows(3, "c.png");
        await page.clock.runFor(100);
        await shows(1, "a.png");
        await pause.click();
        await expect(play).toBeVisible();
        await page.clock.runFor(5_000);
        await shows(1, "a.png");

        // Move c.png to the front: the preview shows it there and plays the new order, b.png keeping its 40 ms.
        await grid.getByRole("button", { name: "Frame 3, c.png, 100 ms" }).click();
        await page.keyboard.press("Meta+ArrowLeft");
        await page.keyboard.press("Meta+ArrowLeft");
        await expect(grid.getByRole("option").first()).toHaveAccessibleName("Frame 1, c.png, 100 ms");
        await shows(1, "c.png");
        await play.click();
        await page.clock.runFor(100);
        await shows(2, "a.png");
        await page.clock.runFor(100);
        await shows(3, "b.png");
        await page.clock.runFor(40);
        await shows(1, "c.png");
        await pause.click();

        // Played once, it stops on the last frame; Play from there starts again at frame 1.
        await page.getByRole("switch", { name: "Loop forever" }).click();
        await grid.getByRole("button", { name: "Frame 3, b.png, 44 ms, own delay" }).click();
        await shows(3, "b.png");
        await play.click();
        await shows(1, "c.png");
        await page.clock.runFor(200);
        await shows(3, "b.png");
        await page.clock.runFor(40);
        await expect(play).toBeVisible();
        await page.clock.runFor(5_000);
        await shows(3, "b.png");

        // The slider steps through the frames by keyboard.
        const slider = page.getByRole("slider", { name: "Preview frame" });
        await slider.focus();
        await page.keyboard.press("Home");
        await shows(1, "c.png");
        await page.keyboard.press("ArrowRight");
        await shows(2, "a.png");
        await expect(slider).toHaveAttribute("aria-valuenow", "2");
    });

    test("Create GIF warns before running when the estimate passes 10 MB, and the warning clears below it", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        // Frame 1 sets a 2000 × 1000 canvas. The estimate is one byte per canvas pixel per frame,
        // so six frames are 12,000,000 bytes and five are exactly 10,000,000, the threshold, which is not large.
        await choose(page, Array.from({ length: 6 }, (_, index) => `/local/frames/2mp-${index + 1}.png`));

        const grid = page.getByRole("listbox", { name: "Frames, in play order" });
        const banner = page.getByRole("status").filter({ hasText: /^This GIF will be about/ });
        const size = page.getByRole("group", { name: "Size" });
        await expect(size).toContainText("2000 × 1000, from frame 1");
        await expect(size).toContainText("Other frames are centred. Larger ones are scaled down to fit, never stretched. Padding is transparent.");
        await expect(banner).toHaveText("This GIF will be about 12 MB. Large GIFs are slow to open and share. Remove frames or add smaller images to reduce it.");
        await expect(size).toContainText("About 12 MB. That's large for a GIF.");

        await grid.getByRole("button", { name: "Frame 6, 2mp-6.png, 100 ms" }).click();
        await page.keyboard.press("Delete");
        await expect(grid.getByRole("option")).toHaveCount(5);
        await expect(banner).toHaveCount(0);
        await expect(size).toContainText("About 10 MB.");
        await expect(size).not.toContainText("large for a GIF");
        // The warning never blocks a run.
        await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    });

    test("Create GIF size follows frame 1, so a smaller first frame clears the warning", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Create GIF");
        await choose(page, [...Array.from({ length: 6 }, (_, index) => `/local/frames/2mp-${index + 1}.png`), "/local/frames/small.png"]);

        const grid = page.getByRole("listbox", { name: "Frames, in play order" });
        const banner = page.getByRole("status").filter({ hasText: /^This GIF will be about/ });
        const size = page.getByRole("group", { name: "Size" });
        await expect(banner).toContainText("This GIF will be about 14 MB.");
        await grid.getByRole("button", { name: "Frame 7, small.png, 100 ms" }).click();
        for (let step = 0; step < 6; step += 1) await page.keyboard.press("Meta+ArrowLeft");
        await expect(grid.getByRole("option").first()).toHaveAccessibleName("Frame 1, small.png, 100 ms");
        // 512 × 512 × 7 frames is 1,835,008 bytes.
        await expect(size).toContainText("512 × 512, from frame 1");
        await expect(size).toContainText("About 1.8 MB.");
        await expect(banner).toHaveCount(0);
        await expect(page.getByText("Shows the GIF at 512 × 512 with your delays and loop.", { exact: false })).toBeVisible();
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
            frameDelayOverrides: [],
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
        await page.getByLabel("Delay for every frame").fill("40");
        await page.getByRole("switch", { name: "Loop forever" }).click();

        await selectRailTool(page, "Generate App Icons");
        const android = page.getByRole("group", { name: "Make icons for" }).getByRole("checkbox", { name: "Android" });
        await expect(android).not.toBeChecked();
        await android.click();
        await selectRailTool(page, "Create GIF");
        await expect(page.getByLabel("Delay for every frame")).toHaveValue("40");
        await expect(page.getByRole("switch", { name: "Loop forever" })).not.toBeChecked();

        expect(await runOnce(page, "create_gif", "frame-1-animated.gif")).toMatchObject({ request: { frameDelayMs: 40, loopForever: false } });
        await selectRailTool(page, "Generate App Icons");
        await expect(android).toBeChecked();
    });

    test("Extract GIF Frames sends only the GIF paths and output location", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Extract GIF Frames");
        await choose(page, ["/local/gifs/wave.gif", "/local/gifs/spin.gif"]);
        await expect(page.getByRole("region", { name: "Frame order" })).toHaveCount(0);
        await expect(page.getByRole("listbox", { name: "Frames, in play order" })).toHaveCount(0);

        expect(await runOnce(page, "extract_gif_frames")).toEqual({ runId, request: {
            paths: ["/local/gifs/wave.gif", "/local/gifs/spin.gif"],
            outputLocation: { customFolder: "/Users/test/Media out" },
        } });
    });

    const tiffMode = (page: Page) => page.getByRole("radiogroup", { name: "TIFF Pages mode" });

    const combineTiff = async (page: Page, files: string[]) => {
        await openToolBySearch(page, "Split and Combine TIFF");
        await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
        await choose(page, files);
        return page.getByRole("listbox", { name: "Pages, in order" });
    };
    const pageNames = (grid: ReturnType<Page["getByRole"]>) =>
        grid.getByRole("option").evaluateAll((options) => options.map((option) => option.getAttribute("aria-label")));

    test("TIFF Combine shows every file's pages together and sends ordered page refs after drag and keyboard reorder", async ({ page }) => {
        await start(page, customFolder);
        const grid = await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);
        const region = page.getByRole("region", { name: "Pages" });
        const status = page.getByRole("status").filter({ hasText: /^(Position|Dragging) / });

        await expect(grid.getByRole("option")).toHaveCount(3);
        expect(await pageNames(grid)).toEqual([
            "Position 1, two-page.tiff page 1",
            "Position 2, two-page.tiff page 2",
            "Position 3, one-page.tiff page 1",
        ]);
        await expect(grid.locator("img")).toHaveCount(3);
        await expect(region).toContainText("3 of 3 pages, from 2 TIFFs");
        const sources = region.getByRole("list", { name: "Source files" }).getByRole("listitem");
        await expect(sources).toHaveText([/^A\s*two-page\.tiff\s*2 pages/, /^B\s*one-page\.tiff\s*1 page/]);
        await expect(grid.getByRole("option").nth(2)).toContainText("Bp1");
        await expect(page.getByText("Pages are joined in this order. Drag a page, or select it and press ⌘+Arrow to move it.", { exact: false })).toBeVisible();
        await expect(page.locator(".workspace-primary-action")).toContainText("Combine 3 pages…");
        expect(await invocationsOf(page, "inspect_tiff_pages")).toEqual([
            { request: { path: "/local/tiff/two-page.tiff" } },
            { request: { path: "/local/tiff/one-page.tiff" } },
        ]);
        expect(await invocationsOf(page, "inspect_image_preview")).toEqual([]);

        // Drag one-page.tiff's page onto the first tile: it lands at position 1.
        await grid.getByRole("button", { name: "Position 3, one-page.tiff page 1" }).hover();
        await page.mouse.down();
        await grid.getByRole("button", { name: "Position 1, two-page.tiff page 1" }).hover();
        await expect(status).toHaveText("Dragging one-page.tiff, page 1. Drop it at position 1.");
        await expect(grid.getByRole("option").nth(0)).toHaveAttribute("data-drop-target", "before");
        await page.mouse.up();
        expect(await pageNames(grid)).toEqual([
            "Position 1, one-page.tiff page 1",
            "Position 2, two-page.tiff page 1",
            "Position 3, two-page.tiff page 2",
        ]);
        await expect(status).toHaveText("Position 1 of 3 selected: one-page.tiff, page 1.");
        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tif")).toEqual({ runId, request: {
            mode: "combine",
            paths: ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"],
            pages: [
                { path: "/local/tiff/one-page.tiff", page: 0 },
                { path: "/local/tiff/two-page.tiff", page: 0 },
                { path: "/local/tiff/two-page.tiff", page: 1 },
            ],
            outputLocation: { customFolder: "/Users/test/Media out" },
            outputPath: "/Users/test/Media out/two-page-combined.tif",
            compression: "lzw",
        } });

        // The keyboard moves the selected page to the front, keeping focus on it.
        await grid.getByRole("button", { name: "Position 3, two-page.tiff page 2" }).click();
        await page.keyboard.press("Meta+ArrowLeft");
        await page.keyboard.press("Meta+ArrowUp");
        expect(await pageNames(grid)).toEqual([
            "Position 1, two-page.tiff page 2",
            "Position 2, one-page.tiff page 1",
            "Position 3, two-page.tiff page 1",
        ]);
        await expect(grid.getByRole("button", { name: "Position 1, two-page.tiff page 2" })).toBeFocused();
        await page.keyboard.press("ArrowRight");
        await expect(grid.getByRole("button", { name: "Position 2, one-page.tiff page 1" })).toBeFocused();
        await expect(status).toHaveText("Position 2 of 3 selected: one-page.tiff, page 1.");
        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tif")).toMatchObject({ request: { pages: [
            { path: "/local/tiff/two-page.tiff", page: 1 },
            { path: "/local/tiff/one-page.tiff", page: 0 },
            { path: "/local/tiff/two-page.tiff", page: 0 },
        ] } });
    });

    test("TIFF Combine leaves pages out without touching the files, and Reset pages brings them back", async ({ page }) => {
        await start(page);
        const grid = await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);
        const region = page.getByRole("region", { name: "Pages" });
        const status = page.getByRole("status").filter({ hasText: /^Position / });
        await expect(grid.getByRole("option")).toHaveCount(3);

        // Delete leaves the selected page out and selects the next one.
        await grid.getByRole("button", { name: "Position 1, two-page.tiff page 1" }).click();
        await page.keyboard.press("Delete");
        expect(await pageNames(grid)).toEqual(["Position 1, two-page.tiff page 2", "Position 2, one-page.tiff page 1"]);
        await expect(grid.getByRole("button", { name: "Position 1, two-page.tiff page 2" })).toBeFocused();
        await expect(status).toHaveText("Position 1 of 2 selected: two-page.tiff, page 2. Left out: two-page.tiff, page 1.");
        await expect(region).toContainText("2 of 3 pages, from 2 TIFFs");
        await expect(page.locator(".workspace-primary-action")).toContainText("Combine 2 pages…");

        // Leave page out does the same from the toolbar.
        await grid.getByRole("button", { name: "Position 2, one-page.tiff page 1" }).click();
        await region.getByRole("button", { name: "Leave page out" }).click();
        expect(await pageNames(grid)).toEqual(["Position 1, two-page.tiff page 2"]);
        await expect(page.locator(".workspace-primary-action")).toContainText("Combine 1 page…");
        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tif")).toEqual({ runId, request: {
            mode: "combine",
            paths: ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"],
            pages: [{ path: "/local/tiff/two-page.tiff", page: 1 }],
            outputLocation: "alongsideInput",
            outputPath: "/local/tiff/two-page-combined.tif",
            compression: "lzw",
        } });

        // With every page left out there is nothing to combine.
        await grid.getByRole("button", { name: "Position 1, two-page.tiff page 2" }).click();
        await page.keyboard.press("Delete");
        await expect(grid).toHaveCount(0);
        await expect(region).toContainText("Every page is left out. Reset pages brings them back.");
        await expect(page.locator(".workspace-primary-action")).toBeDisabled();

        await region.getByRole("button", { name: "Reset pages" }).click();
        expect(await pageNames(grid)).toEqual([
            "Position 1, two-page.tiff page 1",
            "Position 2, two-page.tiff page 2",
            "Position 3, one-page.tiff page 1",
        ]);
        await expect(region.getByRole("button", { name: "Reset pages" })).toBeDisabled();
    });

    test("TIFF Combine removes a source file's pages from its chip, and Add TIFFs adds more", async ({ page }) => {
        await start(page);
        const grid = await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);
        const region = page.getByRole("region", { name: "Pages" });
        await expect(grid.getByRole("option")).toHaveCount(3);

        await region.getByRole("button", { name: "Remove two-page.tiff" }).click();
        expect(await pageNames(grid)).toEqual(["Position 1, one-page.tiff page 1"]);
        await expect(region).toContainText("1 of 1 page, from 1 TIFF");

        await page.evaluate(() => { (window as unknown as MediaWindow).__dialogResults.push(["/local/tiff/scan-two-page.tiff"]); });
        await region.getByRole("button", { name: /^Add TIFFs/ }).click();
        expect(await pageNames(grid)).toEqual([
            "Position 1, one-page.tiff page 1",
            "Position 2, scan-two-page.tiff page 1",
            "Position 3, scan-two-page.tiff page 2",
        ]);
        expect(await runOnce(page, "process_tiff_pages", "one-page-combined.tif")).toMatchObject({ request: {
            paths: ["/local/tiff/one-page.tiff", "/local/tiff/scan-two-page.tiff"],
            pages: [
                { path: "/local/tiff/one-page.tiff", page: 0 },
                { path: "/local/tiff/scan-two-page.tiff", page: 0 },
                { path: "/local/tiff/scan-two-page.tiff", page: 1 },
            ],
        } });
    });

    test("TIFF Combine proposes the first file's name in the resolved folder, and cancelling sends nothing", async ({ page }) => {
        await start(page, customFolder);
        const grid = await combineTiff(page, ["/local/tiff/scan-0043.tiff", "/local/tiff/two-page.tiff"]);
        await expect(grid.getByRole("option")).toHaveCount(3);

        await page.locator(".workspace-primary-action").click();
        const dialog = page.getByRole("dialog");
        await expect(dialog.getByLabel("File name")).toHaveValue("scan-0043-combined.tif");
        await expect(dialog.getByTestId("save-folder")).toHaveText("~/Media out");
        await dialog.getByRole("button", { name: "Cancel" }).click();
        await expect(dialog).toHaveCount(0);
        expect(await invocationsOf(page, "process_tiff_pages")).toEqual([]);

        // A name already ending in .tiff keeps it; the save path goes in the request.
        await page.locator(".workspace-primary-action").click();
        await dialog.getByLabel("File name").fill("Statement.tiff");
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect.poll(async () => (await invocationsOf(page, "process_tiff_pages")).length).toBe(1);
        expect((await invocationsOf(page, "process_tiff_pages"))[0]).toMatchObject({ request: {
            outputLocation: { customFolder: "/Users/test/Media out" },
            outputPath: "/Users/test/Media out/Statement.tiff",
        } });
    });

    test("TIFF Combine with one file still opens the save step", async ({ page }) => {
        await start(page);
        const grid = await combineTiff(page, ["/local/tiff/two-page.tiff"]);
        await expect(grid.getByRole("option")).toHaveCount(2);
        await expect(page.locator(".workspace-primary-action")).toContainText("Combine 2 pages…");

        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tif")).toEqual({ runId, request: {
            mode: "combine",
            paths: ["/local/tiff/two-page.tiff"],
            pages: [
                { path: "/local/tiff/two-page.tiff", page: 0 },
                { path: "/local/tiff/two-page.tiff", page: 1 },
            ],
            outputLocation: "alongsideInput",
            outputPath: "/local/tiff/two-page-combined.tif",
            compression: "lzw",
        } });
    });

    test("TIFF Combine waits for a file it can't read, and says which one", async ({ page }) => {
        await start(page);
        const grid = await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/unreadable.tiff"]);
        const region = page.getByRole("region", { name: "Pages" });
        await expect(grid.getByRole("option")).toHaveCount(2);
        await expect(region.getByRole("alert")).toContainText("The pages of unreadable.tiff could not be read.");
        await expect(region.getByRole("list", { name: "Source files" }).getByRole("listitem").nth(1)).toContainText("Can't read");
        await expect(page.locator(".workspace-primary-action")).toBeDisabled();

        await region.getByRole("button", { name: "Remove unreadable.tiff" }).click();
        await expect(region.getByRole("alert")).toHaveCount(0);
        await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    });

    test("TIFF Combine shows the renamed TIFF with both names", async ({ page }) => {
        await start(page, customFolder);
        await page.evaluate(() => {
            (window as unknown as MediaWindow).__outcomes = { process_tiff_pages: [{
                inputPath: "/local/tiff/two-page.tiff",
                outputPaths: ["/Users/test/Media out/two-page-combined-1.tif"],
                detail: "TIFF saved",
                state: "succeeded",
                failure: null,
                reason: null,
                warnings: [{ code: "renamedToAvoidReplacing", message: "native wording", details: { chosenName: "two-page-combined.tif", finalName: "two-page-combined-1.tif" } }],
            }] };
        });
        await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);
        await runOnce(page, "process_tiff_pages", "two-page-combined.tif");

        const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(rows).toHaveCount(1);
        await expect(rows.first()).toHaveAttribute("data-tone", "warn");
        await expect(rows.first().locator(".result-row-name")).toHaveText("two-page-combined-1.tif");
        await expect(rows.first()).toContainText("Saved as two-page-combined-1.tif. Renamed to avoid replacing an existing file.");
    });

    test("TIFF Combine names the file and page it can't read and says nothing was saved", async ({ page }) => {
        await start(page);
        const message = "Could not process /local/tiff/one-page.tiff: TIFF page 1 of 1 can't be read. The TIFF uses a compression this app can't decode.";
        await page.evaluate((message) => {
            const failed = (inputPath: string, text: string) => ({ inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message: text, pointer: null }, reason: null, warnings: [] });
            (window as unknown as MediaWindow).__outcomes = { process_tiff_pages: [
                failed("/local/tiff/two-page.tiff", "Aggregate operation stopped because /local/tiff/one-page.tiff failed: " + message),
                failed("/local/tiff/one-page.tiff", message),
            ] };
        }, message);
        await combineTiff(page, ["/local/tiff/two-page.tiff", "/local/tiff/one-page.tiff"]);
        await runOnce(page, "process_tiff_pages", "two-page-combined.tif");

        const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
        await expect(rows).toHaveCount(2);
        const failedFile = rows.filter({ has: page.locator(".result-row-name", { hasText: /^one-page\.tiff$/ }) });
        await expect(failedFile).toHaveAttribute("data-tone", "fail");
        await expect(failedFile).toContainText("TIFF page 1 of 1 can't be read.");
        await expect(page.getByRole("list", { name: "Results" })).not.toContainText("combined");
    });

    test("TIFF Pages splits several files as a batch in the chosen Split mode without a save step", async ({ page }) => {
        await start(page, customFolder);
        await openToolBySearch(page, "Split and Combine TIFF");
        await expect(tiffMode(page).getByRole("radio", { name: "Split pages" })).toBeChecked();
        await choose(page, ["/local/tiff/two-page.tiff", "/local/tiff/scan-0043.tiff"]);
        await expect(page.getByRole("region", { name: "Pages" })).toHaveCount(0);
        await expect(page.locator(".workspace-primary-action")).toContainText("Split 2 TIFFs");
        await expect(page.getByTestId("run-destination-example")).toContainText("two-page-page-01.tif");

        expect(await runOnce(page, "process_tiff_pages")).toEqual({ runId, request: {
            mode: "split",
            paths: ["/local/tiff/two-page.tiff", "/local/tiff/scan-0043.tiff"],
            outputLocation: { customFolder: "/Users/test/Media out" },
            compression: "lzw",
        } });
        await expect(page.getByRole("dialog")).toHaveCount(0);
    });

    test("TIFF Pages takes fax and JPEG-compressed TIFFs in Split and Combine without a validation message", async ({ page }) => {
        const scans = ["/local/tiff/fax-g3.tif", "/local/tiff/fax-g4.tiff", "/local/tiff/scan-jpeg.tif"];
        await start(page, customFolder);
        await openToolBySearch(page, "Split and Combine TIFF");
        await choose(page, scans);
        await expect(page.getByRole("region", { name: "Not added" })).toHaveCount(0);
        await expect(page.locator(".file-selection-row")).toHaveCount(3);
        await expect(page.locator(".file-selection-row[data-flag]")).toHaveCount(0);
        await expect(page.locator(".workspace-primary-action")).toContainText("Split 3 TIFFs");
        expect(await runOnce(page, "process_tiff_pages")).toEqual({ runId, request: {
            mode: "split", paths: scans, outputLocation: { customFolder: "/Users/test/Media out" }, compression: "lzw",
        } });

        await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
        const grid = page.getByRole("listbox", { name: "Pages, in order" });
        expect(await pageNames(grid)).toEqual([
            "Position 1, fax-g3.tif page 1",
            "Position 2, fax-g4.tiff page 1",
            "Position 3, scan-jpeg.tif page 1",
        ]);
        await expect(page.getByRole("region", { name: "Pages" }).getByRole("alert")).toHaveCount(0);
        const previewed = (await invocationsOf(page, "inspect_tiff_pages")).map((args) => (args as { request: { path: string } }).request.path);
        expect([...new Set(previewed)]).toEqual(scans);
        expect(await runOnce(page, "process_tiff_pages", "fax-g3-combined.tif")).toEqual({ runId, request: {
            mode: "combine", paths: scans, pages: scans.map((path) => ({ path, page: 0 })),
            outputLocation: { customFolder: "/Users/test/Media out" }, outputPath: "/Users/test/Media out/fax-g3-combined.tif", compression: "lzw",
        } });
    });

    test("TIFF Pages sends the chosen output compression in Split and Combine", async ({ page }) => {
        await start(page);
        await openToolBySearch(page, "Split and Combine TIFF");
        const compression = page.getByLabel("Compression");
        await expect(compression).toHaveValue("lzw");
        await expect(page.getByText("The source compression isn't kept.", { exact: false })).toBeVisible();
        await choose(page, ["/local/tiff/two-page.tiff"]);
        await compression.selectOption({ label: "Deflate, lossless" });
        expect(await runOnce(page, "process_tiff_pages")).toMatchObject({ request: { mode: "split", compression: "deflate" } });

        await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
        await expect(compression).toHaveValue("deflate");
        await compression.selectOption({ label: "None, largest files" });
        await expect(page.getByRole("listbox", { name: "Pages, in order" }).getByRole("option")).toHaveCount(2);
        expect(await runOnce(page, "process_tiff_pages", "two-page-combined.tif")).toMatchObject({ request: { mode: "combine", compression: "none" } });
    });

    test("TIFF Pages Split shows an unsupported TIFF variant on its own row and splits the rest", async ({ page }) => {
        await start(page);
        const message = "Could not process /local/tiff/maps-1998.tiff: TIFF page 1 of 2 can't be read. Old-style JPEG compression isn't supported in TIFF pages.";
        await page.addInitScript((message) => {
            (window as unknown as MediaWindow).__outcomesByPath = {
                "/local/tiff/maps-1998.tiff": { inputPath: "/local/tiff/maps-1998.tiff", outputPaths: [], detail: "", state: "failed", failure: { kind: "processing", message, pointer: null }, reason: null, warnings: [] },
            };
        }, message);
        await page.reload();
        await openToolBySearch(page, "Split and Combine TIFF");
        await choose(page, ["/local/tiff/two-page.tiff", "/local/tiff/maps-1998.tiff"]);
        await runOnce(page, "process_tiff_pages");

        const failed = page.locator('.result-row[data-state="failed"]');
        await expect(failed).toHaveCount(1);
        await expect(failed).toHaveAttribute("data-tone", "fail");
        await expect(failed).toContainText("maps-1998.tiff");
        await expect(failed).toContainText("Old-style JPEG compression isn't supported in TIFF pages.");
        await expect(page.locator('.result-row[data-state="succeeded"]')).toContainText("two-page.tiff");
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
