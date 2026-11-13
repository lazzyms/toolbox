import { expect, test, type Locator, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type FormattedWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __finish: (outcomes: unknown[]) => void;
    __lastSaveFolder?: string;
    __dialogResults: Array<string | null>;
    __drop: (paths: string[], point?: { x: number; y: number }) => void;
    /** What preview_formatted_pdf replies; while `__holdPreview` is set, each reply waits in `__heldPreviews`. */
    __preview: unknown;
    __holdPreview: boolean;
    __heldPreviews: Array<(preview: unknown) => void>;
    /** When set, preview_formatted_pdf rejects with it, as the native command does for a missing script pack. */
    __previewError: unknown;
    /** What the script_packs query reports. */
    __scriptPacks: unknown[];
    /** Sends a native event, as `pack-changed`, to the app's listeners. */
    __emit: (event: string, payload: unknown) => void;
    /** What list_formatted_pdf_fonts replies: the bundled families, then the installed ones. */
    __fonts: { bundled: string[]; system: string[] };
};

/** One-pixel PNGs in two colours, so each faked page is told apart by its image. */
const PAGE_ONE = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGOQi1rwHwADygIY2ofr+QAAAABJRU5ErkJggg==";
const PAGE_TWO = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGM4oaHxHwAE7AIY0raoogAAAABJRU5ErkJggg==";
const fakePreview = (pages: string[], pageCount = pages.length) => ({ pageCount, pages: pages.map((dataUrl) => ({ dataUrl, width: 566, height: 800 })), missingImages: [] });

/** Where the faked app keeps the one draft. */
const DRAFT_KEY = "toolbox-test-formatted-draft";

test.beforeEach(async ({ page }) => {
    await page.addInitScript((DRAFT_KEY) => {
        const win = window as unknown as FormattedWindow;
        let finish: ((outcomes: unknown) => void) | null = null;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__preview = { pageCount: 1, pages: [{ dataUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGOQi1rwHwADygIY2ofr+QAAAABJRU5ErkJggg==", width: 566, height: 800 }], missingImages: [] };
        win.__holdPreview = false;
        win.__heldPreviews = [];
        win.__previewError = null;
        win.__scriptPacks = [];
        win.__fonts = { bundled: ["Liberation Sans", "Liberation Serif", "Liberation Mono", "Carlito", "Caladea", "DejaVu Sans"], system: ["Calibri", "Georgia"] };
        win.__drop = (paths, point) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths, point } });
            }
        };
        win.__emit = (event, payload) => {
            for (const [id, listener] of listeners) {
                if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
            }
        };
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        win.__finish = (outcomes) => finish?.(outcomes);
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return win.__lastSaveFolder ?? "/Users/test/Documents";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "create_formatted_pdf") return new Promise((resolve) => { finish = resolve; });
                if (command === "preview_formatted_pdf" && win.__previewError) throw win.__previewError;
                if (command === "preview_formatted_pdf") return win.__holdPreview ? new Promise((resolve) => { win.__heldPreviews.push(resolve); }) : win.__preview;
                if (command === "list_formatted_pdf_fonts") return win.__fonts;
                if (command === "script_packs") return win.__scriptPacks;
                // The draft lives in session storage, so it survives a reload as the app-data file survives a restart.
                if (command === "load_formatted_draft") {
                    const stored = sessionStorage.getItem(DRAFT_KEY);
                    if (stored === "damaged") {
                        sessionStorage.removeItem(DRAFT_KEY);
                        return { draft: null, problem: "Your saved draft was damaged, so it was cleared." };
                    }
                    return { draft: stored ? JSON.parse(stored) : null, problem: null };
                }
                if (command === "save_formatted_draft") {
                    const draft = { document: (args as { document: unknown }).document, savedAt: Date.now() };
                    sessionStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
                    return draft;
                }
                if (command === "clear_formatted_draft") { sessionStorage.removeItem(DRAFT_KEY); return null; }
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "plugin:event|listen") {
                    const { event, handler } = args as { event: string; handler: number };
                    const id = nextId++;
                    listeners.set(id, { event, handler });
                    return id;
                }
                if (command === "plugin:event|unlisten") listeners.delete((args as { eventId: number }).eventId);
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
    }, DRAFT_KEY);
});

const openFormatted = async (page: Page) => {
    await page.goto("/");
    await openToolBySearch(page, "Formatted PDF");
};

const invocation = (page: Page, name: string) => page.evaluate((name) =>
    ((window as unknown as FormattedWindow).__invocations.findLast(({ command }) => command === name)?.args as { request: unknown } | undefined)?.request, name);

const block = (page: Page, label: string) => page.getByRole("list", { name: "Document blocks, in order" }).getByRole("textbox", { name: label });

const run = (text: string, bold: boolean, italic: boolean) => ({ text, bold, italic });
const plain = (text: string) => run(text, false, false);

/** Saves through the save step and returns the blocks create_formatted_pdf received. */
const savedBlocks = async (page: Page) => {
    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => invocation(page, "create_formatted_pdf")).toBeDefined();
    return ((await invocation(page, "create_formatted_pdf")) as { document: { blocks: unknown } }).document.blocks;
};

/** Sends a key the way the browser does while an input method is composing text. */
const composingKey = (page: Page, init: KeyboardEventInit, how: "isComposing" | "keyCode229") => page.evaluate(({ init, how }) => {
    const event = new KeyboardEvent("keydown", { ...init, bubbles: true, cancelable: true, isComposing: how === "isComposing" });
    if (how === "keyCode229") Object.defineProperty(event, "keyCode", { value: 229 });
    document.activeElement?.dispatchEvent(event);
}, { init, how });

/** Puts the caret at the start or end of the focused field (Home and End differ between macOS and other systems). */
const caretAt = (page: Page, where: "start" | "end") => page.evaluate((where) => {
    const range = document.createRange();
    range.selectNodeContents(document.activeElement!);
    range.collapse(where === "start");
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
}, where);

const defaultPage = { pageSize: "a4", orientation: "portrait", margins: "normal", header: "", footer: "", pageNumbers: false };

/** Opens a fresh document, applies one page-setting change, saves and returns the page settings create_formatted_pdf received. */
const pageSentAfter = async (page: Page, change: (page: Page) => Promise<void>) => {
    // The save is never finished here, so the last case's draft is still stored; each case starts from a new document.
    await page.goto("/");
    await page.evaluate((key) => sessionStorage.removeItem(key), DRAFT_KEY);
    await openToolBySearch(page, "Formatted PDF");
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");
    await change(page);
    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => invocation(page, "create_formatted_pdf")).toBeDefined();
    return ((await invocation(page, "create_formatted_pdf")) as { document: { page: unknown } }).document.page;
};

const saveButton = (page: Page) => page.getByRole("button", { name: /^Save PDF/ });

const resultRows = (page: Page) => page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });

/** Types a heading, then a paragraph with a line break, and marks the first block as Heading 1. */
const writeChecklist = async (page: Page) => {
    await block(page, "Block 1, Paragraph").fill("Move-out checklist");
    await page.getByLabel("Block type").selectOption("heading1");
    await expect(block(page, "Block 1, Heading 1")).toBeFocused();
    await caretAt(page, "end");
    await page.keyboard.press("Enter");
    const second = block(page, "Block 2, Paragraph");
    await expect(second).toBeFocused();
    await second.pressSequentially("Return the keys.");
    await second.press("Shift+Enter");
    await second.pressSequentially("Read the meters.");
};

test("Formatted PDF sits in the PDF Conversion rail and cannot save an empty document", async ({ page }) => {
    await openFormatted(page);
    await expect(page.getByRole("navigation", { name: "PDF Conversion" }).getByRole("button", { name: "Formatted PDF" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Formatted PDF" })).toBeVisible();
    await expect(saveButton(page)).toBeDisabled();
    await expect(page.getByText("Empty document. Add some text to save a PDF.", { exact: true })).toBeVisible();

    // Only blank space is still an empty document.
    await block(page, "Block 1, Paragraph").fill("   ");
    await expect(saveButton(page)).toBeDisabled();
    await block(page, "Block 1, Paragraph").fill("Hello");
    await expect(saveButton(page)).toBeEnabled();
    await block(page, "Block 1, Paragraph").fill("");
    await expect(saveButton(page)).toBeDisabled();
    expect(await invocation(page, "create_formatted_pdf")).toBeUndefined();
});

test("Formatted PDF sends the exact structured document and shows the numbered name with Open and Reveal", async ({ page }) => {
    await openFormatted(page);
    await writeChecklist(page);
    await page.getByLabel("Title", { exact: true }).fill("Move-out checklist");
    await page.getByLabel("Author", { exact: true }).fill("Sam Lee");

    await saveButton(page).click();
    const save = page.getByRole("dialog", { name: "Save the PDF" });
    await expect(save.getByLabel("File name")).toHaveValue("Move-out checklist.pdf");
    await expect(save.getByTestId("save-folder")).toHaveText("~/Documents");
    await save.getByRole("button", { name: "Save" }).click();

    await expect.poll(() => invocation(page, "create_formatted_pdf")).toEqual({
        document: {
            title: "Move-out checklist",
            author: "Sam Lee",
            blocks: [
                { kind: "heading1", text: "Move-out checklist" },
                { kind: "paragraph", runs: [plain("Return the keys.\nRead the meters.")] },
            ],
            page: defaultPage,
            font: { family: "Liberation Serif", size: 11 },
        },
        outputPath: "/Users/test/Documents/Move-out checklist.pdf",
    });

    await page.evaluate(() => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Move-out checklist.pdf",
        outputPaths: ["/Users/test/Documents/Move-out checklist-1.pdf"],
        detail: "1 page, A4.",
        state: "succeeded",
        failure: null,
        reason: null,
        warnings: [{ code: "renamedToAvoidReplacing", message: "", details: { chosenName: "Move-out checklist.pdf", finalName: "Move-out checklist-1.pdf" } }],
        fonts: [],
    }]));
    const saved = resultRows(page);
    await expect(saved).toHaveCount(1);
    await expect(saved.locator(".result-row-name")).toHaveText("Move-out checklist-1.pdf");
    await expect(saved).toContainText("Saved as Move-out checklist-1.pdf. Renamed to avoid replacing an existing file.");
    await expect(saved).toContainText("1 page, A4.");
    await expect(saved.getByRole("button", { name: "Open Move-out checklist-1.pdf" })).toBeVisible();
    await expect(saved.getByRole("button", { name: "Show in Finder: Move-out checklist-1.pdf" })).toBeVisible();

    await saved.getByRole("button", { name: "Show in Finder: Move-out checklist-1.pdf" }).click();
    await expect.poll(() => page.evaluate(() => (window as unknown as FormattedWindow).__invocations.find(({ command }) => command === "reveal_output_path")?.args))
        .toEqual({ path: "/Users/test/Documents/Move-out checklist-1.pdf" });

    await page.getByRole("button", { name: "New document" }).click();
    await expect(resultRows(page)).toHaveCount(0);
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect(saveButton(page)).toBeDisabled();
});

// One test per control: each one reloads the app, which is too slow to share one test's time on Windows on Arm.
const pageSettingCases: Array<[string, (settings: Locator) => Promise<unknown>, Record<string, unknown>]> = [
    ["no change", async () => undefined, {}],
    ["Letter size", (settings) => settings.getByLabel("Size", { exact: true }).selectOption("letter"), { pageSize: "letter" }],
    ["Legal size", (settings) => settings.getByLabel("Size", { exact: true }).selectOption("legal"), { pageSize: "legal" }],
    ["landscape", (settings) => settings.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Landscape" }).click(), { orientation: "landscape" }],
    ["narrow margins", (settings) => settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Narrow" }).click(), { margins: "narrow" }],
    ["wide margins", (settings) => settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Wide" }).click(), { margins: "wide" }],
    ["header text", (settings) => settings.getByLabel("Header text").fill("Flat 4B handover"), { header: "Flat 4B handover" }],
    ["footer text", (settings) => settings.getByLabel("Footer text").fill("Confidential"), { footer: "Confidential" }],
    ["page numbers", (settings) => settings.getByLabel("Page numbers at the bottom").check(), { pageNumbers: true }],
];

for (const [name, change, expected] of pageSettingCases) {
    test(`Formatted PDF sends the exact page settings chosen with ${name}`, async ({ page }) => {
        const settings = page.getByRole("complementary", { name: "Document settings" });
        expect(await pageSentAfter(page, async () => { await change(settings); })).toEqual({ ...defaultPage, ...expected });
    });
}

test("Formatted PDF summarises the page settings, explains each margin and resets them for a new document", async ({ page }) => {
    await openFormatted(page);
    const settings = page.getByRole("complementary", { name: "Document settings" });
    await expect(settings.getByLabel("Size", { exact: true })).toHaveValue("a4");
    await expect(settings.getByLabel("Size", { exact: true }).locator("option")).toHaveText(["A4, 210 × 297 mm", "Letter, 8.5 × 11 in", "Legal, 8.5 × 14 in"]);
    await expect(settings.getByRole("group", { name: "Page" })).toContainText("A4, portrait, normal margins");
    await expect(settings).toContainText("Normal is 25 mm on every side.");
    await expect(settings.getByLabel("Footer text")).toHaveAttribute("placeholder", "None");

    await settings.getByLabel("Size", { exact: true }).selectOption("letter");
    await settings.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Landscape" }).click();
    await settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Wide" }).click();
    await expect(settings.getByRole("group", { name: "Page" })).toContainText("Letter, landscape, wide margins");
    await expect(settings).toContainText("Wide is 38 mm on every side.");
    await settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Narrow" }).click();
    await expect(settings).toContainText("Narrow is 13 mm on every side.");

    const bands = settings.getByRole("group", { name: "Header and footer" });
    await expect(bands).toContainText("None");
    await settings.getByLabel("Header text").fill("Flat 4B handover");
    await settings.getByLabel("Page numbers at the bottom").check();
    await expect(bands).toContainText("Header, page numbers");

    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");
    await saveButton(page).click();
    const save = page.getByRole("dialog", { name: "Save the PDF" });
    await expect(save).toContainText("Letter landscape, Liberation Serif 11 pt");
    await save.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => invocation(page, "create_formatted_pdf")).toMatchObject({
        document: { page: { pageSize: "letter", orientation: "landscape", margins: "narrow", header: "Flat 4B handover", footer: "", pageNumbers: true } },
    });
    await page.evaluate(() => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Untitled.pdf", outputPaths: ["/Users/test/Documents/Untitled.pdf"], detail: "1 page, Letter landscape.",
        state: "succeeded", failure: null, reason: null, warnings: [], fonts: [],
    }]));
    await expect(resultRows(page)).toHaveCount(1);
    await page.getByRole("button", { name: "New document" }).click();
    await expect(settings.getByRole("group", { name: "Page" })).toContainText("A4, portrait, normal margins");
    await expect(settings.getByLabel("Header text")).toHaveValue("");
    await expect(settings.getByLabel("Page numbers at the bottom")).not.toBeChecked();
});

test("Formatted PDF proposes Untitled in the last-used save folder, and cancelling saves nothing", async ({ page }) => {
    await openFormatted(page);
    await page.evaluate(() => { (window as unknown as FormattedWindow).__lastSaveFolder = "/Users/test/Letters"; });
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");

    await saveButton(page).click();
    const save = page.getByRole("dialog", { name: "Save the PDF" });
    await expect(save.getByTestId("save-folder")).toHaveText("~/Letters");
    await expect(save.getByLabel("File name")).toHaveValue("Untitled.pdf");
    await save.getByRole("button", { name: "Cancel" }).click();
    await expect(save).toHaveCount(0);
    expect(await invocation(page, "create_formatted_pdf")).toBeUndefined();

    await saveButton(page).click();
    await save.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => invocation(page, "create_formatted_pdf")).toMatchObject({ outputPath: "/Users/test/Letters/Untitled.pdf", document: { title: "", author: "" } });
});

test("Formatted PDF splits pasted paragraphs into blocks, reorders them and leaves blank blocks out", async ({ page }) => {
    await openFormatted(page);
    const first = block(page, "Block 1, Paragraph");
    await first.focus();
    await page.evaluate(() => {
        const data = new DataTransfer();
        data.setData("text/plain", "First part.\n\nSecond part.\r\n\r\nThird part.");
        document.activeElement?.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    });
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).getByRole("textbox")).toHaveCount(3);
    await expect(block(page, "Block 3, Paragraph")).toHaveText("Third part.");

    await page.getByRole("button", { name: "Move block 3 up" }).click();
    await expect(block(page, "Block 2, Paragraph")).toHaveText("Third part.");
    await block(page, "Block 1, Paragraph").fill("");
    await page.getByRole("button", { name: "Delete block 3" }).click();
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).getByRole("textbox")).toHaveCount(2);

    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => ((await invocation(page, "create_formatted_pdf")) as { document: { blocks: unknown } } | undefined)?.document.blocks)
        .toEqual([{ kind: "paragraph", runs: [plain("Third part.")] }]);
});

test("Formatted PDF sends bold and italic runs set with the shortcuts, the toolbar and a selection", async ({ page }) => {
    await openFormatted(page);
    const paragraph = block(page, "Block 1, Paragraph");
    const bold = page.getByRole("button", { name: "Bold" });
    const italic = page.getByRole("button", { name: "Italic" });
    await expect(bold).toHaveAttribute("aria-keyshortcuts", "Meta+B");
    await expect(italic).toHaveAttribute("aria-keyshortcuts", "Meta+I");

    await paragraph.click();
    await paragraph.pressSequentially("Return ");
    await page.keyboard.press("Meta+B");
    await expect(bold).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.type("every key");
    await page.keyboard.press("Meta+B");
    await page.keyboard.type(" and ");
    await italic.click();
    await expect(paragraph).toBeFocused();
    await page.keyboard.type("photograph");
    await italic.click();
    await page.keyboard.type(" each room.");
    // A selection takes the style too: "each room" becomes italic.
    await page.keyboard.press("ArrowLeft");
    for (let index = 0; index < "each room".length; index += 1) await page.keyboard.press("Shift+ArrowLeft");
    await page.keyboard.press("Meta+I");
    await expect(paragraph.locator("i, em")).toHaveText(["photograph", "each room"]);

    // Headings carry text only, so emphasis is off there.
    await caretAt(page, "end");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Kitchen");
    await page.keyboard.press("Meta+Alt+2");
    await expect(block(page, "Block 2, Heading 2")).toBeFocused();
    await expect(bold).toBeDisabled();
    await page.keyboard.press("Meta+B");
    await expect(block(page, "Block 2, Heading 2").locator("b, strong")).toHaveCount(0);

    expect(await savedBlocks(page)).toEqual([
        { kind: "paragraph", runs: [plain("Return "), run("every key", true, false), plain(" and "), run("photograph", false, true), plain(" "), run("each room", false, true), plain(".")] },
        { kind: "heading2", text: "Kitchen" },
    ]);
});

test("Formatted PDF makes bulleted and numbered lists, ends them on an empty item and splits a numbered list with a page break", async ({ page }) => {
    await openFormatted(page);
    const first = block(page, "Block 1, Paragraph");
    await first.click();
    await first.pressSequentially("Keys");
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type("Meters");
    // Each line becomes an item.
    await page.keyboard.press("Meta+Shift+8");
    await expect(page.getByRole("listitem", { name: "Block 1, Bulleted list, 2 items" })).toBeVisible();
    await expect(block(page, "Block 1, Bulleted list, item 2")).toBeFocused();
    await expect(page.getByRole("button", { name: "Bulleted list" })).toHaveAttribute("aria-pressed", "true");
    await caretAt(page, "end");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Post");
    await page.keyboard.press("Enter");
    // Enter on an empty item ends the list with a new paragraph.
    await page.keyboard.press("Enter");
    const after = block(page, "Block 2, Paragraph");
    await expect(after).toBeFocused();
    await expect(page.getByRole("listitem", { name: "Block 1, Bulleted list, 3 items" })).toBeVisible();

    await after.pressSequentially("Pack the van");
    await page.getByRole("button", { name: "Numbered list" }).click();
    const step = block(page, "Block 2, Numbered list, item 1");
    await expect(step).toBeFocused();
    await caretAt(page, "end");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Drive");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Unload");
    // A page break between items splits the list, and the numbers carry on after it.
    await caretAt(page, "start");
    await page.keyboard.press("Meta+Shift+Enter");
    await expect(saveButton(page)).toBeEnabled();
    await expect(page.getByRole("dialog", { name: "Save the PDF" })).toHaveCount(0);
    await expect(page.getByRole("group", { name: "Block 3, Page break" })).toBeVisible();
    await expect(block(page, "Block 4, Numbered list, item 1")).toBeFocused();
    await expect(page.getByRole("listitem", { name: "Block 4, Numbered list, 1 item" }).locator("ol")).toHaveAttribute("start", "3");

    expect(await savedBlocks(page)).toEqual([
        { kind: "bulletedList", items: [{ runs: [plain("Keys")] }, { runs: [plain("Meters")] }, { runs: [plain("Post")] }] },
        { kind: "numberedList", items: [{ runs: [plain("Pack the van")] }, { runs: [plain("Drive")] }] },
        { kind: "pageBreak" },
        { kind: "numberedList", items: [{ runs: [plain("Unload")] }] },
    ]);
});

test("Formatted PDF inserts page breaks from the toolbar, changes block types by shortcut and moves blocks", async ({ page }) => {
    await openFormatted(page);
    const first = block(page, "Block 1, Paragraph");
    await first.click();
    await first.pressSequentially("Inventory");
    await page.keyboard.press("Meta+Alt+1");
    await expect(page.getByLabel("Block type")).toHaveValue("heading1");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Living room.");
    await page.getByRole("button", { name: "Page break" }).click();
    await expect(page.getByRole("group", { name: "Block 3, Page break" })).toBeVisible();
    const bedroom = block(page, "Block 4, Paragraph");
    await expect(bedroom).toBeFocused();
    await page.keyboard.type("Bedroom.");
    await page.keyboard.press("Meta+Alt+3");
    await page.keyboard.press("Meta+Alt+0");
    await expect(block(page, "Block 4, Paragraph")).toBeFocused();

    // Option+Shift+Up moves the block, and the caret stays in it.
    await page.keyboard.press("Alt+Shift+ArrowUp");
    await expect(block(page, "Block 3, Paragraph")).toBeFocused();
    await expect(block(page, "Block 3, Paragraph")).toHaveText("Bedroom.");
    await page.keyboard.press("Alt+Shift+ArrowDown");
    await expect(block(page, "Block 4, Paragraph")).toHaveText("Bedroom.");

    // A page break block is removed with Delete.
    await page.getByRole("group", { name: "Block 3, Page break" }).focus();
    await expect(page.getByLabel("Block type")).toBeDisabled();
    await page.keyboard.press("Meta+Shift+Enter");
    await expect(page.getByRole("group", { name: "Block 4, Page break" })).toBeFocused();
    await page.keyboard.press("Delete");
    await expect(page.getByRole("group", { name: /Page break/ })).toHaveCount(1);

    expect(await savedBlocks(page)).toEqual([
        { kind: "heading1", text: "Inventory" },
        { kind: "paragraph", runs: [plain("Living room.")] },
        { kind: "pageBreak" },
        { kind: "paragraph", runs: [plain("Bedroom.")] },
    ]);
});

test("Formatted PDF format keys stay out of other text fields and IME composition, and the run key saves", async ({ page }) => {
    await openFormatted(page);
    const paragraph = block(page, "Block 1, Paragraph");
    await paragraph.click();
    await paragraph.pressSequentially("Keys");

    // In the Title field the format keys do nothing to the document.
    const title = page.getByLabel("Title", { exact: true });
    await title.fill("Checklist");
    await title.press("Meta+B");
    await title.press("Meta+Shift+8");
    await title.press("Meta+Alt+1");
    await expect(block(page, "Block 1, Paragraph")).toHaveText("Keys");
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).locator(":scope > li")).toHaveCount(1);
    await expect(title).toHaveValue("Checklist");

    // While an input method composes text, the same keys are left to it.
    await paragraph.click();
    for (const how of ["isComposing", "keyCode229"] as const) {
        await composingKey(page, { key: "*", code: "Digit8", metaKey: true, shiftKey: true }, how);
        await composingKey(page, { key: "Enter", code: "Enter", metaKey: true, shiftKey: true }, how);
        await composingKey(page, { key: "Enter", code: "Enter" }, how);
    }
    await expect(block(page, "Block 1, Paragraph")).toHaveText("Keys");
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).locator(":scope > li")).toHaveCount(1);
    // The same key sent outside composition does apply.
    await page.evaluate(() => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "*", code: "Digit8", metaKey: true, shiftKey: true, bubbles: true, cancelable: true })));
    await expect(block(page, "Block 1, Bulleted list, item 1")).toHaveText("Keys");

    // Command+Return saves from inside a block.
    await page.keyboard.press("Meta+Enter");
    const save = page.getByRole("dialog", { name: "Save the PDF" });
    await expect(save.getByLabel("File name")).toHaveValue("Checklist.pdf");
    await save.getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => ((await invocation(page, "create_formatted_pdf")) as { document: { blocks: unknown } } | undefined)?.document.blocks)
        .toEqual([{ kind: "bulletedList", items: [{ runs: [plain("Keys")] }] }]);
});

test("Formatted PDF adds and edits a table and sends its rows, leaving blank rows out, with the header-row flag", async ({ page }) => {
    await openFormatted(page);
    await block(page, "Block 1, Paragraph").fill("Condition of each room.");
    await block(page, "Block 1, Paragraph").click();
    await page.getByRole("button", { name: "Table", exact: true }).click();
    const table = page.getByRole("listitem", { name: "Block 2, Table, 3 columns, header row" });
    await expect(table).toBeVisible();
    await expect(table.getByRole("columnheader")).toHaveCount(3);
    const cell = (row: number, column: number) => block(page, `Block 2, Table, row ${row}, column ${column}`);
    await expect(cell(1, 1)).toBeFocused();
    await expect(page.getByLabel("Block type")).toBeDisabled();
    await expect(page.getByRole("button", { name: "Bulleted list" })).toBeDisabled();

    for (const [row, column, text] of [[1, 1, "Room"], [1, 2, "Condition"], [1, 3, "Notes"], [2, 1, "Kitchen"], [2, 2, "Good"], [2, 3, "Oven cleaned"]] as const) {
        await cell(row, column).click();
        await page.keyboard.type(text);
    }
    // Enter and Shift+Enter keep a cell to one line.
    await page.keyboard.press("Enter");
    await page.keyboard.press("Shift+Enter");
    await expect(cell(2, 3)).toHaveText("Oven cleaned");
    await expect(page.getByRole("listitem", { name: /^Block 3/ })).toHaveCount(0);

    await page.getByRole("button", { name: "Add a column to block 2" }).click();
    await expect(page.getByRole("listitem", { name: "Block 2, Table, 4 columns, header row" })).toBeVisible();
    await expect(cell(1, 4)).toBeFocused();
    await page.keyboard.type("Photo");
    await page.getByRole("button", { name: "Add a row to block 2" }).click();
    await expect(cell(4, 1)).toBeFocused();
    await page.keyboard.type("Bathroom");
    await cell(4, 2).click();
    await page.keyboard.type("Fair");
    // Pasted lines join into one line in a cell.
    await cell(4, 3).click();
    await page.evaluate(() => {
        const data = new DataTransfer();
        data.setData("text/plain", "Grout\nstained");
        document.activeElement?.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    });
    await expect(cell(4, 3)).toHaveText("Grout stained");

    // Without a header row, the first row is an ordinary row.
    await page.getByRole("checkbox", { name: "Header row" }).uncheck();
    await expect(page.getByRole("listitem", { name: "Block 2, Table, 4 columns" })).toBeVisible();
    await expect(table.getByRole("columnheader")).toHaveCount(0);
    await page.getByRole("checkbox", { name: "Header row" }).check();
    await expect(page.getByRole("listitem", { name: "Block 2, Table, 4 columns, header row" }).getByRole("columnheader")).toHaveCount(4);

    // The table's controls show while the caret is in it.
    await block(page, "Block 1, Paragraph").click();
    await expect(page.getByRole("checkbox", { name: "Header row" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Add a row to block 2" })).toHaveCount(0);
    await cell(2, 2).click();
    await expect(page.getByRole("checkbox", { name: "Header row" })).toBeChecked();

    expect(await savedBlocks(page)).toEqual([
        { kind: "paragraph", runs: [plain("Condition of each room.")] },
        { kind: "table", headerRow: true, rows: [["Room", "Condition", "Notes", "Photo"], ["Kitchen", "Good", "Oven cleaned", ""], ["Bathroom", "Fair", "Grout stained", ""]] },
    ]);
});

test("Formatted PDF sends a table without a header row, and drops blank columns", async ({ page }) => {
    await openFormatted(page);
    await block(page, "Block 1, Paragraph").click();
    await page.getByRole("button", { name: "Table", exact: true }).click();
    // A table typed into an empty paragraph takes its place.
    await expect(page.getByRole("listitem", { name: "Block 1, Table, 3 columns, header row" })).toBeVisible();
    await page.getByRole("checkbox", { name: "Header row" }).uncheck();
    for (const [row, column, text] of [[1, 1, "Kitchen"], [1, 3, "Oven cleaned"], [3, 1, "Bathroom"]] as const) {
        await block(page, `Block 1, Table, row ${row}, column ${column}`).click();
        await page.keyboard.type(text);
    }
    expect(await savedBlocks(page)).toEqual([
        { kind: "table", headerRow: false, rows: [["Kitchen", "Oven cleaned"], ["Bathroom", ""]] },
    ]);
});

const imageBlock = (page: Page, label: string) => page.getByRole("list", { name: "Document blocks, in order" }).getByRole("group", { name: label });

/** Picks `path` in the native file picker that Image… opens. */
const pickImage = async (page: Page, path: string) => {
    await page.evaluate((path) => { (window as unknown as FormattedWindow).__dialogResults.push(path); }, path);
    await page.getByRole("button", { name: "Image…" }).click();
};

/** Drops files from Finder on the window, at the middle of the upper or lower half of `target`. */
const dropOn = async (page: Page, paths: string[], target: ReturnType<Page["locator"]>, half: "upper" | "lower") => {
    await expect.poll(() => page.evaluate(() => (window as unknown as FormattedWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
    const box = (await target.boundingBox())!;
    const point = { x: box.x + box.width / 2, y: box.y + box.height * (half === "upper" ? 0.25 : 0.75) };
    await page.evaluate(({ paths, point }) => (window as unknown as FormattedWindow).__drop(paths, point), { paths, point });
};

const listItem = (page: Page, label: string) => page.getByRole("list", { name: "Document blocks, in order" }).locator(":scope > li").filter({ has: page.getByLabel(label, { exact: true }) });

test("Formatted PDF adds images from the picker and by dropping them where they land, and sends each with its path", async ({ page }) => {
    await openFormatted(page);
    await expect(page.getByText("Drop an image from Finder to add it where you drop.")).toBeVisible();
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");

    await pickImage(page, "/Users/test/Pictures/floorplan.png");
    const picked = imageBlock(page, "Block 2, Image, floorplan.png");
    await expect(picked).toBeFocused();
    await expect(picked).toContainText("Fits the page width. Read from your disk when you save.");
    const dialog = await page.evaluate(() => (window as unknown as FormattedWindow).__invocations.findLast(({ command }) => command === "plugin:dialog|open")?.args);
    expect(dialog).toMatchObject({ options: { multiple: false, filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "heic", "heif", "tif", "tiff"] }] } });
    await expect(page.getByLabel("Block type")).toBeDisabled();
    await expect(page.getByLabel("Block type")).toHaveValue("image");

    // Cancelling the picker adds nothing.
    await page.getByRole("button", { name: "Image…" }).click();
    await expect(page.getByRole("group", { name: /, Image, / })).toHaveCount(1);

    // A drop on the upper half of a block goes before it, on the lower half after it, and files that aren't images are left out.
    await dropOn(page, ["/Users/test/Pictures/Site plan.JPG", "/Users/test/notes.txt"], listItem(page, "Block 1, Paragraph"), "upper");
    await expect(imageBlock(page, "Block 1, Image, Site plan.JPG")).toBeFocused();
    await dropOn(page, ["/Users/test/Pictures/porch.heic"], listItem(page, "Block 2, Paragraph"), "lower");
    await expect(imageBlock(page, "Block 3, Image, porch.heic")).toBeVisible();
    await dropOn(page, ["/Users/test/notes.txt"], listItem(page, "Block 2, Paragraph"), "lower");
    await expect(page.getByRole("group", { name: /, Image, / })).toHaveCount(3);

    // Delete on a focused image removes its block.
    await imageBlock(page, "Block 3, Image, porch.heic").focus();
    await page.keyboard.press("Delete");
    await expect(page.getByRole("group", { name: /, Image, / })).toHaveCount(2);

    expect(await savedBlocks(page)).toEqual([
        { kind: "image", path: "/Users/test/Pictures/Site plan.JPG" },
        { kind: "paragraph", runs: [plain("Dear neighbour,")] },
        { kind: "image", path: "/Users/test/Pictures/floorplan.png" },
    ]);
});

test("Formatted PDF replaces an empty paragraph with a picked image, and an image alone can be saved", async ({ page }) => {
    await openFormatted(page);
    await expect(saveButton(page)).toBeDisabled();
    await pickImage(page, "C:\\Users\\test\\Pictures\\floorplan.png");
    await expect(imageBlock(page, "Block 1, Image, floorplan.png")).toBeVisible();
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).locator(":scope > li")).toHaveCount(1);
    expect(await savedBlocks(page)).toEqual([{ kind: "image", path: "C:\\Users\\test\\Pictures\\floorplan.png" }]);
});

test("Formatted PDF names a missing image, keeps the draft and points to the block to fix", async ({ page }) => {
    const path = "/Users/test/Pictures/floorplan.png";
    const missing = (outputPath: string) => [{
        inputPath: outputPath, state: "failed", outputPaths: [], detail: "", reason: null, warnings: [],
        failure: { kind: "invalidInput", message: `floorplan.png is missing. It was at ${path}. Nothing was written. Choose the image again or remove its block, then save.`, pointer: null },
    }];
    await openFormatted(page);
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");
    await pickImage(page, path);
    await savedBlocks(page);
    await page.evaluate((outcomes) => (window as unknown as FormattedWindow).__finish(outcomes), missing("/Users/test/Documents/Untitled.pdf"));

    await expect(page.getByRole("alert")).toContainText("The PDF wasn't saved: floorplan.png is missing");
    await expect(page.getByRole("alert")).toContainText(`It was at ${path} when you added it. Nothing was written and your draft is kept.`);
    await expect(page.getByRole("status").filter({ hasText: "Save failed, nothing written" })).toContainText("Fix the image block, then save again");
    const image = imageBlock(page, "Block 2, Image, floorplan.png");
    await expect(image).toContainText(`Not found at ${path}`);
    await expect(block(page, "Block 1, Paragraph")).toHaveText("Dear neighbour,");
    await expect(page.getByRole("list", { name: "Results" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "New document" })).toHaveCount(0);

    // Choosing the image again clears the error, and the next save sends the new path.
    await page.evaluate(() => { (window as unknown as FormattedWindow).__dialogResults.push("/Users/test/Desktop/floorplan.png"); });
    await image.getByRole("button", { name: "Choose image…" }).click();
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(imageBlock(page, "Block 2, Image, floorplan.png")).toContainText("Fits the page width.");
    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => ((await invocation(page, "create_formatted_pdf")) as { document: { blocks: unknown } }).document.blocks).toEqual([
        { kind: "paragraph", runs: [plain("Dear neighbour,")] },
        { kind: "image", path: "/Users/test/Desktop/floorplan.png" },
    ]);
    await page.evaluate((outcomes) => (window as unknown as FormattedWindow).__finish(outcomes), missing("/Users/test/Documents/Untitled.pdf").map((outcome) => ({
        ...outcome, failure: { ...outcome.failure, message: outcome.failure.message.replaceAll(path, "/Users/test/Desktop/floorplan.png") },
    })));
    await expect(page.getByRole("alert")).toContainText("The PDF wasn't saved: floorplan.png is missing");

    // Removing the block clears it too.
    await imageBlock(page, "Block 2, Image, floorplan.png").getByRole("button", { name: "Remove block" }).click();
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(page.getByRole("status").filter({ hasText: "Save failed" })).toHaveCount(0);
    await expect(page.getByRole("group", { name: /, Image, / })).toHaveCount(0);
});

const previewRequests = (page: Page) => page.evaluate(() =>
    (window as unknown as FormattedWindow).__invocations.filter(({ command }) => command === "preview_formatted_pdf").map(({ args }) => (args as { request: unknown }).request));

test("Formatted PDF previews the pages beside the editor and sends the current document again after each edit", async ({ page }) => {
    await openFormatted(page);
    const preview = page.getByRole("region", { name: "Preview" });
    const shownPage = preview.getByRole("img");
    // Nothing typed yet: a blank page and no preview request.
    await expect(preview.getByRole("img", { name: "Blank A4 portrait page" })).toBeVisible();
    await expect(preview.getByText("Nothing to preview yet. The page fills in as you type.")).toBeVisible();
    await expect(preview.getByText("1 of 1")).toBeVisible();
    expect(await previewRequests(page)).toEqual([]);

    // Typing quickly sends one request, once the typing pauses, with the document a save would send.
    await page.evaluate((preview) => { (window as unknown as FormattedWindow).__preview = preview; }, fakePreview([PAGE_ONE, PAGE_TWO]));
    await block(page, "Block 1, Paragraph").pressSequentially("Dear neighbour,");
    await expect(preview.getByRole("img", { name: "Page 1 of 2, A4 portrait" })).toBeVisible();
    expect(await previewRequests(page)).toEqual([{
        document: { title: "", author: "", blocks: [{ kind: "paragraph", runs: [plain("Dear neighbour,")] }], page: defaultPage, font: { family: "Liberation Serif", size: 11 } },
    }]);
    await expect(shownPage.locator("img")).toHaveAttribute("src", PAGE_ONE);
    await expect(page.getByText("2 pages, A4", { exact: true })).toBeVisible();
    await expect(preview.getByRole("button", { name: "Previous page" })).toBeDisabled();
    await preview.getByRole("button", { name: "Next page" }).click();
    await expect(preview.getByRole("img", { name: "Page 2 of 2, A4 portrait" }).locator("img")).toHaveAttribute("src", PAGE_TWO);
    await expect(preview.getByText("2 of 2")).toBeVisible();
    await expect(preview.getByRole("button", { name: "Next page" })).toBeDisabled();

    // An edit dims the shown page while a new request, with the current document, is drawn.
    await page.evaluate(() => { (window as unknown as FormattedWindow).__holdPreview = true; });
    await page.getByRole("radio", { name: "Landscape" }).click();
    await expect.poll(async () => (await previewRequests(page)).length).toBe(2);
    expect(((await previewRequests(page))[1] as { document: unknown }).document).toEqual({
        title: "", author: "", blocks: [{ kind: "paragraph", runs: [plain("Dear neighbour,")] }], page: { ...defaultPage, orientation: "landscape" }, font: { family: "Liberation Serif", size: 11 },
    });
    await expect(preview.getByRole("status")).toHaveText("Updating…");
    await expect(preview.getByText("Updating preview…")).toBeVisible();
    await expect(shownPage).toHaveAttribute("aria-busy", "true");

    // A second edit before the first reply: the older reply is dropped, so the pages are always the latest document's.
    await page.getByLabel("Title", { exact: true }).fill("Move-out checklist");
    await expect.poll(async () => (await previewRequests(page)).length).toBe(3);
    expect(((await previewRequests(page))[2] as { document: { title: string } }).document.title).toBe("Move-out checklist");
    await page.evaluate((latest) => (window as unknown as FormattedWindow).__heldPreviews[1](latest), fakePreview([PAGE_TWO], 1));
    await expect(preview.getByRole("img", { name: "Page 1 of 1, A4 landscape" }).locator("img")).toHaveAttribute("src", PAGE_TWO);
    await page.evaluate((stale) => (window as unknown as FormattedWindow).__heldPreviews[0](stale), fakePreview([PAGE_ONE, PAGE_ONE, PAGE_ONE]));
    await page.waitForTimeout(100);
    await expect(preview.getByRole("img", { name: "Page 1 of 1, A4 landscape" }).locator("img")).toHaveAttribute("src", PAGE_TWO);
    await expect(preview.getByRole("status")).toHaveText("");
    await expect(page.getByText("1 page, A4 landscape", { exact: true })).toBeVisible();

    // Clearing the document goes back to the blank page without asking for a preview.
    await block(page, "Block 1, Paragraph").fill("");
    await expect(preview.getByRole("img", { name: "Blank A4 landscape page" })).toBeVisible();
    await page.waitForTimeout(400);
    expect((await previewRequests(page)).length).toBe(3);
});

test("Formatted PDF says how many pages the preview leaves out and why a preview is unavailable", async ({ page }) => {
    await openFormatted(page);
    const preview = page.getByRole("region", { name: "Preview" });
    await page.evaluate((capped) => { (window as unknown as FormattedWindow).__preview = capped; }, fakePreview([PAGE_ONE, PAGE_TWO], 30));
    await block(page, "Block 1, Paragraph").fill("A long report");
    await expect(preview.getByRole("img", { name: "Page 1 of 30, A4 portrait" })).toBeVisible();
    await expect(preview.getByText("The preview shows the first 2 pages. Saving makes all 30.")).toBeVisible();
    await preview.getByRole("button", { name: "Next page" }).click();
    await expect(preview.getByRole("button", { name: "Next page" })).toBeDisabled();
    await expect(page.getByText("30 pages, A4", { exact: true })).toBeVisible();

    await page.evaluate(() => { (window as unknown as FormattedWindow).__preview = null; });
    await block(page, "Block 1, Paragraph").fill("A short note");
    await expect(preview.getByRole("status")).toHaveText("Preview unavailable");
    await expect(preview.getByText("The preview could not be drawn.")).toBeVisible();
    await expect(saveButton(page)).toBeEnabled();
});

const MB = 1024 * 1024;
const scriptPacks = (status: string) => ["Arabic", "Devanagari", "Han"].map((script) => ({ script, packId: "unicode-scripts", packName: "Unicode scripts", status, downloadBytes: 38 * MB }));
/** What the native preview rejects with when Arabic text needs the Unicode scripts pack. */
const arabicRefusal = {
    message: "Arabic text needs the Unicode scripts pack, which isn't installed. Nothing was saved.",
    script: "Arabic",
    pointer: { kind: "pack", packId: "unicode-scripts", name: "Unicode scripts", reason: "notInstalled" },
};

test("Formatted PDF pauses the preview for text whose script pack is missing, offers the install and clears once it's installed", async ({ page }) => {
    await page.addInitScript((states) => { (window as unknown as FormattedWindow).__scriptPacks = states; }, scriptPacks("notInstalled"));
    await openFormatted(page);
    const preview = page.getByRole("region", { name: "Preview" });
    await block(page, "Block 1, Paragraph").fill("Receipt");
    await expect(preview.getByRole("img", { name: "Page 1 of 1, A4 portrait" })).toBeVisible();
    await expect(saveButton(page)).toBeEnabled();

    // The native preview refuses the Arabic text before drawing anything; the last page stays, dimmed and paused.
    await page.evaluate((refusal) => { (window as unknown as FormattedWindow).__previewError = refusal; }, arabicRefusal);
    await block(page, "Block 1, Paragraph").focus();
    await caretAt(page, "end");
    await page.keyboard.press("Enter");
    await block(page, "Block 2, Paragraph").fill("مرحبا بكم");
    const note = preview.getByRole("status").filter({ hasText: "Arabic text needs the Unicode scripts pack" });
    await expect(note).toContainText("Install it to preview and save block 2, or remove that text. Nothing downloads until you choose Install.");
    await expect(note).toContainText("38 MB download");
    await expect(preview.getByRole("status").first()).toHaveText("Paused");
    await expect(preview.getByText("Preview paused")).toBeVisible();
    await expect(preview.getByRole("img", { name: "Page 1 of 1, A4 portrait" })).toHaveAttribute("data-updating", "true");
    await expect(preview.getByText(arabicRefusal.message)).toHaveCount(0);
    const blocks = page.getByRole("list", { name: "Document blocks, in order" });
    await expect(blocks.getByRole("listitem", { name: "Block 2, Paragraph, has Arabic text that needs the Unicode scripts pack" }))
        .toContainText("Arabic text needs the Unicode scripts pack to be drawn in the PDF.");
    await expect(blocks.locator("[data-script-pack]")).toHaveCount(1);
    const footer = page.getByRole("status").filter({ hasText: "Can't save yet" });
    await expect(footer).toContainText("Install the script pack, or remove the Arabic text");
    await expect(saveButton(page)).toBeDisabled();
    expect((await page.evaluate(() => (window as unknown as FormattedWindow).__invocations.map(({ command }) => command))).includes("create_formatted_pdf")).toBe(false);

    // Install asks for the pack the refusal named, and nothing else.
    await note.getByRole("button", { name: "Install script pack", exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as unknown as FormattedWindow).__invocations
        .filter(({ command }) => command === "install_pack").map(({ args }) => (args as { packId: string }).packId))).toEqual(["unicode-scripts"]);

    // The native install announces the pack; the query reports it installed, the prompt clears and the preview is drawn again, with no restart.
    const before = (await previewRequests(page)).length;
    await page.evaluate((states) => {
        const win = window as unknown as FormattedWindow;
        win.__scriptPacks = states;
        win.__previewError = null;
        win.__emit("pack-changed", { packId: "unicode-scripts" });
    }, scriptPacks("installed"));
    await expect(note).toHaveCount(0);
    await expect(footer).toHaveCount(0);
    await expect(blocks.locator("[data-script-pack]")).toHaveCount(0);
    await expect.poll(async () => (await previewRequests(page)).length).toBeGreaterThan(before);
    await expect(preview.getByText("Preview paused")).toHaveCount(0);
    await expect(preview.getByRole("img", { name: "Page 1 of 1, A4 portrait" })).not.toHaveAttribute("data-updating");
    await expect(saveButton(page)).toBeEnabled();
});

test("Formatted PDF names the document instead of a block when the refused text is in no block, as in the header", async ({ page }) => {
    await page.addInitScript((states) => { (window as unknown as FormattedWindow).__scriptPacks = states; }, scriptPacks("notInstalled"));
    await openFormatted(page);
    await page.evaluate((refusal) => { (window as unknown as FormattedWindow).__previewError = refusal; }, arabicRefusal);
    await block(page, "Block 1, Paragraph").fill("Receipt");
    const note = page.getByRole("region", { name: "Preview" }).getByRole("status").filter({ hasText: "Arabic text needs the Unicode scripts pack" });
    await expect(note).toContainText("Install it to preview and save the document, or remove that text.");
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).locator("[data-script-pack]")).toHaveCount(0);
    await expect(saveButton(page)).toBeDisabled();
});

const fontSent = async (page: Page, command: "create_formatted_pdf" | "preview_formatted_pdf") =>
    ((await invocation(page, command)) as { document: { font: unknown } } | undefined)?.document.font;

const fontUse = (requestedFamily: string, requestedStyle: string, fontUsed: string, styleUsed: string, source: string) => ({ requestedFamily, requestedStyle, fontUsed, styleUsed, source });

test("Formatted PDF lists bundled fonts first, marks installed ones as this computer only, and sends the chosen family and size", async ({ page }) => {
    await openFormatted(page);
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");
    const fontSettings = page.getByRole("group", { name: "Font" });
    await expect(fontSettings).toContainText("Liberation Serif, 11 pt");
    await expect(fontSettings).toContainText("Headings scale from the base size. Bold and italic use the same family.");
    await expect.poll(() => fontSent(page, "preview_formatted_pdf")).toEqual({ family: "Liberation Serif", size: 11 });

    await page.getByLabel("Family").click();
    const list = page.getByRole("listbox");
    await expect(list.getByRole("option")).toHaveText(["Liberation Sans", "Liberation Serif", "Liberation Mono", "Carlito", "Caladea", "DejaVu Sans", "CalibriThis computer only", "GeorgiaThis computer only"]);
    await expect(list.getByRole("group", { name: "Bundled with Toolbox" }).getByRole("option")).toHaveCount(6);
    const here = list.getByRole("group", { name: "On this computer" });
    await expect(here.getByRole("option", { name: "Georgia, this computer only" })).toBeVisible();
    await expect(here.getByRole("option", { name: "Calibri, this computer only" })).toBeVisible();
    await expect(list.getByRole("group", { name: "Bundled with Toolbox" }).getByText("This computer only")).toHaveCount(0);
    await expect(list.getByRole("option", { name: "Liberation Serif" })).toHaveAttribute("aria-selected", "true");
    await expect(list).toContainText("Bundled fonts look the same on every computer. Fonts from this computer can space text differently on Windows.");
    await here.getByRole("option", { name: "Georgia, this computer only" }).click();
    await expect(page.getByLabel("Family")).toHaveText("Georgia");

    const size = page.getByRole("spinbutton", { name: "Base size in points" });
    await expect(size).toHaveAttribute("min", "8");
    await expect(size).toHaveAttribute("max", "24");
    await size.fill("14");
    await expect(fontSettings).toContainText("Georgia, 14 pt");
    await expect.poll(() => fontSent(page, "preview_formatted_pdf")).toEqual({ family: "Georgia", size: 14 });
    // A size outside the range is not sent, and leaving the field puts back the size in use.
    await size.fill("40");
    await size.blur();
    await expect(size).toHaveValue("14");
    await expect(fontSettings).toContainText("Georgia, 14 pt");

    await page.getByLabel("Page numbers at the bottom").check();
    await saveButton(page).click();
    const dialog = page.getByRole("dialog", { name: "Save the PDF" });
    await expect(dialog).toContainText("A4, Georgia 14 pt");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => fontSent(page, "create_formatted_pdf")).toEqual({ family: "Georgia", size: 14 });
    expect(await fontSent(page, "preview_formatted_pdf")).toEqual({ family: "Georgia", size: 14 });

    await page.evaluate((fonts) => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Untitled.pdf", outputPaths: ["/Users/test/Documents/Untitled.pdf"], detail: "1 page, A4.",
        state: "succeeded", failure: null, reason: null, warnings: [], fonts,
    }]), [fontUse("Georgia", "regular", "Georgia", "regular", "system"), fontUse("Georgia", "bold", "Georgia", "bold", "system"), fontUse("Georgia", "italic", "Georgia", "italic", "system")]);
    const used = page.getByRole("table", { name: "Fonts used" });
    await expect(used.getByRole("columnheader")).toHaveText(["You chose", "Used in the PDF", "From"]);
    await expect(used.getByRole("row").nth(1).getByRole("cell")).toHaveText(["Georgia", "Georgia Regular, Bold, Italic", "This computerInstalled on this computer"]);
    await expect(used.getByRole("row").nth(2).getByRole("cell")).toHaveText(["Page numbers", "Georgia", "Same as the text"]);
    const note = page.getByRole("note").filter({ hasText: "Georgia came from this computer" });
    await expect(note).toContainText("Saving the same document on Windows can space or wrap text slightly differently. Pick a bundled font when the PDF must match everywhere.");
});

test("Formatted PDF names the bundled font that stood in for a missing one and why", async ({ page }) => {
    await openFormatted(page);
    await block(page, "Block 1, Paragraph").fill("Dear neighbour,");
    await page.getByLabel("Family").click();
    await page.getByRole("option", { name: "Calibri, this computer only" }).click();
    await expect.poll(() => fontSent(page, "preview_formatted_pdf")).toEqual({ family: "Calibri", size: 11 });
    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => fontSent(page, "create_formatted_pdf")).toEqual({ family: "Calibri", size: 11 });
    await page.evaluate((fonts) => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Untitled.pdf", outputPaths: ["/Users/test/Documents/Untitled.pdf"], detail: "1 page, A4.",
        state: "succeeded", failure: null, reason: null, warnings: [], fonts,
    }]), [fontUse("Calibri", "regular", "Carlito", "regular", "bundled fallback")]);
    const used = page.getByRole("table", { name: "Fonts used" });
    await expect(used.getByRole("row")).toHaveCount(2);
    await expect(used.getByRole("row").nth(1).getByRole("cell")).toHaveText(["Calibri", "Carlito Regular", "Bundled with ToolboxStands in for Calibri, which isn't installed here"]);
    await expect(page.getByRole("note").filter({ hasText: "Calibri isn't installed, so Carlito was used" }))
        .toContainText("Carlito comes with Toolbox and has the same letter widths, so lines break in the same places. Bundled fonts give the same PDF on every computer.");
    await expect(page.getByRole("note").filter({ hasText: "came from this computer" })).toHaveCount(0);

    await page.getByRole("button", { name: "New document" }).click();
    await expect(page.getByRole("group", { name: "Font" })).toContainText("Liberation Serif, 11 pt");
});

const storedDraft = (page: Page) => page.evaluate((key) => {
    const stored = sessionStorage.getItem(key);
    return stored ? (JSON.parse(stored) as { document: { title: string; blocks: unknown[] } }) : null;
}, DRAFT_KEY);

const draftStatus = (page: Page) => page.getByRole("status").filter({ hasText: /draft/i }).first();

const leaveAndReopen = async (page: Page) => {
    await page.getByRole("button", { name: "← All tools" }).click();
    await openToolBySearch(page, "Lock & Unlock");
    await page.getByRole("button", { name: "← All tools" }).click();
    await openToolBySearch(page, "Formatted PDF");
};

const expectChecklist = async (page: Page) => {
    await expect(block(page, "Block 1, Heading 1")).toHaveText("Move-out checklist");
    await expect(block(page, "Block 2, Paragraph")).toHaveText("Return the keys.\nRead the meters.");
};

test("the draft is autosaved on this device and restored after leaving the tool and after a reload", async ({ page }) => {
    await openFormatted(page);
    await expect(draftStatus(page)).toHaveText("No draft yet");
    await writeChecklist(page);
    await page.getByLabel("Page numbers at the bottom").check();
    await expect(draftStatus(page)).toHaveText("Draft saved on this device");
    await expect.poll(async () => (await storedDraft(page))?.document.blocks.length).toBe(2);

    // Leaving straight after typing still keeps what was typed.
    await page.getByLabel("Title", { exact: true }).fill("  Move-out checklist ");
    await leaveAndReopen(page);
    await expectChecklist(page);
    await expect(page.getByLabel("Title", { exact: true })).toHaveValue("  Move-out checklist ");
    await expect(page.getByLabel("Page numbers at the bottom")).toBeChecked();
    const restored = page.getByRole("status").filter({ hasText: "Restored your draft from" });
    await expect(restored).toContainText(/Restored your draft from today at \d\d:\d\d/);
    await expect(restored).toContainText("It stays on this device until you save the PDF or discard it. Nothing is sent anywhere.");
    await expect(draftStatus(page)).toHaveText("Draft saved on this device");
    await restored.getByRole("button", { name: "Dismiss restored-draft note" }).click();
    await expect(restored).toHaveCount(0);

    await page.reload();
    await openToolBySearch(page, "Formatted PDF");
    await expectChecklist(page);
    await expect(page.getByLabel("Title", { exact: true })).toHaveValue("  Move-out checklist ");
    await expect(page.getByRole("status").filter({ hasText: "Restored your draft from" })).toBeVisible();
    // Reading the draft back writes nothing.
    expect(await page.evaluate(() => (window as unknown as FormattedWindow).__invocations.filter(({ command }) => command === "save_formatted_draft").length)).toBe(0);
});

test("discarding the draft asks first, keeps it when cancelled and clears it when confirmed", async ({ page }) => {
    await openFormatted(page);
    await writeChecklist(page);
    await expect(draftStatus(page)).toHaveText("Draft saved on this device");

    const discard = page.locator(".formatted-pdf-footer").getByRole("button", { name: "Discard draft…" });
    const dialog = page.getByRole("alertdialog", { name: "Discard this draft?" });
    await discard.click();
    await expect(dialog).toContainText("This draft has 2 blocks and was last saved a moment ago. It will be removed from this device and can't be brought back. Files on your disk aren't touched.");
    await expect(dialog.getByRole("button", { name: /^Keep editing/ })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await discard.click();
    await dialog.getByRole("button", { name: /^Keep editing/ }).click();
    await expect(dialog).toHaveCount(0);
    await expectChecklist(page);
    expect(await storedDraft(page)).not.toBeNull();
    expect(await page.evaluate(() => (window as unknown as FormattedWindow).__invocations.some(({ command }) => command === "clear_formatted_draft"))).toBe(false);

    await discard.click();
    await dialog.getByRole("button", { name: "Discard draft", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect(page.getByRole("listitem").filter({ has: page.getByRole("textbox", { name: "Block 2, Paragraph" }) })).toHaveCount(0);
    await expect(draftStatus(page)).toHaveText("No draft yet");
    await expect.poll(() => storedDraft(page)).toBeNull();
    await expect(discard).toHaveCount(0);

    await leaveAndReopen(page);
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect(page.getByRole("status").filter({ hasText: "Restored your draft from" })).toHaveCount(0);
});

test("the restored note's discard clears the restored draft", async ({ page }) => {
    await openFormatted(page);
    await writeChecklist(page);
    await expect.poll(async () => (await storedDraft(page))?.document.blocks.length).toBe(2);
    await page.reload();
    await openToolBySearch(page, "Formatted PDF");
    await page.getByRole("status").filter({ hasText: "Restored your draft from" }).getByRole("button", { name: "Discard draft…" }).click();
    await page.getByRole("alertdialog", { name: "Discard this draft?" }).getByRole("button", { name: "Discard draft", exact: true }).click();
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect.poll(() => storedDraft(page)).toBeNull();
});

test("a saved PDF clears the draft, and a failed save keeps it", async ({ page }) => {
    await openFormatted(page);
    await writeChecklist(page);
    await expect(draftStatus(page)).toHaveText("Draft saved on this device");

    await savedBlocks(page);
    await page.evaluate(() => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Untitled.pdf", outputPaths: [], detail: null,
        state: "failed", failure: { kind: "processing", message: "The disk is full." }, reason: null, warnings: [], fonts: [],
    }]));
    await expect(resultRows(page)).toHaveCount(1);
    expect(await storedDraft(page)).not.toBeNull();
    await expect(draftStatus(page)).toHaveText("Draft saved on this device");

    await savedBlocks(page);
    await page.evaluate(() => (window as unknown as FormattedWindow).__finish([{
        inputPath: "/Users/test/Documents/Untitled.pdf", outputPaths: ["/Users/test/Documents/Untitled.pdf"], detail: "1 page, A4.",
        state: "succeeded", failure: null, reason: null, warnings: [], fonts: [],
    }]));
    await expect(draftStatus(page)).toHaveText("Draft cleared");
    await expect.poll(() => storedDraft(page)).toBeNull();
    const cleared = page.getByRole("region", { name: "Draft" });
    await expect(cleared).toContainText("The draft was cleared from this device now that the PDF is saved. The PDF is the copy to keep.");
    await expect(page.locator(".formatted-pdf-footer").getByRole("button", { name: "Discard draft…" })).toHaveCount(0);

    await leaveAndReopen(page);
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect(draftStatus(page)).toHaveText("No draft yet");
});

test("a damaged draft is reported and the tool opens on a new document", async ({ page }) => {
    await page.goto("/");
    await page.evaluate((key) => sessionStorage.setItem(key, "damaged"), DRAFT_KEY);
    await openToolBySearch(page, "Formatted PDF");
    const alert = page.getByRole("alert").filter({ hasText: "Your saved draft was damaged, so it was cleared." });
    await expect(alert).toBeVisible();
    await expect(block(page, "Block 1, Paragraph")).toHaveText("");
    await expect(draftStatus(page)).toHaveText("No draft yet");
    await alert.getByRole("button", { name: "Dismiss draft problem" }).click();
    await expect(alert).toHaveCount(0);
    expect(await storedDraft(page)).toBeNull();
});
