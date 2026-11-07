import { expect, test, type Page } from "@playwright/test";
import type { ImageMetadataInspection } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";

type InspectWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResult: string[];
    __inspections: Record<string, ImageMetadataInspection>;
    __inspectFailure?: string;
};

const camera = "/local/IMG_4012.jpg";
const clean = "/local/badge.png";
const broken = "/local/cut.heic";

const inspections: Record<string, ImageMetadataInspection> = {
    [camera]: { status: "report", path: camera, format: "jpeg", exif: true, gps: true, xmp: true, iptc: false, comments: false, icc: true, orientation: 6 },
    [broken]: { status: "error", path: broken, code: "truncated", message: "The HEIC file ends before its structure is complete." },
    [clean]: { status: "report", path: clean, format: "png", exif: false, gps: false, xmp: false, iptc: false, comments: false, icc: false, orientation: null },
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript((inspections) => {
        const win = window as unknown as InspectWindow;
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResult = [];
        win.__inspections = inspections;
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "plugin:event|listen") return nextId++;
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "plugin:dialog|open") return win.__dialogResult;
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "inspect_image_metadata") {
                    if (win.__inspectFailure) throw win.__inspectFailure;
                    return (args as { request: { paths: string[] } }).request.paths.map((path) => win.__inspections[path]);
                }
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                return [];
            },
            transformCallback: () => nextId++,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, inspections);
});

const openWithFiles = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, "Image Metadata");
    await page.evaluate((paths) => { (window as unknown as InspectWindow).__dialogResult = paths; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
    await page.getByRole("radio", { name: "Inspect metadata" }).click();
};

const rowsIn = (page: Page, name: string, present?: boolean) =>
    page.getByRole("region", { name: `Metadata in ${name}` }).locator(present === undefined ? ".image-metadata-row" : `.image-metadata-row[data-present=${present}]`);

test("inspection shows labelled categories per file, and an unreadable file shows only its own error", async ({ page }) => {
    await openWithFiles(page, [camera, broken, clean]);
    await page.getByRole("button", { name: "Inspect metadata", exact: true }).click();

    const requests = await page.evaluate(() => (window as unknown as InspectWindow).__invocations.filter(({ command }) => command === "inspect_image_metadata"));
    expect(requests).toHaveLength(1);
    expect(requests[0].args).toMatchObject({ request: { paths: [camera, broken, clean] } });

    const report = page.getByRole("region", { name: "Metadata found" });
    await expect(report.getByRole("region")).toHaveCount(3);

    await expect(rowsIn(page, "IMG_4012.jpg")).toHaveText([
        /^Camera\s*Camera details \(EXIF\)\s*Removed$/,
        /^Location\s*GPS location\s*Removed$/,
        /^XMP\s*XMP metadata\s*Removed$/,
        /^IPTC\s*None found\s*None found$/,
        /^Comments\s*None found\s*None found$/,
        /^Orientation\s*Rotate 90° clockwise to show upright\s*Kept$/,
        /^Colour profile\s*Embedded colour profile\s*Kept$/,
    ]);
    await expect(rowsIn(page, "IMG_4012.jpg", true).locator("dt")).toHaveText(["Camera", "Location", "XMP", "Orientation", "Colour profile"]);
    await expect(rowsIn(page, "IMG_4012.jpg", false).locator("dt")).toHaveText(["IPTC", "Comments"]);

    const brokenCard = page.getByRole("region", { name: "Metadata in cut.heic" });
    await expect(brokenCard.getByRole("alert")).toHaveText("Could not read this image's metadata. The HEIC file ends before its structure is complete.");
    await expect(brokenCard.locator(".image-metadata-row")).toHaveCount(0);

    await expect(rowsIn(page, "badge.png")).toHaveCount(7);
    await expect(rowsIn(page, "badge.png", true)).toHaveCount(0);
    await expect(rowsIn(page, "badge.png").getByText("None found", { exact: true })).toHaveCount(14);

    const text = await report.innerText();
    for (const raw of ["{", "}", '"status"', "exif", "true", "false", "null", "orientation"]) expect(text).not.toContain(raw);
    await expect(page.locator("pre")).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as InspectWindow).__invocations.some(({ command }) => command === "image_metadata"))).toBe(false);
});

test("a failed inspect call is shown as an error instead of a report", async ({ page }) => {
    await openWithFiles(page, [camera]);
    await page.evaluate(() => { (window as unknown as InspectWindow).__inspectFailure = "Inspection is unavailable."; });
    await page.getByRole("button", { name: "Inspect metadata", exact: true }).click();

    await expect(page.getByRole("alert").filter({ hasText: "Metadata could not be inspected." })).toHaveText("Metadata could not be inspected. Inspection is unavailable.");
    await expect(page.locator(".image-metadata-row")).toHaveCount(0);
});
