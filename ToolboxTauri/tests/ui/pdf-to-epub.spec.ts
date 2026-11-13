import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type EpubWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
    __outcomes: unknown[] | null;
};

const runFolder = "/Users/test/Books";
const scanned = "PDF contains no selectable text; scanned PDFs require OCR. No EPUB was made.";
const locked = "This PDF is password-protected. Remove the password, then convert again.";

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as EpubWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__outcomes = null;
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "plugin:event|listen") return nextId++;
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_quick_actions") return [];
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "shell_ready" || command === "open_output_path" || command === "reveal_output_path") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                if (command === "read_pdf_properties") return (args as { request: { paths: string[] } }).request.paths.map((inputPath) => ({
                    inputPath, outputPaths: [], state: "succeeded", failure: null, reason: null, warnings: [],
                    detail: JSON.stringify({ title: "The Long Road Home", author: "Mira Okafor", subject: null }),
                }));
                if (win.__outcomes) return win.__outcomes;
                const { request } = (args ?? {}) as { request?: { paths?: string[] } };
                return (request?.paths ?? []).map((inputPath) => ({
                    inputPath, outputPaths: [inputPath.replace(/\.pdf$/i, ".epub")], detail: "3 pages.", state: "succeeded", failure: null, reason: null, warnings: [],
                }));
            },
            transformCallback: (callback: (event: unknown) => void) => {
                const id = nextId++;
                callbacks.set(id, callback);
                return id;
            },
            unregisterCallback: (id: number) => callbacks.delete(id),
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    });
    await page.goto("/");
});

const queueDialog = (page: Page, result: string | string[]) =>
    page.evaluate((result) => { (window as unknown as EpubWindow).__dialogResults.push(result); }, result);

const choose = async (page: Page, files: string[]) => {
    await queueDialog(page, files);
    await page.getByRole("button", { name: "Choose files to process" }).click();
};

const invocationsOf = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as EpubWindow).__invocations.filter((invocation) => invocation.command === command).map(({ args }) => args),
command);

const run = async (page: Page) => {
    const before = (await invocationsOf(page, "pdf_to_epub")).length;
    await page.locator(".workspace-primary-action").click();
    await expect.poll(async () => (await invocationsOf(page, "pdf_to_epub")).length).toBe(before + 1);
    return ((await invocationsOf(page, "pdf_to_epub")) as Array<{ request: Record<string, unknown> }>)[before].request;
};

const destination = (page: Page) => page.getByRole("region", { name: "Destination" });

test("PDF to EPUB sits in the PDF Conversion rail and names the run by its PDFs", async ({ page }) => {
    await openToolBySearch(page, "PDF to EPUB");
    await expect(page.getByRole("navigation", { name: "PDF Conversion" }).getByRole("button", { name: "PDF to EPUB" })).toBeVisible();
    await expect(page.getByText("Accepts PDF. Makes one EPUB for each PDF. Works best on text-heavy books, reports and articles.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Convert", exact: true })).toBeDisabled();
    await expect(page.getByText("Add PDFs to start", { exact: true })).toBeVisible();

    await choose(page, ["/Users/test/books/field notes.pdf", "/Users/test/books/memo.pdf"]);
    await expect(page.getByRole("button", { name: "Convert 2 PDFs", exact: true })).toBeEnabled();
    await expect(page.getByText("This will make up to 2 EPUBs. A PDF with no selectable text is skipped.")).toBeVisible();
    await expect(page.getByText("Originals are only read, never changed.")).toBeVisible();
    await expect(destination(page)).toContainText("field notes.epub");
});

test("PDF to EPUB sends the paths and the output location, and a folder chosen for this run keeps the saved setting", async ({ page }) => {
    const paths = ["/Users/test/books/field notes.pdf", "/Users/test/books/memo.pdf"];
    await openToolBySearch(page, "PDF to EPUB");
    await choose(page, paths);
    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("Next to each original");
    expect(await run(page)).toEqual({ paths, outputLocation: "alongsideInput", removeHeadersAndFooters: true, cover: "firstPage" });

    await queueDialog(page, runFolder);
    await destination(page).getByRole("button", { name: "Change for this run" }).click();
    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("~/Books");
    await expect(destination(page)).toContainText("This run only");
    expect(await run(page)).toEqual({ paths, outputLocation: { customFolder: runFolder }, removeHeadersAndFooters: true, cover: "firstPage" });
    expect(await invocationsOf(page, "set_output_settings")).toEqual([]);
});

test("PDF to EPUB removes running headers, footers and page numbers by default and sends the choice", async ({ page }) => {
    const paths = ["/Users/test/books/field notes.pdf"];
    await openToolBySearch(page, "PDF to EPUB");
    const cleanUp = page.getByRole("group", { name: "Clean up" });
    const removal = cleanUp.getByRole("checkbox", { name: "Remove running headers, footers and page numbers" });
    await expect(removal).toBeChecked();
    await expect(removal).toHaveAccessibleDescription("Text repeated on most pages is left out of the book.");

    await choose(page, paths);
    expect((await run(page)).removeHeadersAndFooters).toBe(true);

    await removal.click();
    await expect(removal).not.toBeChecked();
    expect(await run(page)).toMatchObject({ paths, outputLocation: "alongsideInput", removeHeadersAndFooters: false });
});

test("PDF to EPUB makes a cover from the first page by default and sends the choice", async ({ page }) => {
    await openToolBySearch(page, "PDF to EPUB");
    const cover = page.getByRole("group", { name: "Cover" }).getByRole("radiogroup", { name: "Cover" });
    await expect(cover.getByRole("radio", { name: "First page" })).toBeChecked();
    await expect(cover).toHaveAccessibleDescription("The first page becomes the cover.");

    await choose(page, ["/Users/test/books/field notes.pdf", "/Users/test/books/memo.pdf"]);
    expect((await run(page)).cover).toBe("firstPage");
    await cover.getByRole("radio", { name: "No cover" }).click();
    await expect(cover.getByRole("radio", { name: "No cover" })).toBeChecked();
    expect((await run(page)).cover).toBe("none");
});

test("PDF to EPUB fills in one PDF's title and author, sends the edits, and hides them for a batch", async ({ page }) => {
    const single = "/Users/test/books/the-long-road-home.pdf";
    await openToolBySearch(page, "PDF to EPUB");
    const details = page.getByRole("group", { name: "Book details" });
    await choose(page, [single]);
    const title = details.getByRole("textbox", { name: "Title" });
    const author = details.getByRole("textbox", { name: "Author" });
    await expect(title).toHaveValue("The Long Road Home");
    await expect(author).toHaveValue("Mira Okafor");
    await expect(title).toHaveAccessibleDescription("Filled in from the PDF. Your changes go into this EPUB only; the PDF isn't changed.");
    expect(await invocationsOf(page, "read_pdf_properties")).toEqual([{ request: { paths: [single] } }]);
    expect(await run(page)).toEqual({
        paths: [single], outputLocation: "alongsideInput", removeHeadersAndFooters: true, cover: "firstPage",
        title: "The Long Road Home", author: "Mira Okafor",
    });

    await title.fill("Road Home");
    await author.fill("M. Okafor");
    expect(await run(page)).toMatchObject({ paths: [single], title: "Road Home", author: "M. Okafor" });

    await choose(page, ["/Users/test/books/memo.pdf"]);
    await expect(details.getByRole("textbox")).toHaveCount(0);
    await expect(details).toContainText("Each book uses its PDF's title and author. Convert one PDF to edit them.");
    const batch = await run(page);
    expect(batch.paths).toEqual([single, "/Users/test/books/memo.pdf"]);
    expect(batch).not.toHaveProperty("title");
    expect(batch).not.toHaveProperty("author");
});

test("PDF to EPUB shows each PDF's own result with Open and Reveal on converted rows", async ({ page }) => {
    const paths = ["/Users/test/books/field notes.pdf", "/Users/test/books/scan.pdf", "/Users/test/books/locked.pdf"];
    await openToolBySearch(page, "PDF to EPUB");
    await choose(page, paths);
    await page.evaluate(({ paths, scanned, locked }) => {
        (window as unknown as EpubWindow).__outcomes = [
            {
                inputPath: paths[0], outputPaths: ["/Users/test/books/field notes-1.epub"], detail: "3 pages.", state: "succeeded", failure: null, reason: null,
                warnings: [{ code: "renamedToAvoidReplacing", message: "Saved as field notes-1.epub so the existing field notes.epub was not replaced.", details: { chosenName: "field notes.epub", finalName: "field notes-1.epub" } }],
            },
            {
                inputPath: paths[1], outputPaths: [], detail: "", state: "skipped", failure: null,
                reason: { code: "needsOcr", message: scanned, pointer: { kind: "tool", toolId: "pdf-ocr" } }, warnings: [],
            },
            {
                inputPath: paths[2], outputPaths: [], detail: "", state: "failed",
                failure: { kind: "invalidInput", message: locked, pointer: { kind: "tool", toolId: "lock-unlock" } }, reason: null, warnings: [],
            },
        ];
    }, { paths, scanned, locked });
    await page.getByRole("button", { name: "Convert 3 PDFs", exact: true }).click();

    const rows = page.locator(".result-row");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toHaveAttribute("data-state", "succeeded");
    await expect(rows.nth(0)).toContainText("1 EPUB. 1 renamed, because field notes.epub already existed. It ends in -1.");
    await rows.nth(0).getByRole("button", { name: "Open field notes-1.epub", exact: true }).click();
    await rows.nth(0).getByRole("button", { name: /^Show in .+: field notes-1\.epub$/ }).click();
    expect(await page.evaluate(() => (window as unknown as EpubWindow).__invocations
        .filter(({ command }) => command === "open_output_path" || command === "reveal_output_path")
        .map(({ command, args }) => [command, (args as { path: string }).path]))).toEqual([
        ["open_output_path", "/Users/test/books/field notes-1.epub"],
        ["reveal_output_path", "/Users/test/books/field notes-1.epub"],
    ]);

    await expect(rows.nth(1)).toHaveAttribute("data-state", "skipped");
    await expect(rows.nth(1)).toContainText(scanned);
    await expect(rows.nth(1).getByRole("button", { name: "Open OCR PDF", exact: true })).toBeVisible();
    await expect(rows.nth(1).getByRole("button", { name: /^(Open scan|Show in)/ })).toHaveCount(0);

    await expect(rows.nth(2)).toHaveAttribute("data-state", "failed");
    await expect(rows.nth(2)).toContainText(locked);
    await expect(rows.nth(2).getByRole("button", { name: /^Show in/ })).toHaveCount(0);
});
