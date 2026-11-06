import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Invocation = { command: string; args: unknown };
type TestWindow = Window & { __invocations: Invocation[]; __dialogResults: string[][]; __outcome?: (inputPath: string) => Record<string, unknown> };

const outputLocation = { customFolder: "/fake/shell/output" };

test.beforeEach(async ({ page }) => {
    await page.addInitScript((folder) => {
        const testWindow = window as TestWindow;
        testWindow.__invocations = [];
        testWindow.__dialogResults = [];
        const preview = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'/%3E";
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                testWindow.__invocations.push({ command, args });
                if (command === "get_quick_actions") return [];
                if (command === "get_output_settings") return { mode: "customFolder", folder };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:dialog|open") return testWindow.__dialogResults.shift() ?? null;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "inspect_pdf" || command === "inspect_pdf_scene") {
                    const path = (args as { request: { path: string } }).request.path;
                    if (path.includes("locked")) throw "This PDF needs a password.";
                    const pageCount = path.endsWith(".three.pdf") ? 3 : Number(/-(\d+)p\.pdf$/.exec(path)?.[1] ?? 1);
                    return { pages: Array.from({ length: pageCount }, (_, index) => ({ index, x: 0, y: 0, width: 612, height: 792, preview: command === "inspect_pdf" ? preview : null })) };
                }
                if (command === "inspect_image_preview") {
                    const tall = (args as { request: { path: string } }).request.path.includes("tall");
                    return tall ? { width: 20, height: 30, dataUrl: preview } : { width: 40, height: 20, dataUrl: preview };
                }
                if (command.startsWith("plugin:")) return null;
                const paths = (args as { request?: { paths?: string[] } }).request?.paths ?? [];
                return paths.map((inputPath) => ({ inputPath, outputPaths: [`${inputPath}.out`], detail: "Test output", state: "succeeded", failure: null, reason: null, warnings: [], ...testWindow.__outcome?.(inputPath) }));
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, outputLocation.customFolder);
});

const chooseFiles = async (page: Page, ...paths: string[]) => {
    await page.evaluate((paths) => { (window as TestWindow).__dialogResults.push(paths); }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    for (const path of paths) await expect(page.getByRole("option", { name: path.split("/").pop() })).toBeVisible();
};

const selectFromRail = (page: Page, rail: string, tool: string) =>
    page.getByRole("toolbar", { name: `${rail} tools` }).getByRole("button", { name: tool, exact: true }).click();

const exportAndCapture = async (page: Page, command: string, { save = false } = {}) => {
    await page.locator(".workspace-primary-action").click();
    if (save) {
        const dialog = page.getByRole("dialog");
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect(dialog).toHaveCount(0);
    }
    await expect.poll(() => requestsFor(page, command)).toHaveLength(1);
    const [args] = await requestsFor(page, command);
    await page.evaluate(() => { (window as TestWindow).__invocations.length = 0; });
    return args;
};

const requestsFor = (page: Page, command: string) => page.evaluate((command) =>
    (window as TestWindow).__invocations.filter((invocation) => invocation.command === command).map((invocation) => invocation.args), command);

const chooseOption = (page: Page, group: string, option: string) =>
    page.getByRole("radiogroup", { name: group }).getByRole("radio", { name: option, exact: true }).click();

const sent = (request: Record<string, unknown>) => ({ runId: expect.any(String), request });

test.describe("PDF Conversion tools send their exact requests", () => {
    test.beforeEach(async ({ page }) => {
        await page.goto("/");
        await openToolBySearch(page, "PDF to Images");
    });

    test("Images to PDF from the rail", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "Images to PDF");
        await chooseFiles(page, "/local/scan-1.png", "/local/scan-2.jpg");
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({
            paths: ["/local/scan-1.png", "/local/scan-2.jpg"],
            outputLocation,
            outputPath: "/fake/shell/output/scan-1-combined.pdf",
            pageSize: "matchImage",
        }));
    });

    test("Images to PDF sends margin, fit and orientation only for A4 and Letter", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "Images to PDF");
        await chooseFiles(page, "/local/wide.jpg", "/local/tall.png");
        const pageSize = page.getByRole("radiogroup", { name: "Page size" });
        const layout = (name: string) => page.getByRole("group", { name: `${name} layout` });
        const matchHint = page.getByText("Each page is the size of its image. Margin, fit and orientation appear for A4 and Letter.");
        const layoutControls = [
            page.getByRole("radiogroup", { name: "Orientation" }),
            page.getByRole("radiogroup", { name: "Fit" }),
            page.getByRole("radiogroup", { name: "Margin" }),
        ];
        const paths = ["/local/wide.jpg", "/local/tall.png"];
        const base = { paths, outputLocation, outputPath: "/fake/shell/output/wide-combined.pdf" };

        await expect(pageSize.getByRole("radio", { name: "Match image" })).toBeChecked();
        await expect(matchHint).toBeVisible();
        for (const control of layoutControls) await expect(control).toHaveCount(0);
        await expect(layout("A4")).toHaveCount(0);
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({ ...base, pageSize: "matchImage" }));

        await pageSize.getByRole("radio", { name: "A4" }).click();
        await expect(matchHint).toHaveCount(0);
        const a4 = layout("A4");
        await expect(a4).toBeVisible();
        await expect(a4.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Automatic" })).toBeChecked();
        await expect(a4.getByText("Automatic turns each page to suit its image.")).toBeVisible();
        await expect(a4.getByRole("radio", { name: "Fit within the page" })).toBeChecked();
        await expect(a4.getByRole("radio", { name: "Fill the page, trimming edges" })).not.toBeChecked();
        await expect(a4.getByRole("radiogroup", { name: "Margin" }).getByRole("radio", { name: "Small" })).toBeChecked();
        await expect(a4.getByText("Page 1, wide.jpg on A4 landscape with a small margin. Select another page to preview it.")).toBeVisible();
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({
            ...base, pageSize: "a4", margin: "small", fit: "fit", orientation: "automatic",
        }));

        await page.getByRole("option", { name: "tall.png" }).click();
        await expect(a4.getByText("Page 2, tall.png on A4 portrait with a small margin. Select another page to preview it.")).toBeVisible();

        await a4.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Landscape" }).click();
        await a4.getByRole("radio", { name: "Fill the page, trimming edges" }).click();
        await a4.getByRole("radiogroup", { name: "Margin" }).getByRole("radio", { name: "Large" }).click();
        await expect(a4.getByText("Page 2, tall.png on A4 landscape with a large margin. Select another page to preview it.")).toBeVisible();
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({
            ...base, pageSize: "a4", margin: "large", fit: "fill", orientation: "landscape",
        }));

        await pageSize.getByRole("radio", { name: "Letter" }).click();
        await expect(layout("A4")).toHaveCount(0);
        const letter = layout("Letter");
        await expect(letter).toBeVisible();
        await letter.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Portrait" }).click();
        await letter.getByRole("radiogroup", { name: "Margin" }).getByRole("radio", { name: "None" }).click();
        await expect(letter.getByText("Page 2, tall.png on Letter portrait with no margin. Select another page to preview it.")).toBeVisible();
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({
            ...base, pageSize: "letter", margin: "none", fit: "fill", orientation: "portrait",
        }));

        await pageSize.getByRole("radio", { name: "Match image" }).click();
        await expect(matchHint).toBeVisible();
        await expect(layout("Letter")).toHaveCount(0);
        for (const control of layoutControls) await expect(control).toHaveCount(0);
        const request = await exportAndCapture(page, "images_to_pdf", { save: true });
        expect(request).toEqual(sent({ ...base, pageSize: "matchImage" }));
        for (const key of ["margin", "fit", "orientation"]) expect((request as { request: object }).request).not.toHaveProperty(key);
    });

    test("PDF to Images from the rail", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf");
        await chooseOption(page, "Resolution", "300 dpi");
        await chooseOption(page, "Format", "PNG");
        await page.getByRole("checkbox", { name: "Page 2" }).click();
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 300, format: "png", pageRange: null,
            picks: [{ path: "/local/report.three.pdf", pages: [0, 2] }], outputLocation,
        }));

        await page.getByRole("button", { name: "Use the range instead" }).click();
        await chooseOption(page, "Pages", "Range");
        await page.getByRole("textbox", { name: "Page range for every PDF" }).fill("2-3");
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 300, format: "png", pageRange: "2-3", picks: [], outputLocation,
        }));
    });

    test("PDF to Images sends WebP and a custom resolution, and blocks one outside 72 to 300", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await expect(page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "PDF to Images", exact: true }))
            .toHaveAttribute("title", "Render PDF pages to JPEG, PNG or WebP images at 72 to 300 dpi.");
        await chooseFiles(page, "/local/report.three.pdf");
        const format = page.getByRole("radiogroup", { name: "Format" });
        await expect(format.getByRole("radio")).toHaveText(["JPEG", "PNG", "WebP"]);
        await expect(format.getByRole("radio", { name: "PNG" })).toBeChecked();
        await expect(page.getByRole("radiogroup", { name: "Resolution" }).getByRole("radio", { name: "150 dpi" })).toBeChecked();
        await expect(page.getByText("At 150 dpi a Letter page is 1275 × 1650 px. Use 300 for print.")).toBeVisible();
        await expect(page.getByTestId("run-destination-example")).toHaveText("report.three-page-01.png");
        await expect(page.getByRole("region", { name: "Destination" }))
            .toContainText("Page numbers are padded so names sort in page order. An existing name gets -1, -2 and so on.");
        await expect(page.getByRole("region", { name: "Destination" })).not.toContainText("Nothing is replaced.");

        await chooseOption(page, "Format", "WebP");
        await expect(page.getByText("Lossy at quality 90. Small files for the web.")).toBeVisible();
        await expect(page.getByTestId("run-destination-example")).toHaveText("report.three-page-01.webp");
        await chooseOption(page, "Resolution", "Custom");
        const custom = page.getByRole("spinbutton", { name: "Custom resolution in dpi, 72 to 300" });
        await expect(custom).toHaveValue("200");
        await expect(page.getByText("Any value from 72 to 300. At 200 dpi a Letter page is 1700 × 2200 px.")).toBeVisible();

        await custom.fill("301");
        await expect(page.getByRole("alert")).toHaveText("Enter a whole number from 72 to 300.");
        await expect(page.locator(".workspace-primary-action")).toBeDisabled();
        await custom.fill("72");
        await expect(page.getByRole("alert")).toHaveCount(0);
        await custom.fill("240");
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 240, format: "webp", pageRange: null, picks: [], outputLocation,
        }));

        await chooseOption(page, "Format", "JPEG");
        await chooseOption(page, "Resolution", "72 dpi");
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 72, format: "jpg", pageRange: null, picks: [], outputLocation,
        }));
    });

    test("PDF to Images lists every page image on a row with Open and Reveal", async ({ page }) => {
        await page.evaluate(() => {
            (window as TestWindow).__outcome = (inputPath) => ({
                outputPaths: [1, 2, 3].map((page) => `${inputPath.replace(/\.pdf$/, "")}-page-0${page}.png`),
                detail: "3 images, pages 1-3",
            });
        });
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf");
        await exportAndCapture(page, "pdf_to_images");

        const row = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ hasText: "report.three.pdf" });
        await expect(row).toContainText("3 images, pages 1-3");
        await expect(row).not.toContainText("3 files");
        const toggle = row.getByRole("button", { name: "Show 3 images" });
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await expect(row.getByRole("button", { name: /^Open/ })).toHaveCount(0);
        await toggle.click();
        await expect(row.getByRole("button", { name: "Hide images" })).toHaveAttribute("aria-expanded", "true");

        const images = row.getByRole("list", { name: "Images from report.three.pdf" });
        await expect(images.getByRole("listitem")).toHaveText([/report\.three-page-01\.png/, /report\.three-page-02\.png/, /report\.three-page-03\.png/]);
        for (const image of ["report.three-page-01.png", "report.three-page-02.png", "report.three-page-03.png"]) {
            const item = images.getByRole("listitem").filter({ hasText: image });
            await item.getByRole("button", { name: `Open ${image}` }).click();
            await item.getByRole("button", { name: new RegExp(`^Show in .+: ${image.replace(/\./g, "\\.")}$`) }).click();
        }
        const actions = await page.evaluate(() => (window as TestWindow).__invocations
            .filter(({ command }) => command === "open_output_path" || command === "reveal_output_path")
            .map(({ command, args }) => [command, (args as { path: string }).path]));
        expect(actions).toEqual([1, 2, 3].flatMap((index) => [
            ["open_output_path", `/local/report.three-page-0${index}.png`],
            ["reveal_output_path", `/local/report.three-page-0${index}.png`],
        ]));

        await row.getByRole("button", { name: "Hide images" }).click();
        await expect(images).toHaveCount(0);
    });

    test("PDF to Images marks a row Renamed when page images got numbered names", async ({ page }) => {
        await page.addInitScript(() => Object.defineProperty(navigator, "platform", { get: () => "Win32" }));
        await page.goto("/");
        await openToolBySearch(page, "PDF to Images");
        await page.evaluate(() => {
            const renamed = (page: string) => ({
                code: "renamedToAvoidReplacing",
                message: "",
                details: { chosenName: `survey-page-${page}.webp`, finalName: `survey-page-${page}-1.webp` },
            });
            (window as TestWindow).__outcome = (inputPath) => inputPath.endsWith("survey.pdf")
                ? { outputPaths: ["/local/survey-page-03-1.webp", "/local/survey-page-04-1.webp", "/local/survey-page-05.webp"], detail: "3 images, pages 3-5", warnings: [renamed("03"), renamed("04")] }
                : { outputPaths: ["/local/memo-page-01.webp", "/local/memo-page-02.webp"], detail: "2 images, pages 1 and 2" };
        });
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/survey.pdf", "/local/memo.pdf");
        await exportAndCapture(page, "pdf_to_images");

        const results = page.getByRole("list", { name: "Results" });
        const survey = results.getByRole("listitem").filter({ hasText: "survey.pdf" });
        await expect(survey).toHaveAttribute("data-tone", "warn");
        await expect(survey).toContainText("Renamed");
        await expect(survey).toContainText("3 images. 2 renamed, because survey-page-03.webp and -page-04.webp already existed. They end in -1.");
        const memo = results.getByRole("listitem").filter({ hasText: "memo.pdf" });
        await expect(memo).toHaveAttribute("data-tone", "ok");
        await expect(memo).toContainText("2 images, pages 1 and 2");

        await survey.getByRole("button", { name: "Show 3 images" }).click();
        const reveal = survey.getByRole("button", { name: "Show in File Explorer: survey-page-03-1.webp" });
        await expect(reveal).toHaveText("Show in Explorer");
        await reveal.click();
        await expect.poll(() => requestsFor(page, "reveal_output_path")).toEqual([{ path: "/local/survey-page-03-1.webp" }]);
    });

    test("PDF to Images sends one range for the batch and estimates the images per PDF", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/lease-14p.pdf", "/local/memo-4p.pdf", "/local/cover-2p.pdf", "/local/payslip-locked.pdf");
        const estimate = page.getByRole("status", { name: "Image estimate" });
        await expect(page.getByText("Every page of every PDF. Choose Range to limit them, or select a PDF to pick its pages.")).toBeVisible();
        await expect(estimate).toHaveText(/This will make 20 images\s*from 3 PDFs, 1 locked/);

        await chooseOption(page, "Pages", "Range");
        const range = page.getByRole("textbox", { name: "Page range for every PDF" });
        await expect(page.getByText("Commas and dashes, like 1-3, 5, 8-. 8- means page 8 to the end.")).toBeVisible();
        await expect(page.getByText("One range applies to every PDF. Pages past a PDF's last page are left out, and a PDF with no pages left is skipped.")).toBeVisible();
        await range.fill("3-5, 8-");
        await expect(page.getByText("Range 3-5, 8- applied to each PDF. 1 shortened, 1 will be skipped.")).toBeVisible();
        await expect(estimate).toHaveText(/This will make 12 images\s*from 2 PDFs, 1 skipped, 1 locked/);

        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/lease-14p.pdf", "/local/memo-4p.pdf", "/local/cover-2p.pdf", "/local/payslip-locked.pdf"],
            dpi: 150, format: "png", pageRange: "3-5, 8-", picks: [], outputLocation,
        }));
    });

    test("PDF to Images rejects a malformed range before sending anything", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/lease-14p.pdf", "/local/memo-4p.pdf");
        await chooseOption(page, "Pages", "Range");
        const range = page.getByRole("textbox", { name: "Page range for every PDF" });
        await range.fill("5-3, 8-");
        await expect(page.getByRole("alert")).toHaveText("5-3 runs backwards. Write it as 3-5.");
        await expect(range).toHaveAttribute("aria-invalid", "true");
        await expect(page.getByRole("status", { name: "Image estimate" })).toHaveText(/No image count yet\s*Fix the page range to convert/);
        await expect(page.locator(".workspace-primary-action")).toBeDisabled();
        await page.keyboard.press("ControlOrMeta+Enter");
        expect(await requestsFor(page, "pdf_to_images")).toEqual([]);

        await range.fill("3-5, 8-");
        await expect(page.getByRole("alert")).toHaveCount(0);
        await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    });

    test("PDF to Images picks replace the range for the selected PDF only", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf", "/local/memo-4p.pdf");
        await chooseOption(page, "Pages", "Range");
        await page.getByRole("textbox", { name: "Page range for every PDF" }).fill("2-");
        await expect(page.getByRole("checkbox", { name: "Page 1" })).not.toBeChecked();
        await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeChecked();
        await page.getByRole("checkbox", { name: "Page 1" }).click();
        await expect(page.getByText("3 of 3 pages picked: 1, 2 and 3. These picks replace the range for this PDF only.")).toBeVisible();
        await expect(page.getByText("report.three.pdf uses its own picks")).toBeVisible();
        await expect(page.getByRole("status", { name: "Image estimate" })).toHaveText(/This will make 6 images\s*from 2 PDFs/);

        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf", "/local/memo-4p.pdf"], dpi: 150, format: "png", pageRange: "2-",
            picks: [{ path: "/local/report.three.pdf", pages: [0, 1, 2] }], outputLocation,
        }));
    });

    test("PDF to Images shows the shortened-range note and a Skipped row", async ({ page }) => {
        await page.evaluate(() => {
            (window as TestWindow).__outcome = (inputPath) => inputPath.endsWith("cover-2p.pdf")
                ? { outputPaths: [], detail: "", state: "skipped", reason: { code: "rangeEmptyAfterClamping", message: "No pages left in 3-5, 8-. It has 2 pages, so nothing was made." } }
                : { outputPaths: ["/local/memo-4p-page-03.png", "/local/memo-4p-page-04.png"], detail: "2 images. Range shortened to pages 3 and 4, because memo-4p.pdf has 4 pages." };
        });
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/memo-4p.pdf", "/local/cover-2p.pdf");
        await chooseOption(page, "Pages", "Range");
        await page.getByRole("textbox", { name: "Page range for every PDF" }).fill("3-5, 8-");
        await exportAndCapture(page, "pdf_to_images");

        const results = page.getByRole("list", { name: "Results" });
        await expect(results.getByRole("listitem").filter({ hasText: "memo-4p.pdf" }))
            .toContainText("2 images. Range shortened to pages 3 and 4, because memo-4p.pdf has 4 pages.");
        const cover = results.getByRole("listitem").filter({ hasText: "cover-2p.pdf" });
        await expect(cover).toContainText("Skipped");
        await expect(cover).toContainText("No pages left in 3-5, 8-. It has 2 pages, so nothing was made.");
        await expect(cover.getByRole("button", { name: /^Open/ })).toHaveCount(0);
    });

    test("options survive a switch to another tool and back", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf");
        await chooseOption(page, "Resolution", "72 dpi");
        await chooseOption(page, "Format", "WebP");
        await selectFromRail(page, "PDF Conversion", "Images to PDF");
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await expect(page.getByRole("option", { name: "report.three.pdf" })).toBeVisible();
        await expect(page.getByRole("radiogroup", { name: "Resolution" }).getByRole("radio", { name: "72 dpi" })).toBeChecked();
        await expect(page.getByRole("radiogroup", { name: "Format" }).getByRole("radio", { name: "WebP" })).toBeChecked();
        await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeChecked();
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 72, format: "webp", pageRange: null, picks: [], outputLocation,
        }));
    });
});

test.describe("PDF Editor file tools send their exact requests", () => {
    test.beforeEach(async ({ page }) => {
        await page.goto("/");
        await openToolBySearch(page, "Repair PDF");
    });

    test("Repair PDF", async ({ page }) => {
        await chooseFiles(page, "/local/broken.pdf");
        expect(await exportAndCapture(page, "repair_pdf")).toEqual(sent({ paths: ["/local/broken.pdf"], outputLocation }));
    });

    test("Merge PDF", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Merge PDF");
        await chooseFiles(page, "/local/a.pdf", "/local/b.pdf");
        expect(await exportAndCapture(page, "merge_pdfs", { save: true })).toEqual(sent({
            paths: ["/local/a.pdf", "/local/b.pdf"], outputLocation, outputPath: "/fake/shell/output/a-merged.pdf",
        }));
    });

    test("Compare PDFs", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Compare PDFs");
        await chooseFiles(page, "/local/original.pdf", "/local/revised.pdf");
        expect(await exportAndCapture(page, "compare_pdfs", { save: true })).toEqual(sent({
            paths: ["/local/original.pdf", "/local/revised.pdf"], outputLocation, outputPath: "/fake/shell/output/original-comparison.txt",
        }));
    });

    test("Split PDF in every mode", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Split PDF");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByRole("textbox", { name: "Page ranges" }).fill("1-2, 3");
        expect(await exportAndCapture(page, "split_pdf")).toEqual(sent({
            paths: ["/local/report.three.pdf"], mode: { kind: "ranges", ranges: "1-2, 3" }, outputLocation,
        }));

        await page.getByRole("radiogroup", { name: "Split by" }).getByRole("radio", { name: "Every N pages" }).click();
        await page.getByRole("spinbutton", { name: "Pages per file" }).fill("3");
        expect(await exportAndCapture(page, "split_pdf")).toEqual(sent({
            paths: ["/local/report.three.pdf"], mode: { kind: "everyN", pages: 3 }, outputLocation,
        }));
    });

    test("Extract Pages", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Extract PDF Pages");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByRole("textbox", { name: "Page numbers or ranges" }).fill("1, 3");
        expect(await exportAndCapture(page, "extract_pdf_pages")).toEqual(sent({
            paths: ["/local/report.three.pdf"], pages: [0, 2], pageRanges: "1, 3", outputLocation,
        }));
    });

    test("Rotate PDFs", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Rotate PDFs");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByLabel("Rotation direction").selectOption("180");
        expect(await exportAndCapture(page, "rotate_pdf")).toEqual(sent({ paths: ["/local/report.three.pdf"], degrees: 180, pages: null, outputLocation }));

        await page.getByRole("checkbox", { name: "Page 2" }).click();
        expect(await exportAndCapture(page, "rotate_pdf")).toEqual(sent({ paths: ["/local/report.three.pdf"], degrees: 180, pages: [0, 2], outputLocation }));
    });

    test("Redact PDF", async ({ page }) => {
        await selectFromRail(page, "PDF Editor", "Redact PDF");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByRole("checkbox", { name: "Page 3" }).click();
        const area = page.getByRole("region", { name: "Redaction area" });
        await area.evaluate((element) => element.scrollIntoView({ block: "start" }));
        const bounds = (await area.boundingBox())!;
        await page.mouse.move(bounds.x + bounds.width * .25, bounds.y + bounds.height * .5);
        await page.mouse.down();
        await page.mouse.move(bounds.x + bounds.width * .75, bounds.y + bounds.height * .75, { steps: 5 });
        await page.mouse.up();

        const args = await exportAndCapture(page, "redact_pdf") as { request: { rectangle: Record<string, number> } };
        const { rectangle } = args.request;
        expect(args).toEqual(sent({
            paths: ["/local/report.three.pdf"],
            rectangle: { x: expect.any(Number), y: expect.any(Number), width: expect.any(Number), height: expect.any(Number) },
            scope: { selected: { pages: [0, 2] } },
            outputLocation,
        }));
        expect(rectangle.x).toBeCloseTo(153, -1);
        expect(rectangle.y).toBeCloseTo(198, -1);
        expect(rectangle.width).toBeCloseTo(306, -1);
        expect(rectangle.height).toBeCloseTo(198, -1);
    });
});
