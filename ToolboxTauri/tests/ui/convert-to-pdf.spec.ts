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
    await drop(page, ["/local/notes.txt", "/local/archive.zip", "/local/photo.png", "/local/report.docx", "/local/signed.pdf"]);

    await expect(page.getByText(/archive\.zip/).first()).toBeVisible();
    await expect(fileRow(page, "archive.zip")).toHaveCount(0);
    await expect(fileRow(page, "photo.png")).toContainText("This file type can't become a PDF here.");
    await expect(fileRow(page, "photo.png")).toContainText("Won't run");
    await expect(fileRow(page, "report.docx")).toContainText("Not available yet");
    await expect(fileRow(page, "report.docx")).toContainText("DOCX comes in a later update");
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
