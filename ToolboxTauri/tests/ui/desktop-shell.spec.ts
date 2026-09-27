import { expect, test, type Page } from "@playwright/test";
import path from "node:path";

const pdfPath = path.resolve("tests/fixtures/document.pdf");
const imagePath = path.resolve("src-tauri/icons/icon.png");
const preview = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/></svg>');

type ShellCommand = "search" | "open" | "undo" | "redo" | "export";
type ShellEvent =
    | { kind: "files"; activationId: string; workspace: "pdf-editor" | "image-editor"; paths: string[] }
    | { kind: "dropped-files"; paths: string[] }
    | { kind: "command"; command: ShellCommand }
    | { kind: "rejected-files"; paths: string[]; reason: string };

type TestWindow = Window & {
    __toolboxInvocations?: Array<{ command: string; args: unknown }>;
    __toolboxListeners?: Record<string, number>;
    __toolboxEmit?: (event: string, payload: unknown) => void;
    __toolboxDialogResults?: Array<string | string[] | null>;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ pdfPath, preview }) => {
        const invocations: Array<{ command: string; args: unknown }> = [];
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners: Record<string, number> = {};
        let nextCallbackId = 1;
        (window as TestWindow).__toolboxInvocations = invocations;
        (window as TestWindow).__toolboxListeners = listeners;
        (window as TestWindow).__toolboxEmit = (event, payload) => {
            const callbackId = listeners[event];
            if (callbackId) callbacks.get(callbackId)?.({ event, id: 1, payload });
        };
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: {
                currentWindow: { label: "main" },
                currentWebview: { label: "main", windowLabel: "main" },
            },
            transformCallback: (callback) => {
                const id = nextCallbackId++;
                callbacks.set(id, callback as (event: unknown) => void);
                return id;
            },
            unregisterCallback: (id) => callbacks.delete(id),
            invoke: async (command, args) => {
                invocations.push({ command, args });
                if (command === "plugin:event|listen") {
                    const listenArgs = args as { event: string; handler: number };
                    listeners[listenArgs.event] = listenArgs.handler;
                    return nextCallbackId++;
                }
                if (command === "plugin:event|unlisten") return null;
                if (command === "plugin:dialog|open") {
                    return (window as TestWindow).__toolboxDialogResults?.shift() ?? pdfPath;
                }
                if (command === "open_paths") {
                    return null;
                }
                if (command === "inspect_pdf_scene") return { path: pdfPath, pages: [{ index: 0, width: 612, height: 792, preview }] };
                if (command === "inspect_pdf_scene_page") return { index: 0, width: 612, height: 792, preview, textRuns: [] };
                if (command === "preview_pdf_scene_pages") return (args as { request: { pageIndices: number[] } }).request.pageIndices.map((pageIndex) => ({ pageIndex, preview: { dataUrl: preview, width: 612, height: 792 } }));
                if (command === "inspect_tiff_pages") return [{ width: 640, height: 480, dataUrl: preview }];
                if (command === "export_pdf_scene") return [{ inputPath: pdfPath, outputPaths: [`${pdfPath}.edited.pdf`], detail: "Exported", failure: null }];
                if (command.startsWith("plugin:")) return null;
                return null;
            },
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, { pdfPath, preview });
});

const emitShellEvent = async (page: Page, payload: ShellEvent) => {
    await expect.poll(() => page.evaluate(() => Boolean((window as TestWindow).__toolboxListeners?.["toolbox://shell-event"]))).toBe(true);
    await page.evaluate((event) => {
        (window as TestWindow).__toolboxEmit?.("toolbox://shell-event", event);
    }, payload);
};

const invocations = (page: Page) => page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []);

test("subscribes before readiness and routes typed native shell commands", async ({ page }) => {
    await page.goto("/");
    await expect.poll(async () => {
        const calls = await invocations(page);
        return calls.findIndex((call) => call.command === "plugin:event|listen") < calls.findIndex((call) => call.command === "shell_ready");
    }).toBe(true);

    await emitShellEvent(page, { kind: "command", command: "search" });
    await expect(page.getByRole("textbox", { name: "Search tools" })).toBeFocused();

    await emitShellEvent(page, { kind: "command", command: "open" });
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "open_paths" && JSON.stringify(call.args) === JSON.stringify({ paths: [pdfPath] }))).toBe(true);
    await emitShellEvent(page, { kind: "files", activationId: "activation-open-dialog", workspace: "pdf-editor", paths: [pdfPath] });
    await expect(page.getByRole("button", { name: "← All tools" })).toBeVisible();
    await expect(page.getByText(path.basename(pdfPath), { exact: true })).toBeVisible();
    expect((await invocations(page)).some((call) => call.command === "plugin:dialog|open")).toBe(true);
});

test("publishes the selected workspace and document title to the native window", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Edit PDF" }).click();
    await expect.poll(async () => (await invocations(page)).filter((call) => call.command === "set_document_title")).toContainEqual({
        command: "set_document_title",
        args: { title: expect.any(String) },
    });

    await emitShellEvent(page, { kind: "files", activationId: "activation-pdf-1", workspace: "pdf-editor", paths: [pdfPath] });
    await expect(page.getByText(path.basename(pdfPath), { exact: true })).toBeVisible();
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "set_document_title" && JSON.stringify(call.args).includes(path.basename(pdfPath)))).toBe(true);
});

test("accepts each typed file activation and acknowledges only after intake", async ({ page }) => {
    await page.goto("/");
    await emitShellEvent(page, { kind: "files", activationId: "activation-pdf-2", workspace: "pdf-editor", paths: [pdfPath] });
    await expect(page.getByText(path.basename(pdfPath), { exact: true })).toBeVisible();
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "acknowledge_activation" && JSON.stringify(call.args) === JSON.stringify({ activationId: "activation-pdf-2" }))).toBe(true);

    await emitShellEvent(page, { kind: "files", activationId: "activation-image-1", workspace: "image-editor", paths: [imagePath] });
    await expect(page.getByText(path.basename(imagePath), { exact: true })).toBeVisible();
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "acknowledge_activation" && JSON.stringify(call.args) === JSON.stringify({ activationId: "activation-image-1" }))).toBe(true);
});

test("ignores a replayed activation after its intake was acknowledged", async ({ page }) => {
    const event: ShellEvent = { kind: "files", activationId: "activation-replayed", workspace: "pdf-editor", paths: [pdfPath] };
    await page.goto("/");
    await emitShellEvent(page, event);
    await expect.poll(async () => (await invocations(page)).filter((call) => call.command === "acknowledge_activation")).toHaveLength(1);

    await emitShellEvent(page, event);
    await page.waitForTimeout(300);
    expect((await invocations(page)).filter((call) => call.command === "acknowledge_activation")).toHaveLength(1);
});

test("hands off a second activation without overwriting the in-flight intake", async ({ page }) => {
    const secondPath = "/tmp/second-document.pdf";
    await page.goto("/");
    await emitShellEvent(page, { kind: "files", activationId: "activation-burst-1", workspace: "pdf-editor", paths: [pdfPath] });
    await emitShellEvent(page, { kind: "files", activationId: "activation-burst-2", workspace: "pdf-editor", paths: [secondPath] });

    const acknowledge = (activationId: string) => JSON.stringify({ activationId });
    await expect.poll(async () => {
        const calls = await invocations(page);
        const accepted = calls.filter((call) => call.command === "acknowledge_activation").map((call) => JSON.stringify(call.args));
        return accepted;
    }).toEqual([acknowledge("activation-burst-1"), acknowledge("activation-burst-2")]);
    await expect(page.getByText(path.basename(secondPath), { exact: true })).toBeVisible();
});

test("reports rejected files and mixed batches without accepting or acknowledging them", async ({ page }) => {
    await page.goto("/");
    const unsupported = "/tmp/not-a-toolbox-input.xyz";
    await emitShellEvent(page, { kind: "rejected-files", paths: [unsupported], reason: "Unsupported file type: .xyz" });
    await expect(page.getByRole("alert")).toContainText("Unsupported file type: .xyz");
    await expect(page.getByText(path.basename(unsupported), { exact: true })).toBeVisible();

    const mixed = [pdfPath, imagePath];
    await emitShellEvent(page, { kind: "rejected-files", paths: mixed, reason: "Choose PDF and image files separately." });
    await expect(page.getByRole("alert")).toContainText("Choose PDF and image files separately.");
    const calls = await invocations(page);
    expect(calls.filter((call) => call.command === "acknowledge_activation")).toHaveLength(0);
    await expect(page.getByRole("heading", { name: "Ready to process" })).toBeVisible();
    await expect(page.getByText("0 files open", { exact: true })).toHaveCount(0);
});

test("routes dropped files through the active tool input contract", async ({ page }) => {
    await page.goto("/");

    const officePath = "/tmp/contract.docx";
    await page.getByRole("button", { name: "Open Protect Office Files", exact: true }).click();
    await emitShellEvent(page, { kind: "dropped-files", paths: [officePath] });
    await expect(page.getByText("contract.docx", { exact: true })).toBeVisible();
    await expect(page.locator(".file-selection-count")).toHaveText("1 file open");
    await expect(page.getByRole("alert")).toHaveCount(0);

    await page.getByRole("button", { name: "← All tools" }).click();
    const tiffPath = "/tmp/archive.tiff";
    await page.getByRole("button", { name: "Open Split and Combine TIFF", exact: true }).click();
    await emitShellEvent(page, { kind: "dropped-files", paths: [tiffPath] });
    await expect(page.getByText("archive.tiff", { exact: true })).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);

    await page.getByRole("button", { name: "← All tools" }).click();
    const gifPath = "/tmp/animation.gif";
    await page.getByRole("button", { name: "Open Extract GIF Frames", exact: true }).click();
    await emitShellEvent(page, { kind: "dropped-files", paths: [gifPath] });
    await expect(page.getByText("animation.gif", { exact: true })).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);

    await emitShellEvent(page, { kind: "dropped-files", paths: [officePath] });
    await expect(page.getByRole("alert")).toContainText("not supported by Extract GIF Frames");

    await page.getByRole("button", { name: "← All tools" }).click();
    await emitShellEvent(page, { kind: "dropped-files", paths: ["/tmp/unknown.xyz"] });
    await expect(page.getByRole("alert")).toContainText("Open a tool before dropping files here.");
});

test("keeps multi-file drops intact for ordered tools", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Images to PDF", exact: true }).click();

    const imagePaths = ["/tmp/cover.jpg", "/tmp/inside.jpg"];
    await emitShellEvent(page, { kind: "dropped-files", paths: imagePaths });
    await expect(page.locator(".file-selection-count")).toHaveText("2 files open");
    await expect(page.getByText("cover.jpg", { exact: true })).toBeVisible();
    await expect(page.getByText("inside.jpg", { exact: true })).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
});

test("native undo, redo, and export commands target the active editor", async ({ page }) => {
    await page.goto("/");
    await emitShellEvent(page, { kind: "files", activationId: "activation-pdf-history", workspace: "pdf-editor", paths: [pdfPath] });
    await expect(page.getByText(path.basename(pdfPath), { exact: true })).toBeVisible();
    const canvas = page.getByRole("group", { name: "PDF page canvas" });
    await expect(canvas).toBeVisible();
    const bounds = (await canvas.boundingBox())!;
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Text", exact: true }).click();
    await page.mouse.move(bounds.x + bounds.width * 0.3, bounds.y + bounds.height * 0.3);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width * 0.5, bounds.y + bounds.height * 0.36, { steps: 4 });
    await page.mouse.up();
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toBeVisible();

    await emitShellEvent(page, { kind: "command", command: "undo" });
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toHaveCount(0);
    await emitShellEvent(page, { kind: "command", command: "redo" });
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Export PDF", exact: true })).toBeEnabled();
    await emitShellEvent(page, { kind: "command", command: "export" });
    await expect.poll(async () => (await invocations(page)).filter((call) => call.command === "export_pdf_scene")).toHaveLength(1);
});

test("keyboard shortcuts focus search, open a document, undo, redo, and export", async ({ page }) => {
    await page.goto("/");
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "shell_ready")).toBe(true);
    await page.keyboard.press("Control+k");
    await expect(page.getByRole("textbox", { name: "Search tools" })).toBeFocused();

    await page.keyboard.press("Control+o");
    await expect.poll(async () => (await invocations(page)).some((call) => call.command === "open_paths")).toBe(true);
    await emitShellEvent(page, { kind: "files", activationId: "activation-shortcut-open", workspace: "pdf-editor", paths: [pdfPath] });
    await expect(page.getByRole("button", { name: "← All tools" })).toBeVisible();
    await expect(page.getByText(path.basename(pdfPath), { exact: true })).toBeVisible();
    const canvas = page.getByRole("group", { name: "PDF page canvas" });
    const bounds = (await canvas.boundingBox())!;
    await page.getByRole("toolbar", { name: "PDF editor tools" }).getByRole("button", { name: "Text", exact: true }).click();
    await page.mouse.move(bounds.x + bounds.width * 0.3, bounds.y + bounds.height * 0.3);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width * 0.5, bounds.y + bounds.height * 0.36, { steps: 4 });
    await page.mouse.up();
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toBeVisible();

    await page.keyboard.press("Control+z");
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toHaveCount(0);
    await page.keyboard.press("Control+Shift+z");
    await expect(page.getByRole("button", { name: "text object 1", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Export PDF", exact: true })).toBeEnabled();
    await page.keyboard.press("Control+e");
    await expect.poll(async () => (await invocations(page)).filter((call) => call.command === "export_pdf_scene")).toHaveLength(1);
});

test("Escape closes the topmost settings surface before leaving the workspace", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Edit PDF" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeHidden();
    await expect(page.getByRole("heading", { name: "PDF editor", exact: true })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByRole("heading", { name: "Ready to process" })).toBeVisible();
});
