import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type ConvertWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __drop: (paths: string[]) => void;
    __finish: (outcomes: unknown[]) => void;
};

const textFonts = [{ requestedFamily: "Courier New", requestedStyle: "regular", fontUsed: "Liberation Mono", styleUsed: "regular", source: "bundled fallback" }];

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as ConvertWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        win.__drop = (paths) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths } });
            }
        };
        win.__finish = (outcomes) => finish?.(outcomes);
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "plugin:event|listen") {
                    const { event, handler } = args as { event: string; handler: number };
                    const id = nextId++;
                    listeners.set(id, { event, handler });
                    return id;
                }
                if (command === "plugin:event|unlisten") {
                    listeners.delete((args as { eventId: number }).eventId);
                    return null;
                }
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "shell_ready" || command === "open_output_path" || command === "reveal_output_path") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                return new Promise((resolve) => { finish = resolve; });
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
});

const openConvert = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Convert to PDF");
    await expect.poll(() => page.evaluate(() => (window as unknown as ConvertWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
};

const drop = (page: Page, paths: string[]) => page.evaluate((paths) => (window as unknown as ConvertWindow).__drop(paths), paths);

const fileRow = (page: Page, name: string) => page.getByRole("listbox").getByRole("option").filter({ hasText: name });

const convertRequest = (page: Page) => page.evaluate(() =>
    ((window as unknown as ConvertWindow).__invocations.find(({ command }) => command === "convert_to_pdf")?.args as { request: unknown } | undefined)?.request);

test("Convert to PDF sits in the PDF Conversion rail and shows text options only while a text file is listed", async ({ page }) => {
    await openConvert(page);
    await expect(page.getByRole("navigation", { name: "PDF Conversion" }).getByRole("button", { name: "Convert to PDF" })).toBeVisible();
    await expect(page.getByText("Options for each format appear here when a file of that format is in the list.", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "Plain text options" })).toHaveCount(0);

    await drop(page, ["/local/notes.txt"]);
    const options = page.getByRole("region", { name: "Plain text options" });
    await expect(options).toBeVisible();
    await expect(options).toContainText("Applies to notes.txt");
    await expect(options.getByLabel("Page size")).toHaveValue("a4");
    await expect(options.getByLabel("Encoding")).toHaveAccessibleDescription("Pick one if characters look wrong.");

    await fileRow(page, "notes.txt").getByRole("button", { name: /Remove/ }).click();
    await expect(page.getByRole("region", { name: "Plain text options" })).toHaveCount(0);
});

test("unsupported drops are refused with a reason, and later formats and PDFs are flagged on their own rows", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/notes.txt", "/local/archive.zip", "/local/photo.png", "/local/report.odt", "/local/signed.pdf"]);

    await expect(page.getByText(/archive\.zip/).first()).toBeVisible();
    await expect(fileRow(page, "archive.zip")).toHaveCount(0);
    await expect(fileRow(page, "photo.png")).toContainText("This file type can't become a PDF here.");
    await expect(fileRow(page, "photo.png")).toContainText("Won't run");
    await expect(fileRow(page, "report.odt")).toContainText("Not available yet");
    await expect(fileRow(page, "report.odt")).toContainText("ODT comes in a later update");
    await expect(fileRow(page, "signed.pdf")).toContainText("Will be skipped");
    await expect(fileRow(page, "signed.pdf")).toContainText("Already a PDF");
    await expect(fileRow(page, "notes.txt")).not.toContainText("Will be skipped");
    await expect(page.getByText("3 files won't become PDFs. Each one says why on its row.", { exact: true })).toBeVisible();
    await expect(page.getByText("This will make 1 PDF, 3 skipped", { exact: true })).toBeVisible();
});

test("a run sends only paths, output location and plain text options, and rows show fonts and skips", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/notes.txt", "/local/signed.pdf"]);
    const options = page.getByRole("region", { name: "Plain text options" });
    await options.getByLabel("Page size").selectOption("letter");
    await options.getByLabel("Encoding").selectOption("windows1252");
    await options.getByRole("checkbox", { name: "Wrap long lines" }).click();
    await page.locator(".workspace-primary-action").click();

    await expect.poll(() => convertRequest(page)).toEqual({
        paths: ["/local/notes.txt", "/local/signed.pdf"],
        outputLocation: "alongsideInput",
        plainText: { pageSize: "letter", margins: "normal", font: "monospace", encoding: "windows1252", wrapLines: false },
    });

    await page.evaluate((fonts) => (window as unknown as ConvertWindow).__finish([
        { inputPath: "/local/notes.txt", outputPaths: ["/local/notes.pdf"], detail: "1 page. Read as Windows-1252.", state: "succeeded", failure: null, reason: null, warnings: [], fonts },
        { inputPath: "/local/signed.pdf", outputPaths: [], detail: "", state: "skipped", failure: null, reason: { code: "alreadyInTargetFormat", message: "Already a PDF. Nothing was made.", pointer: null }, warnings: [] },
    ]), textFonts);

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });
    const saved = rows.filter({ hasText: "notes.txt" });
    await expect(saved).toContainText("1 page. Read as Windows-1252.");
    await expect(saved.getByRole("button", { name: "Open notes.pdf" })).toBeVisible();
    await expect(saved.getByRole("button", { name: "Show in Finder: notes.pdf" })).toBeVisible();

    const toggle = saved.getByRole("button", { name: "Show fonts" });
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await toggle.click();
    const fonts = page.getByRole("list", { name: "Fonts used in notes.txt" });
    await expect(fonts.getByRole("listitem")).toHaveText([/Courier New.*Liberation Mono.*Bundled fallback, same widths/]);
    await expect(saved.getByRole("button", { name: "Hide fonts" })).toHaveAttribute("aria-expanded", "true");
    await saved.getByRole("button", { name: "Hide fonts" }).click();
    await expect(fonts).toHaveCount(0);

    const skipped = rows.filter({ hasText: "signed.pdf" });
    await expect(skipped).toContainText("Skipped");
    await expect(skipped).toContainText("Already a PDF. Nothing was made.");
    await expect(skipped.getByRole("button", { name: /fonts/ })).toHaveCount(0);
});

test("a run without text files leaves plain text options out of the request", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/signed.pdf"]);
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({ paths: ["/local/signed.pdf"], outputLocation: "alongsideInput" });
});

test("the Markdown section appears only while a Markdown file is listed and sits beside the text section", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/release-notes.md"]);
    const markdown = page.getByRole("region", { name: "Markdown options" });
    await expect(markdown).toBeVisible();
    await expect(markdown).toContainText("Applies to release-notes.md");
    await expect(page.getByRole("region", { name: "Plain text options" })).toHaveCount(0);
    await expect(page.getByText("Options for each format appear here when a file of that format is in the list.", { exact: true })).toHaveCount(0);
    await expect(markdown.getByLabel("Page size")).toHaveValue("a4");
    await expect(markdown.getByLabel("Margins")).toHaveValue("normal");
    await expect(markdown.getByLabel("Body font")).toHaveValue("sans");
    await expect(markdown.getByRole("radiogroup", { name: "Code blocks" }).getByRole("radio", { name: "Shaded" })).toBeChecked();
    const contents = markdown.getByRole("checkbox", { name: "Add a table of contents" });
    await expect(contents).not.toBeChecked();
    await expect(contents).toHaveAccessibleDescription("Built from the headings, with links to each section.");

    await drop(page, ["/local/guide.markdown", "/local/notes.txt"]);
    await expect(markdown).toContainText("Applies to 2 Markdown files");
    await expect(page.getByRole("region", { name: "Plain text options" })).toBeVisible();

    await fileRow(page, "release-notes.md").getByRole("button", { name: /Remove/ }).click();
    await fileRow(page, "guide.markdown").getByRole("button", { name: /Remove/ }).click();
    await expect(page.getByRole("region", { name: "Markdown options" })).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Plain text options" })).toBeVisible();
});

test("a Markdown run carries only the Markdown section, and the left-out remote note shows on its row", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/release-notes.md", "/local/signed.pdf"]);
    const markdown = page.getByRole("region", { name: "Markdown options" });
    await markdown.getByLabel("Page size").selectOption("letter");
    await markdown.getByLabel("Body font").selectOption("serif");
    await markdown.getByRole("radio", { name: "Plain" }).click();
    await markdown.getByRole("checkbox", { name: "Add a table of contents" }).click();
    await page.locator(".workspace-primary-action").click();

    await expect.poll(() => convertRequest(page)).toEqual({
        paths: ["/local/release-notes.md", "/local/signed.pdf"],
        outputLocation: "alongsideInput",
        markdown: { pageSize: "letter", margins: "normal", bodyFont: "serif", codeStyle: "plain", tableOfContents: true },
    });

    const note = "3 pages. Remote content left out: 2 images from the internet weren't loaded.";
    await page.evaluate((note) => (window as unknown as ConvertWindow).__finish([
        { inputPath: "/local/release-notes.md", outputPaths: ["/local/release-notes.pdf"], detail: note, state: "succeeded", failure: null, reason: null, warnings: [], fonts: [] },
        { inputPath: "/local/signed.pdf", outputPaths: [], detail: "", state: "skipped", failure: null, reason: { code: "alreadyInTargetFormat", message: "Already a PDF. Nothing was made.", pointer: null }, warnings: [] },
    ]), note);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });
    const saved = rows.filter({ hasText: "release-notes.md" });
    await expect(saved).toContainText(note);
    await expect(saved.getByRole("button", { name: "Open release-notes.pdf" })).toBeVisible();
});

test("a run with Markdown and text files sends both sections", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/notes.txt", "/local/readme.MD"]);
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({
        paths: ["/local/notes.txt", "/local/readme.MD"],
        outputLocation: "alongsideInput",
        plainText: { pageSize: "a4", margins: "normal", font: "monospace", encoding: "detect", wrapLines: true },
        markdown: { pageSize: "a4", margins: "normal", bodyFont: "sans", codeStyle: "shaded", tableOfContents: false },
    });
});

test("an RTF file needs no format section, sends no RTF section, and its row lists the fonts it used", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/memo.rtf"]);
    await expect(fileRow(page, "memo.rtf")).not.toContainText("comes in a later update");
    await expect(fileRow(page, "memo.rtf")).not.toContainText("Not available yet");
    await expect(page.getByText("Options for each format appear here when a file of that format is in the list.", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: /options/ })).toHaveCount(0);
    await expect(page.getByText("This will make 1 PDF", { exact: true })).toBeVisible();
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({ paths: ["/local/memo.rtf"], outputLocation: "alongsideInput" });

    const fonts = [
        { requestedFamily: "Times New Roman", requestedStyle: "regular", fontUsed: "Liberation Serif", styleUsed: "regular", source: "bundled fallback" },
        { requestedFamily: "Arial", requestedStyle: "bold", fontUsed: "Liberation Sans", styleUsed: "bold", source: "bundled fallback" },
        { requestedFamily: "Brand Grotesk", requestedStyle: "regular", fontUsed: "DejaVu Sans", styleUsed: "regular", source: "bundled default" },
    ];
    const note = "Omitted content: 1 picture couldn't be shown and was left out.";
    await page.evaluate(({ fonts, note }) => (window as unknown as ConvertWindow).__finish([
        { inputPath: "/local/memo.rtf", outputPaths: ["/local/memo.pdf"], detail: "2 pages.", state: "succeeded", failure: null, reason: null, warnings: [{ code: "omittedContent", message: note }], fonts },
    ]), { fonts, note });
    const saved = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") }).filter({ hasText: "memo.rtf" });
    await expect(saved.locator(".result-row-detail")).toHaveText([note, "2 pages."]);
    await expect(saved).toHaveAttribute("data-tone", "warn");
    await saved.getByRole("button", { name: "Show fonts" }).click();
    await expect(page.getByRole("list", { name: "Fonts used in memo.rtf" }).getByRole("listitem")).toHaveText([
        /Times New Roman.*Liberation Serif.*Bundled fallback, same widths/,
        /Arial.*Liberation Sans.*Bundled fallback, same widths/,
        /Brand Grotesk.*DejaVu Sans.*Bundled default font/,
    ]);
});

test("a Word document needs no format section, and a password-protected one points to Lock & Unlock on its row", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/report.docx", "/local/locked.DOCX"]);
    for (const name of ["report.docx", "locked.DOCX"]) {
        await expect(fileRow(page, name)).not.toContainText("Not available yet");
        await expect(fileRow(page, name)).not.toContainText("Won't run");
    }
    await expect(page.getByText("Options for each format appear here when a file of that format is in the list.", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: /options/ })).toHaveCount(0);
    await expect(page.getByText("This will make 2 PDFs", { exact: true })).toBeVisible();
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({ paths: ["/local/report.docx", "/local/locked.DOCX"], outputLocation: "alongsideInput" });

    const locked = "It's password-protected. Unlock it in Lock & Unlock, then convert the unlocked copy.";
    await page.evaluate((locked) => (window as unknown as ConvertWindow).__finish([
        { inputPath: "/local/report.docx", outputPaths: ["/local/report.pdf"], detail: "2 pages. 1 font, embedded from the file.", state: "succeeded", failure: null, reason: null, warnings: [],
            fonts: [{ requestedFamily: "Brand Script", requestedStyle: "regular", fontUsed: "Brand Script", styleUsed: "regular", source: "embedded" }] },
        { inputPath: "/local/locked.DOCX", outputPaths: [], detail: "", state: "failed", failure: { kind: "invalidInput", message: locked, pointer: { kind: "tool", toolId: "lock-unlock" } }, reason: null, warnings: [] },
    ]), locked);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });
    const saved = rows.filter({ hasText: "report.docx" });
    await expect(saved).toContainText("2 pages. 1 font, embedded from the file.");
    await saved.getByRole("button", { name: "Show fonts" }).click();
    await expect(page.getByRole("list", { name: "Fonts used in report.docx" }).getByRole("listitem")).toHaveText([/Brand Script.*Embedded/]);
    const failed = rows.filter({ hasText: "locked.DOCX" });
    await expect(failed).toContainText(locked);
    await failed.getByRole("button", { name: "Open Lock & Unlock" }).click();
    await expect(page.getByRole("heading", { name: "Lock & Unlock" }).first()).toBeVisible();
});

test("Word rows show omitted content as a warning and approximate layout as a note, as the results board draws them", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/budget.docx", "/local/brochure.docx", "/local/review.docx", "/local/plain.docx"]);
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({ paths: ["/local/budget.docx", "/local/brochure.docx", "/local/review.docx", "/local/plain.docx"], outputLocation: "alongsideInput" });

    const omitted = "Omitted content: 1 chart has no stored image, so it was left out.";
    const layout = "Approximate layout: 3 floating objects on pages 1 and 2 were placed as close as possible.";
    const saved = (name: string, detail: string, warnings: unknown[]) =>
        ({ inputPath: `/local/${name}`, outputPaths: [`/local/${name.replace(".docx", ".pdf")}`], detail, state: "succeeded", failure: null, reason: null, warnings });
    await page.evaluate((outcomes) => (window as unknown as ConvertWindow).__finish(outcomes), [
        saved("budget.docx", "3 pages.", [{ code: "omittedContent", message: omitted }]),
        saved("brochure.docx", "2 pages.", [{ code: "approximateLayout", message: layout }]),
        saved("review.docx", "4 pages.", [{ code: "omittedContent", message: omitted }, { code: "approximateLayout", message: layout }]),
        saved("plain.docx", "1 page.", []),
    ]);

    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });
    const budget = rows.filter({ hasText: "budget.docx" });
    await expect(budget).toHaveAttribute("data-tone", "warn");
    await expect(budget.locator(".result-row-icon svg")).toHaveClass(/lucide-triangle-alert/);
    await expect(budget.locator(".result-row-detail")).toHaveText([omitted, "3 pages."]);
    await expect(budget.locator(".result-row-detail").first()).toHaveAttribute("data-tone", "warn");
    await expect(budget.locator(".result-row-state [aria-hidden]")).toHaveText("Saved");
    await expect(budget.locator(".result-row-state .sr-only")).toHaveText("Saved with a warning");
    await expect(budget.getByRole("button", { name: "Open budget.pdf", exact: true })).toBeVisible();

    const brochure = rows.filter({ hasText: "brochure.docx" });
    await expect(brochure).toHaveAttribute("data-tone", "note");
    await expect(brochure.locator(".result-row-icon svg")).toHaveClass(/lucide-info/);
    await expect(brochure.locator(".result-row-detail")).toHaveText([layout, "2 pages."]);
    await expect(brochure.locator(".result-row-detail").first()).not.toHaveAttribute("data-tone", /.+/);
    await expect(brochure.locator(".result-row-state [aria-hidden]")).toHaveText("Saved");
    await expect(brochure.locator(".result-row-state .sr-only")).toHaveText("Saved with a note");
    const muted = await brochure.locator(".result-row-detail").first().evaluate((line) => getComputedStyle(line).color);
    const amber = await budget.locator(".result-row-detail").first().evaluate((line) => getComputedStyle(line).color);
    expect(muted).not.toBe(amber);

    const review = rows.filter({ hasText: "review.docx" });
    await expect(review).toHaveAttribute("data-tone", "warn");
    await expect(review.locator(".result-row-detail")).toHaveText([omitted, layout, "4 pages."]);
    await expect(review.locator(".result-row-state .sr-only")).toHaveText("Saved with warnings");

    await expect(rows.filter({ hasText: "plain.docx" })).toHaveAttribute("data-tone", "ok");
    await expect(page.locator(".result-summary")).toContainText("3 PDFs have a warning or note.");
});

test("the Tables section appears only while a CSV or TSV is listed and picks the delimiter by file type", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/sales-export.csv"]);
    await expect(fileRow(page, "sales-export.csv")).not.toContainText("Not available yet");
    const tables = page.getByRole("region", { name: "Tables options" });
    await expect(tables).toBeVisible();
    await expect(tables).toContainText("Applies to sales-export.csv");
    await expect(page.getByText("Options for each format appear here when a file of that format is in the list.", { exact: true })).toHaveCount(0);
    const delimiter = tables.getByLabel("Delimiter");
    await expect(delimiter).toHaveValue("auto");
    await expect(delimiter.locator("option:checked")).toHaveText("By file type");
    await expect(delimiter).toHaveAccessibleDescription("Comma for CSV and tab for TSV, unless you pick one.");
    await expect(tables.getByLabel("Encoding")).toHaveValue("detect");
    await expect(tables.getByRole("checkbox", { name: "First row is a header, repeated on each page" })).toBeChecked();
    await expect(tables.getByLabel("Orientation")).toHaveValue("landscape");
    await expect(tables.getByLabel("Fit")).toHaveValue("width");

    await drop(page, ["/local/people.TSV", "/local/notes.txt"]);
    await expect(tables).toContainText("Applies to 2 table files");
    await expect(page.getByRole("region", { name: "Plain text options" })).toBeVisible();

    await fileRow(page, "sales-export.csv").getByRole("button", { name: /Remove/ }).click();
    await expect(tables).toContainText("Applies to people.TSV");
    await fileRow(page, "people.TSV").getByRole("button", { name: /Remove/ }).click();
    await expect(page.getByRole("region", { name: "Tables options" })).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Plain text options" })).toBeVisible();
});

test("a table run carries the chosen table options and only the sections in the list", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/ledger.csv", "/local/people.tsv"]);
    const tables = page.getByRole("region", { name: "Tables options" });
    await tables.getByLabel("Delimiter").selectOption("semicolon");
    await tables.getByLabel("Encoding").selectOption("windows1252");
    await tables.getByRole("checkbox", { name: "First row is a header, repeated on each page" }).click();
    await tables.getByLabel("Orientation").selectOption("portrait");
    await tables.getByLabel("Fit").selectOption("none");
    await page.locator(".workspace-primary-action").click();

    await expect.poll(() => convertRequest(page)).toEqual({
        paths: ["/local/ledger.csv", "/local/people.tsv"],
        outputLocation: "alongsideInput",
        tables: { delimiter: "semicolon", headerRow: false, encoding: "windows1252", orientation: "portrait", fit: "none" },
    });

    const note = "4 pages. 1,240 rows, semicolon. Read as Windows-1252.";
    await page.evaluate((note) => (window as unknown as ConvertWindow).__finish([
        { inputPath: "/local/ledger.csv", outputPaths: ["/local/ledger.pdf"], detail: note, state: "succeeded", failure: null, reason: null, warnings: [], fonts: [] },
        { inputPath: "/local/people.tsv", outputPaths: [], detail: "", state: "failed", failure: { kind: "invalidInput", message: "This table has no rows, so no PDF was made.", pointer: null }, reason: null, warnings: [] },
    ]), note);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });
    await expect(rows.filter({ hasText: "ledger.csv" })).toContainText(note);
    await expect(rows.filter({ hasText: "people.tsv" })).toContainText("This table has no rows, so no PDF was made.");
});

test("a TSV run with default table options sends the by-file-type delimiter", async ({ page }) => {
    await openConvert(page);
    await drop(page, ["/local/people.tsv", "/local/notes.txt"]);
    await page.locator(".workspace-primary-action").click();
    await expect.poll(() => convertRequest(page)).toEqual({
        paths: ["/local/people.tsv", "/local/notes.txt"],
        outputLocation: "alongsideInput",
        plainText: { pageSize: "a4", margins: "normal", font: "monospace", encoding: "detect", wrapLines: true },
        tables: { delimiter: "auto", headerRow: true, encoding: "detect", orientation: "landscape", fit: "width" },
    });
});
