import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type EditWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
    __emit: (event: string, payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

const blank = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E";

const edited = (inputPath: string, folder = inputPath.replace(/\/[^/]+$/, "")): Outcome => ({
    inputPath,
    outputPaths: [`${folder}/${inputPath.split("/").pop()!.replace(/(\.\w+)$/, "-cropped$1")}`],
    detail: "Image saved",
    state: "succeeded",
    failure: null,
    reason: null,
    warnings: [],
});

test.beforeEach(async ({ page }) => {
    await page.addInitScript((blank) => {
        const win = window as unknown as EditWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__emit = (name, payload) => {
            for (const [id, listener] of listeners) {
                if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
            }
        };
        win.__finish = (outcomes) => finish?.(outcomes);
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
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
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "get_quick_actions") return [];
                if (command === "cancel_run") return true;
                if (command === "plugin:app|version") return "9.4.2";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: blank };
                if (command === "inspect_image_edit_preview") return { width: 384, height: 480, dataUrl: blank };
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command === "export_image_edit_stack") return new Promise((resolve) => { finish = resolve; });
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
    }, blank);
});

const invocations = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as EditWindow).__invocations.filter((invocation) => invocation.command === command), command);

const queueDialog = (page: Page, result: string | string[] | null) =>
    page.evaluate((result) => { (window as unknown as EditWindow).__dialogResults.push(result); }, result);

const openCrop = async (page: Page, paths: string[]) => {
    await page.goto("/");
    await openToolBySearch(page, "Crop Images");
    await queueDialog(page, paths);
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(`Previewing ${paths[0].split("/").pop()}`, { exact: true })).toBeVisible();
};

const failed = (inputPath: string, failure: Record<string, unknown>): Outcome => ({
    inputPath, outputPaths: [], detail: "", state: "failed", failure, reason: null, warnings: [],
});

const tooLarge = "Too large to edit: 42000 × 9000. The limit is 16384 px on a side.";
const animated = "Animated GIF. Edit Image works on still images. Extract GIF Frames saves each frame.";
const outcomes = [
    edited("/photos/IMG_4012.jpg"),
    failed("/photos/panorama.png", { kind: "limitExceeded", details: { limit: "dimensions" }, message: tooLarge, pointer: null }),
    failed("/photos/walkthrough.gif", { kind: "invalidInput", message: animated, pointer: { kind: "tool", toolId: "gif-extract" } }),
];
const runnable = outcomes.map((outcome) => outcome.inputPath);
const chosen = [...runnable, "/photos/listing-notes.docx"];

const runWithLimits = async (page: Page) => {
    await openCrop(page, chosen);
    const notAdded = page.getByRole("region", { name: "Not added" });
    await expect(notAdded.getByText("1 item not added")).toBeVisible();
    await expect(notAdded.getByRole("listitem")).toHaveText([/listing-notes\.docx.*Image Editor only takes .*\.gif.* files\./]);

    await page.getByRole("button", { name: "Run on 3 images" }).click();
    await expect.poll(async () => (await invocations(page, "export_image_edit_stack")).length).toBe(1);
    const [run] = await invocations(page, "export_image_edit_stack");
    expect((run.args as { request: { paths: string[] } }).request.paths).toEqual(runnable);
    await page.evaluate((outcomes) => (window as unknown as EditWindow).__finish(outcomes), outcomes);
    const rows = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(rows).toHaveCount(3);
    return rows;
};

test("a rejected type stays out of the run, and over-limit and animated GIF rows fail alone", async ({ page }) => {
    const rows = await runWithLimits(page);
    await expect(rows.nth(0).getByRole("button", { name: "Open IMG_4012-cropped.jpg" })).toBeVisible();

    const panorama = rows.filter({ hasText: "panorama.png" });
    await expect(panorama).toContainText(tooLarge);
    await expect(panorama.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

    const gif = rows.filter({ hasText: "walkthrough.gif" });
    await expect(gif).toContainText(animated);
    await gif.getByRole("button", { name: "Open Extract GIF Frames" }).click();
    await expect(page.getByRole("heading", { name: "Extract GIF Frames" }).first()).toBeVisible();
});

for (const theme of ["light", "dark"] as const) {
    test(`rejected types, over-limit and animated GIF rows render in ${theme} mode`, async ({ page }, testInfo) => {
        await page.addInitScript((theme) => localStorage.setItem("toolbox-theme", theme), theme);
        const rows = await runWithLimits(page);
        await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
        await expect(rows.filter({ hasText: "panorama.png" })).toContainText(tooLarge);
        await expect(rows.filter({ hasText: "walkthrough.gif" }).getByRole("button", { name: "Open Extract GIF Frames" })).toBeVisible();
        await rows.last().scrollIntoViewIfNeeded();
        await page.screenshot({ path: testInfo.outputPath(`image-edit-limits-${theme}.png`) });
    });
}
