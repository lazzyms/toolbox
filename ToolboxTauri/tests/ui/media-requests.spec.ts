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
                    return { width: 640, height: 480, dataUrl: blankImage };
                }
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
            (window as unknown as MediaWindow).__results = { generate_icon_set: [
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
        const android = page.getByRole("group", { name: "Make icons for" }).getByRole("checkbox", { name: "Android" });
        await expect(android).not.toBeChecked();
        await android.click();
        await selectRailTool(page, "Create GIF");
        await expect(page.getByLabel("Frame delay in milliseconds")).toHaveValue("40");
        await expect(page.getByRole("switch", { name: "Loop animation" })).not.toBeChecked();

        expect(await runOnce(page, "create_gif", "frame-1-animated.gif")).toMatchObject({ request: { frameDelayMs: 40, loopForever: false } });
        await selectRailTool(page, "Generate App Icons");
        await expect(android).toBeChecked();
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
