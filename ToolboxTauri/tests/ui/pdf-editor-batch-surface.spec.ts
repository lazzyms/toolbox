import { expect, test, type Page } from "@playwright/test";

const svg = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/></svg>');
const pdfPath = "/local/contract.pdf";
const secondPdfPath = "/local/appendix.pdf";

type HarnessWindow = Window & { calls: Array<{ command: string; args: any }>; dialogNext: string[] | null };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg, pdfPath }) => {
    Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
    const w = window as unknown as HarnessWindow;
    w.calls = [];
    w.dialogNext = null;
    (window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === "plugin:dialog|open") {
          const next = w.dialogNext;
          w.dialogNext = null;
          return next ?? [pdfPath];
        }
        if (command === "plugin:path|resolve_directory") return "/Users/test";
        if (command === "check_output_folder") return "ready";
        if (command === "inspect_pdf_scene") return { path: args.request.path, pages: [{ index: 0, width: 612, height: 792, preview: svg, textRuns: [] }] };
        if (command === "preview_pdf_scene_pages") return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        if (command === "test_batch_tool") {
          return args.request.paths.map((inputPath: string) => ({ inputPath, outputPaths: [inputPath.replace(/\.pdf$/, "-tested.pdf")], state: "succeeded", detail: "Test batch output", failure: null, reason: null, warnings: [] }));
        }
        return null;
      },
    };
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg, pdfPath });
  await page.goto("/tests/ui/harness/pdf-editor-batch.html");
});

const rail = (page: Page) => page.getByRole("toolbar", { name: "PDF Editor tools" });
const fileRow = (page: Page, name: string) => page.locator(".tool-view, .command-center").getByText(name, { exact: true }).first();

test("a batch tool in the PDF Editor rail runs every file through the shared batch shell", async ({ page }) => {
  await expect(rail(page).getByRole("button", { name: "Test Batch Tool" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("heading", { name: "Test Batch Tool" })).toBeVisible();
  await expect(page.getByRole("group", { name: "PDF page canvas" })).toHaveCount(0);

  await page.evaluate(([first, second]) => { (window as unknown as HarnessWindow).dialogNext = [first, second]; }, [pdfPath, secondPdfPath]);
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await expect(fileRow(page, "contract.pdf")).toBeVisible();
  await expect(fileRow(page, "appendix.pdf")).toBeVisible();

  await page.getByRole("button", { name: "Export Test Batch Tool" }).click();

  const runs = await page.evaluate(() => (window as unknown as HarnessWindow).calls.filter((call) => call.command === "test_batch_tool"));
  expect(runs).toHaveLength(1);
  expect(runs[0].args).toEqual({
    runId: expect.any(String),
    request: { paths: [pdfPath, secondPdfPath], outputLocation: "alongsideInput", mode: "test" },
  });
  await expect(page.getByText("contract-tested.pdf").first()).toBeVisible();
  await expect(page.getByText("appendix-tested.pdf").first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Open contract-tested.pdf" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Show in Finder: contract-tested.pdf" })).toBeVisible();
});

test("switching between a batch tool, an existing batch tool and the scene editor keeps the open file", async ({ page }) => {
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await expect(fileRow(page, "contract.pdf")).toBeVisible();

  await rail(page).getByRole("button", { name: "Rotate PDFs" }).click();
  await expect(page.getByRole("button", { name: "Export Rotate PDFs" })).toBeVisible();
  await expect(fileRow(page, "contract.pdf")).toBeVisible();

  await rail(page).getByRole("button", { name: "Edit PDF" }).click();
  await expect(page.getByRole("group", { name: "PDF page canvas" })).toBeVisible();
  const opened = await page.evaluate(() => (window as unknown as HarnessWindow).calls.filter((call) => call.command === "inspect_pdf_scene").map((call) => call.args.request.path));
  expect(opened).toEqual([pdfPath]);

  await rail(page).getByRole("button", { name: "Test Batch Tool" }).click();
  await expect(page.getByRole("heading", { name: "Test Batch Tool" })).toBeVisible();
  await expect(fileRow(page, "contract.pdf")).toBeVisible();
});
