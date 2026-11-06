import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: null; reason: null; warnings: unknown[] };

type OrderingWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResult: string[] | null;
    __outputSettings: unknown;
    __modifiedMs: Record<string, number>;
    __takenNames: string[];
    __drop: (paths: string[]) => void;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as OrderingWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResult = null;
        win.__outputSettings = { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
        win.__modifiedMs = {};
        win.__takenNames = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        win.__drop = (paths) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths } });
            }
        };
        const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);
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
                if (command === "describe_intake_files") {
                    return (args as { paths: string[] }).paths.map((path) => ({ path, sizeBytes: 1000, modifiedMs: win.__modifiedMs[path] ?? null, pageCount: null }));
                }
                if (command === "plugin:dialog|open") return win.__dialogResult;
                if (command === "get_output_settings") return win.__outputSettings;
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "images_to_pdf") {
                    const { request } = args as { request: { paths: string[]; outputPath: string } };
                    const chosen = request.outputPath;
                    const chosenName = nameOf(chosen);
                    const taken = win.__takenNames.includes(chosenName);
                    const finalName = taken ? chosenName.replace(/\.pdf$/, "-1.pdf") : chosenName;
                    const outcome: Outcome = {
                        inputPath: request.paths[0],
                        outputPaths: [`${chosen.slice(0, chosen.length - chosenName.length)}${finalName}`],
                        detail: "",
                        state: "succeeded",
                        failure: null,
                        reason: null,
                        warnings: taken ? [{ code: "renamedToAvoidReplacing", message: "", details: { chosenName, finalName } }] : [],
                    };
                    return [outcome];
                }
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                return null;
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

const open = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Images to PDF");
};

const drop = async (page: Page, paths: string[]) => {
    await expect.poll(() => page.evaluate(() => (window as unknown as OrderingWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as OrderingWindow).__drop(paths), paths);
};

const pick = async (page: Page, paths: string[]) => {
    await page.evaluate((paths) => { (window as unknown as OrderingWindow).__dialogResult = paths; }, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
};

const rows = (page: Page) => page.getByRole("listbox").getByRole("option");

const requests = (page: Page) => page.evaluate(() => (window as unknown as OrderingWindow).__invocations
    .filter(({ command }) => command === "images_to_pdf")
    .map(({ args }) => (args as { request: { paths: string[]; outputLocation: unknown; outputPath: string } }).request));

const runAndSave = async (page: Page) => {
    await page.locator(".workspace-primary-action").click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect(dialog).toHaveCount(0);
};

test("the list starts in drop order, then in pick order for later picks", async ({ page }) => {
    await open(page);
    await drop(page, ["/local/scans/c.png", "/local/scans/a.jpg"]);
    await pick(page, ["/local/scans/b.heic", "/local/scans/d.webp"]);
    await expect(rows(page)).toHaveText([/c\.png/, /a\.jpg/, /b\.heic/, /d\.webp/]);

    await runAndSave(page);
    await expect.poll(() => requests(page)).toHaveLength(1);
    expect((await requests(page))[0].paths).toEqual(["/local/scans/c.png", "/local/scans/a.jpg", "/local/scans/b.heic", "/local/scans/d.webp"]);
});

test("drag, the keyboard shortcut and Move up or down set the page order", async ({ page }) => {
    await open(page);
    await drop(page, ["/local/a.png", "/local/b.png", "/local/c.png"]);

    await page.getByRole("button", { name: "Drag c.png to reorder" }).hover();
    await page.mouse.down();
    await rows(page).nth(0).hover();
    await page.mouse.up();
    await expect(rows(page)).toHaveText([/c\.png/, /a\.png/, /b\.png/]);
    await expect(page.getByRole("status").filter({ hasText: "Moved" })).toHaveText("Moved c.png to position 1.");

    const b = rows(page).filter({ hasText: "b.png" }).getByRole("button", { name: /^b\.png/ });
    await b.click();
    await b.press("Meta+ArrowUp");
    await expect(rows(page)).toHaveText([/c\.png/, /b\.png/, /a\.png/]);
    await expect(page.getByRole("status").filter({ hasText: "Moved" })).toHaveText("Moved b.png to position 2.");

    await page.getByRole("button", { name: "Move c.png down" }).click();
    await expect(rows(page)).toHaveText([/b\.png/, /c\.png/, /a\.png/]);
    await page.getByRole("button", { name: "Move a.png up" }).click();
    await expect(rows(page)).toHaveText([/b\.png/, /a\.png/, /c\.png/]);
    await expect(page.getByRole("button", { name: "Move b.png up" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Move c.png down" })).toBeDisabled();

    await runAndSave(page);
    await expect.poll(() => requests(page)).toHaveLength(1);
    expect((await requests(page))[0].paths).toEqual(["/local/b.png", "/local/a.png", "/local/c.png"]);
});

test("each sort reorders the pages, and Order added restores the drop order", async ({ page }) => {
    await open(page);
    await page.evaluate(() => {
        (window as unknown as OrderingWindow).__modifiedMs = { "/local/IMG_10.png": 3000, "/local/IMG_2.png": 1000, "/local/b.png": 2000 };
    });
    await drop(page, ["/local/IMG_10.png", "/local/b.png", "/local/IMG_2.png", "/local/undated.png"]);
    const sort = page.getByLabel("Sort by");
    await expect(sort).toHaveValue("added");
    await expect(sort.locator("option")).toHaveText(["Order added", "Name", "Modified date"]);

    await sort.selectOption({ label: "Name" });
    await expect(rows(page)).toHaveText([/b\.png/, /IMG_2\.png/, /IMG_10\.png/, /undated\.png/]);
    expect(await page.evaluate(() => (window as unknown as OrderingWindow).__invocations.some(({ command }) => command === "describe_intake_files"))).toBe(false);

    await sort.selectOption({ label: "Modified date" });
    await expect(rows(page)).toHaveText([/IMG_2\.png/, /b\.png/, /IMG_10\.png/, /undated\.png/]);

    await sort.selectOption({ label: "Order added" });
    await expect(rows(page)).toHaveText([/IMG_10\.png/, /b\.png/, /IMG_2\.png/, /undated\.png/]);

    await sort.selectOption({ label: "Name" });
    await runAndSave(page);
    await expect.poll(() => requests(page)).toHaveLength(1);
    expect((await requests(page))[0].paths).toEqual(["/local/b.png", "/local/IMG_2.png", "/local/IMG_10.png", "/local/undated.png"]);
});

test("alongside the original, the save dialog opens in the first image's folder with the combined name", async ({ page }) => {
    await open(page);
    await drop(page, ["/Users/test/Pictures/Trip/IMG_4012.jpg", "/Users/test/Pictures/Other/IMG_4013.jpg"]);
    await page.getByRole("button", { name: "Move IMG_4013.jpg up" }).click();

    await page.locator(".workspace-primary-action").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Save the combined PDF" })).toBeVisible();
    await expect(dialog.getByLabel("File name")).toHaveValue("IMG_4013-combined.pdf");
    await expect(dialog.getByTestId("save-folder")).toHaveText("~/Pictures/Other");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);
    expect(await requests(page)).toEqual([]);

    await runAndSave(page);
    await expect.poll(() => requests(page)).toHaveLength(1);
    expect((await requests(page))[0]).toMatchObject({ outputLocation: "alongsideInput", outputPath: "/Users/test/Pictures/Other/IMG_4013-combined.pdf" });
});

test("with a custom output folder, the save dialog opens there and the request carries the chosen path", async ({ page }) => {
    await page.addInitScript(() => { (window as unknown as OrderingWindow).__outputSettings = { mode: "customFolder", folder: "/Users/test/Documents/PDFs" }; });
    await open(page);
    await drop(page, ["/local/scans/receipt.png", "/local/scans/page-2.png"]);

    await page.locator(".workspace-primary-action").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByLabel("File name")).toHaveValue("receipt-combined.pdf");
    await expect(dialog.getByTestId("save-folder")).toHaveText("~/Documents/PDFs");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    expect(await requests(page)).toEqual([]);

    await page.locator(".workspace-primary-action").click();
    await dialog.getByLabel("File name").fill("receipts");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => requests(page)).toHaveLength(1);
    expect((await requests(page))[0]).toMatchObject({
        paths: ["/local/scans/receipt.png", "/local/scans/page-2.png"],
        outputLocation: { customFolder: "/Users/test/Documents/PDFs" },
        outputPath: "/Users/test/Documents/PDFs/receipts.pdf",
    });
});

test("the combined result lists every image with the shared PDF, and Open and Reveal use it", async ({ page }) => {
    await open(page);
    await drop(page, ["/Users/test/Pictures/Trip/IMG_4012.jpg", "/Users/test/Pictures/Trip/receipt.png", "/Users/test/Pictures/Trip/map.heic"]);
    await runAndSave(page);

    await expect(page.getByRole("status").filter({ hasText: "Made 1 PDF" })).toHaveText(/^Done\. Made 1 PDF from 3 images\s*Every image is in, in the order shown\.$/);
    const output = page.getByRole("button", { name: "Open IMG_4012-combined.pdf" });
    await expect(output).toBeVisible();
    await expect(page.getByText("in ~/Pictures/Trip")).toBeVisible();
    await expect(page.getByText(/Renamed\./)).toHaveCount(0);

    await expect(page.getByRole("heading", { level: 2, name: "3 images in this PDF" })).toBeVisible();
    const inputs = page.getByRole("list", { name: "3 images in this PDF" }).getByRole("listitem");
    await expect(inputs).toHaveText([/^IMG_4012\.jpg\s*Page 1\s*Added$/, /^receipt\.png\s*Page 2\s*Added$/, /^map\.heic\s*Page 3\s*Added$/]);
    await expect(page.getByRole("list", { name: "Results" })).toHaveCount(0);

    await output.click();
    await page.getByRole("button", { name: "Show in Finder: IMG_4012-combined.pdf" }).click();
    const actions = await page.evaluate(() => (window as unknown as OrderingWindow).__invocations
        .filter(({ command }) => command === "open_output_path" || command === "reveal_output_path"));
    expect(actions).toEqual([
        { command: "open_output_path", args: { path: "/Users/test/Pictures/Trip/IMG_4012-combined.pdf" } },
        { command: "reveal_output_path", args: { path: "/Users/test/Pictures/Trip/IMG_4012-combined.pdf" } },
    ]);
});

test("when the chosen name exists, the result shows the numbered name and that the original was left alone", async ({ page }) => {
    await open(page);
    await page.evaluate(() => { (window as unknown as OrderingWindow).__takenNames = ["IMG_4012-combined.pdf"]; });
    await drop(page, ["/Users/test/Pictures/Trip/IMG_4012.jpg", "/Users/test/Pictures/Trip/receipt.png"]);
    await runAndSave(page);

    await expect(page.getByRole("status").filter({ hasText: "Made 1 PDF" })).toContainText("Done with 1 warning. Made 1 PDF from 2 images");
    await expect(page.getByRole("button", { name: "Open IMG_4012-combined-1.pdf" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Show in Finder: IMG_4012-combined-1.pdf" })).toBeVisible();
    await expect(page.getByText("Renamed. IMG_4012-combined.pdf already exists and was left as it is.")).toBeVisible();
    await expect(page.getByRole("list", { name: "2 images in this PDF" }).getByRole("listitem")).toHaveCount(2);
});
