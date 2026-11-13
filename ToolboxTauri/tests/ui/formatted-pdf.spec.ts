import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type FormattedWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __finish: (outcomes: unknown[]) => void;
    __lastSaveFolder?: string;
    __dialogResults: Array<string | null>;
    __drop: (paths: string[], point?: { x: number; y: number }) => void;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as FormattedWindow;
        let finish: ((outcomes: unknown) => void) | null = null;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__drop = (paths, point) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths, point } });
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
    });
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
    await openFormatted(page);
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

test("Formatted PDF sends the exact page settings chosen with each page and header control", async ({ page }) => {
    const settings = page.getByRole("complementary", { name: "Document settings" });
    const cases: Array<[(page: Page) => Promise<void>, Record<string, unknown>]> = [
        [async () => undefined, {}],
        [() => settings.getByLabel("Size").selectOption("letter"), { pageSize: "letter" }],
        [() => settings.getByLabel("Size").selectOption("legal"), { pageSize: "legal" }],
        [() => settings.getByRole("radiogroup", { name: "Orientation" }).getByRole("radio", { name: "Landscape" }).click(), { orientation: "landscape" }],
        [() => settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Narrow" }).click(), { margins: "narrow" }],
        [() => settings.getByRole("radiogroup", { name: "Margins" }).getByRole("radio", { name: "Wide" }).click(), { margins: "wide" }],
        [() => settings.getByLabel("Header text").fill("Flat 4B handover"), { header: "Flat 4B handover" }],
        [() => settings.getByLabel("Footer text").fill("Confidential"), { footer: "Confidential" }],
        [() => settings.getByLabel("Page numbers at the bottom").check(), { pageNumbers: true }],
    ];
    for (const [change, expected] of cases) {
        expect(await pageSentAfter(page, change)).toEqual({ ...defaultPage, ...expected });
    }
});

test("Formatted PDF summarises the page settings, explains each margin and resets them for a new document", async ({ page }) => {
    await openFormatted(page);
    const settings = page.getByRole("complementary", { name: "Document settings" });
    await expect(settings.getByLabel("Size")).toHaveValue("a4");
    await expect(settings.getByLabel("Size").locator("option")).toHaveText(["A4, 210 × 297 mm", "Letter, 8.5 × 11 in", "Legal, 8.5 × 14 in"]);
    await expect(settings.getByRole("group", { name: "Page" })).toContainText("A4, portrait, normal margins");
    await expect(settings).toContainText("Normal is 25 mm on every side.");
    await expect(settings.getByLabel("Footer text")).toHaveAttribute("placeholder", "None");

    await settings.getByLabel("Size").selectOption("letter");
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
