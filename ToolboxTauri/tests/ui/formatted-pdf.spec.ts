import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type FormattedWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __finish: (outcomes: unknown[]) => void;
    __lastSaveFolder?: string;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as FormattedWindow;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
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
                return null;
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
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

const saveButton = (page: Page) => page.getByRole("button", { name: /^Save PDF/ });

const resultRows = (page: Page) => page.getByRole("list", { name: "Results" }).getByRole("listitem").filter({ has: page.locator(".result-row-name") });

/** Types a heading, then a paragraph with a line break, and marks the first block as Heading 1. */
const writeChecklist = async (page: Page) => {
    await block(page, "Block 1, Paragraph").fill("Move-out checklist");
    await page.getByLabel("Block type").selectOption("heading1");
    await block(page, "Block 1, Heading 1").press("End");
    await block(page, "Block 1, Heading 1").press("Enter");
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
                { kind: "paragraph", text: "Return the keys.\nRead the meters." },
            ],
            page: { pageSize: "a4", margins: "normal" },
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
    await expect(block(page, "Block 1, Paragraph")).toHaveValue("");
    await expect(saveButton(page)).toBeDisabled();
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
    await expect(block(page, "Block 3, Paragraph")).toHaveValue("Third part.");

    await page.getByRole("button", { name: "Move block 3 up" }).click();
    await expect(block(page, "Block 2, Paragraph")).toHaveValue("Third part.");
    await block(page, "Block 1, Paragraph").fill("");
    await page.getByRole("button", { name: "Delete block 3" }).click();
    await expect(page.getByRole("list", { name: "Document blocks, in order" }).getByRole("textbox")).toHaveCount(2);

    await saveButton(page).click();
    await page.getByRole("dialog", { name: "Save the PDF" }).getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => ((await invocation(page, "create_formatted_pdf")) as { document: { blocks: unknown } } | undefined)?.document.blocks)
        .toEqual([{ kind: "paragraph", text: "Third part." }]);
});
