import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Invocation = { command: string; args: unknown };
type TestWindow = Window & { __invocations: Invocation[]; __dialogResults: string[][] };

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
                if (command === "inspect_pdf") {
                    const pageCount = (args as { request: { path: string } }).request.path.endsWith(".three.pdf") ? 3 : 1;
                    return { pages: Array.from({ length: pageCount }, (_, index) => ({ index, x: 0, y: 0, width: 612, height: 792, preview })) };
                }
                if (command.startsWith("plugin:")) return null;
                const paths = (args as { request?: { paths?: string[] } }).request?.paths ?? [];
                return paths.map((inputPath) => ({ inputPath, outputPaths: [`${inputPath}.out`], detail: "Test output", state: "succeeded", failure: null, reason: null, warnings: [] }));
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

const sent = (request: Record<string, unknown>) => ({ runId: expect.any(String), request });

test.describe("PDF Conversion tools send their exact requests", () => {
    test.beforeEach(async ({ page }) => {
        await page.goto("/");
        await openToolBySearch(page, "PDF to Text");
    });

    test("Images to PDF from the rail", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "Images to PDF");
        await chooseFiles(page, "/local/scan-1.png", "/local/scan-2.jpg");
        expect(await exportAndCapture(page, "images_to_pdf", { save: true })).toEqual(sent({
            paths: ["/local/scan-1.png", "/local/scan-2.jpg"],
            outputLocation,
            outputPath: "/fake/shell/output/scan-1-combined.pdf",
        }));
    });

    test("PDF to Images from the rail", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByLabel("Render DPI").selectOption("300");
        await page.getByLabel("Output format").selectOption("png");
        await page.getByRole("checkbox", { name: "Page 2" }).click();
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 300, format: "png", pageRange: "1,3", pages: [0, 2], outputLocation,
        }));

        await page.getByRole("textbox", { name: "Page range" }).fill("2-3");
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 300, format: "png", pageRange: "2-3", pages: [1, 2], outputLocation,
        }));
    });

    test("PDF to Text", async ({ page }) => {
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByRole("checkbox", { name: "Page 1" }).click();
        expect(await exportAndCapture(page, "pdf_to_text")).toEqual(sent({ paths: ["/local/report.three.pdf"], pages: [1, 2], outputLocation }));
    });

    test("Extract Images from PDF", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "Extract Images from PDF");
        await chooseFiles(page, "/local/report.three.pdf");
        await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeChecked();
        expect(await exportAndCapture(page, "extract_pdf_images")).toEqual(sent({ paths: ["/local/report.three.pdf"], pages: [0, 1, 2], outputLocation }));
    });

    test("options survive a switch to another tool and back", async ({ page }) => {
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await chooseFiles(page, "/local/report.three.pdf");
        await page.getByLabel("Render DPI").selectOption("72");
        await selectFromRail(page, "PDF Conversion", "PDF to Text");
        await selectFromRail(page, "PDF Conversion", "PDF to Images");
        await expect(page.getByRole("option", { name: "report.three.pdf" })).toBeVisible();
        await expect(page.getByLabel("Render DPI")).toHaveValue("72");
        await expect(page.getByRole("checkbox", { name: "Page 3" })).toBeChecked();
        expect(await exportAndCapture(page, "pdf_to_images")).toEqual(sent({
            paths: ["/local/report.three.pdf"], dpi: 72, format: "jpg", pageRange: "1-3", pages: [0, 1, 2], outputLocation,
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
        expect(await exportAndCapture(page, "split_pdf")).toEqual(sent({
            paths: ["/local/report.three.pdf"], pages: [], pageRanges: null, splitMode: "pages", chunkSize: null, outputLocation,
        }));

        await page.getByLabel("Split mode").selectOption("ranges");
        await page.getByRole("textbox", { name: "Page ranges" }).fill("1-2, 3");
        expect(await exportAndCapture(page, "split_pdf")).toEqual(sent({
            paths: ["/local/report.three.pdf"], pages: [], pageRanges: "1-2, 3", splitMode: "ranges", chunkSize: null, outputLocation,
        }));

        await page.getByLabel("Split mode").selectOption("chunks");
        await page.getByRole("spinbutton", { name: "Pages per file" }).fill("3");
        expect(await exportAndCapture(page, "split_pdf")).toEqual(sent({
            paths: ["/local/report.three.pdf"], pages: [], pageRanges: null, splitMode: "chunks", chunkSize: 3, outputLocation,
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
