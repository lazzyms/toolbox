import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

// The creation tools (App Icon Set, Create GIF, Extract GIF Frames and TIFF Pages) share one destination, one set of
// shortcuts and one intake. These tests pin what each tool shows and sends for them.

type MediaWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
    __drop: (paths: string[]) => void;
};

const blankImage = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E";
const runFolder = "/Users/test/Run out";

test.beforeEach(async ({ page }) => {
    await page.addInitScript((blankImage) => {
        const win = window as unknown as MediaWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResults = [];
        win.__drop = (paths) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths } });
            }
        };
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
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_quick_actions") return [];
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: blankImage };
                if (command === "inspect_tiff_pages") return [{ width: 640, height: 480, dataUrl: blankImage }];
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                const { request } = (args ?? {}) as { request?: { paths?: string[] } };
                return (request?.paths ?? []).map((inputPath) => ({
                    inputPath, outputPaths: [`${inputPath}.out`], detail: "", state: "succeeded", failure: null, reason: null, warnings: [],
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
    }, blankImage);
    await page.goto("/");
});

const queueDialog = (page: Page, result: string | string[]) =>
    page.evaluate((result) => { (window as unknown as MediaWindow).__dialogResults.push(result); }, result);

const choose = async (page: Page, files: string[]) => {
    await queueDialog(page, files);
    await page.getByRole("button", { name: "Choose files to process" }).click();
};

const drop = async (page: Page, paths: string[]) => {
    await expect.poll(() => page.evaluate(() => (window as unknown as MediaWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as MediaWindow).__drop(paths), paths);
};

const invocationsOf = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as MediaWindow).__invocations.filter((invocation) => invocation.command === command).map(({ args }) => args),
command);

const requestsOf = async (page: Page, command: string) =>
    (await invocationsOf(page, command) as Array<{ request: Record<string, unknown> }>).map(({ request }) => request);

const destination = (page: Page) => page.getByRole("region", { name: "Destination" });

const changeForThisRun = async (page: Page) => {
    await queueDialog(page, runFolder);
    await destination(page).getByRole("button", { name: "Change for this run" }).click();
    await expect(destination(page).getByTestId("run-destination-path")).toHaveText("~/Run out");
    await expect(destination(page)).toContainText("This run only");
};

const run = async (page: Page, command: string, save?: { folder: string; name: string }) => {
    const before = (await invocationsOf(page, command)).length;
    await page.locator(".workspace-primary-action").click();
    if (save) {
        const dialog = page.getByRole("dialog");
        await expect(dialog.getByTestId("save-folder")).toHaveText(save.folder);
        await expect(dialog.getByLabel("File name")).toHaveValue(save.name);
        await dialog.getByRole("button", { name: "Save" }).click();
    }
    await expect.poll(async () => (await invocationsOf(page, command)).length).toBe(before + 1);
    return (await requestsOf(page, command))[before];
};

const tiffMode = (page: Page) => page.getByRole("radiogroup", { name: "TIFF Pages mode" });

test.describe("Creation tools: destination", () => {
    for (const { title, command, file, mode } of [
        { title: "Generate App Icons", command: "generate_icon_set", file: "/Users/test/art/logo.png" },
        { title: "Extract GIF Frames", command: "extract_gif_frames", file: "/Users/test/clips/spinner.gif" },
        { title: "Split and Combine TIFF", command: "process_tiff_pages", file: "/Users/test/scans/scan.tif", mode: "split" },
    ]) {
        test(`${title} saves into a folder chosen for this run without changing the saved setting`, async ({ page }) => {
            await openToolBySearch(page, title);
            await choose(page, [file]);
            await expect(destination(page).getByTestId("run-destination-path")).toHaveText("Next to each original");
            expect(await run(page, command)).toMatchObject({ paths: [file], outputLocation: "alongsideInput", ...(mode ? { mode } : {}) });

            await changeForThisRun(page);
            expect(await run(page, command)).toMatchObject({ paths: [file], outputLocation: { customFolder: runFolder } });
            expect(await invocationsOf(page, "set_output_settings")).toEqual([]);

            await destination(page).getByRole("button", { name: "Reset to default" }).click();
            await expect(destination(page).getByTestId("run-destination-path")).toHaveText("Next to each original");
            expect(await run(page, command)).toMatchObject({ outputLocation: "alongsideInput" });
        });
    }

    for (const { title, command, noun, nextTo, files, name, combine } of [
        { title: "Create GIF", command: "create_gif", noun: "GIF", nextTo: "first frame", files: ["/Users/test/frames/a.png", "/Users/test/frames/b.png"], name: "a-animated.gif", combine: false },
        { title: "Split and Combine TIFF", command: "process_tiff_pages", noun: "TIFF", nextTo: "first file", files: ["/Users/test/scans/a.tif", "/Users/test/scans/b.tif"], name: "a-combined.tif", combine: true },
    ]) {
        test(`${title}${combine ? " Combine" : ""} shows where its save dialog opens and opens it in the folder chosen for this run`, async ({ page }) => {
            await openToolBySearch(page, title);
            if (combine) await tiffMode(page).getByRole("radio", { name: "Combine into one TIFF" }).click();
            const place = destination(page);
            await expect(place).toContainText(`You name the ${noun} when you start. The save dialog opens next to the ${nextTo}:`);
            await expect(place.getByTestId("run-destination-path")).toHaveText(`Next to the ${nextTo}`);
            await expect(place).toContainText("If you pick a name that's taken, Toolbox adds -1 and tells you. Nothing is replaced.");

            await choose(page, files);
            await expect(place.getByTestId("run-destination-path")).toHaveText(files[0].startsWith("/Users/test/frames") ? "~/frames" : "~/scans");
            const folder = files[0].slice(0, files[0].lastIndexOf("/"));
            expect(await run(page, command, { folder: folder.replace("/Users/test", "~"), name })).toMatchObject({
                outputLocation: "alongsideInput",
                outputPath: `${folder}/${name}`,
            });

            await changeForThisRun(page);
            await expect(place).toContainText(`You name the ${noun} when you start. The save dialog opens in:`);
            expect(await run(page, command, { folder: "~/Run out", name })).toMatchObject({
                outputLocation: { customFolder: runFolder },
                outputPath: `${runFolder}/${name}`,
            });
            expect(await invocationsOf(page, "set_output_settings")).toEqual([]);
        });
    }
});

test.describe("Creation tools: shortcuts", () => {
    test("Create GIF adds files and runs from the keyboard, and Add stays quiet in the delay field", async ({ page }) => {
        await openToolBySearch(page, "Create GIF");
        const open = page.getByRole("button", { name: "Choose files to process" });
        await expect(open).toHaveAttribute("aria-keyshortcuts", "Meta+O");
        await queueDialog(page, ["/Users/test/frames/a.png", "/Users/test/frames/b.png"]);
        await page.locator("body").press("Meta+O");
        const frames = page.getByRole("listbox", { name: "Frames, in play order" }).getByRole("option");
        await expect(frames).toHaveCount(2);

        // The selected frame moves with the modifier and an arrow key.
        await page.getByRole("button", { name: "Frame 2, b.png, 100 ms" }).click();
        await page.keyboard.press("Meta+ArrowLeft");
        await expect(frames.first()).toHaveAccessibleName("Frame 1, b.png, 100 ms");

        // Add files does not fire while typing a delay; Start the run does.
        const opensBefore = (await invocationsOf(page, "plugin:dialog|open")).length;
        const delay = page.getByLabel("Delay for every frame");
        await delay.fill("120");
        await delay.press("Meta+O");
        await expect(delay).toBeFocused();
        expect((await invocationsOf(page, "plugin:dialog|open")).length).toBe(opensBefore);

        await expect(page.locator(".workspace-primary-action")).toHaveAttribute("aria-keyshortcuts", "Meta+Enter");
        await delay.press("Meta+Enter");
        const dialog = page.getByRole("dialog");
        await expect(dialog.getByLabel("File name")).toHaveValue("b-animated.gif");
        await dialog.getByRole("button", { name: "Save" }).click();
        await expect.poll(async () => (await requestsOf(page, "create_gif")).length).toBe(1);
        expect((await requestsOf(page, "create_gif"))[0]).toMatchObject({
            paths: ["/Users/test/frames/b.png", "/Users/test/frames/a.png"],
            frameDelayMs: 120,
        });
    });

    for (const { title, command, file } of [
        { title: "Generate App Icons", command: "generate_icon_set", file: "/Users/test/art/logo.png" },
        { title: "Extract GIF Frames", command: "extract_gif_frames", file: "/Users/test/clips/spinner.gif" },
        { title: "Split and Combine TIFF", command: "process_tiff_pages", file: "/Users/test/scans/scan.tif" },
    ]) {
        test(`${title} adds files and runs from the keyboard`, async ({ page }) => {
            await openToolBySearch(page, title);
            await queueDialog(page, [file]);
            await page.locator("body").press("Meta+O");
            await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(1);
            await page.locator("body").press("Meta+Enter");
            await expect.poll(async () => (await requestsOf(page, command)).length).toBe(1);
            expect((await requestsOf(page, command))[0]).toMatchObject({ paths: [file] });
        });
    }
});

test.describe("Creation tools: file types", () => {
    for (const { title, accepts, refused, message } of [
        {
            title: "Generate App Icons",
            accepts: "Accepts .png, .jpg, .jpeg, .webp, .heic, .heif",
            refused: "/Users/test/drop/scan.tif",
            message: "Generate App Icons only takes .png, .jpg, .jpeg, .webp, .heic, .heif files.",
        },
        {
            title: "Create GIF",
            accepts: "Accepts .png, .jpg, .jpeg, .webp, .heic, .heif",
            refused: "/Users/test/drop/clip.gif",
            message: "Create GIF only takes .png, .jpg, .jpeg, .webp, .heic, .heif files.",
        },
        {
            title: "Split and Combine TIFF",
            accepts: "Accepts .tif, .tiff, including fax and JPEG-compressed scans",
            refused: "/Users/test/drop/photo.png",
            message: "Split and Combine TIFF only takes .tif, .tiff files.",
        },
    ]) {
        test(`${title} says what it accepts and why it won't run another type`, async ({ page }) => {
            await openToolBySearch(page, title);
            await expect(page.getByRole("region", { name: "Open document" })).toContainText(accepts);
            await drop(page, [refused]);
            const row = page.getByRole("listbox").getByRole("option").filter({ hasText: refused.split("/").pop() });
            await expect(row).toContainText(message);
            await expect(page.locator(".workspace-primary-action")).toBeDisabled();
        });
    }

    test("Extract GIF Frames keeps a still image out of the list and points it at Create GIF", async ({ page }) => {
        await openToolBySearch(page, "Extract GIF Frames");
        await expect(page.getByRole("region", { name: "Open document" })).toContainText("Accepts .gif");
        await drop(page, ["/Users/test/drop/hero-banner.png", "/Users/test/drop/spinner.gif", "/Users/test/drop/notes.txt"]);

        const notAdded = page.getByRole("region", { name: "Not added" });
        const still = notAdded.locator("li").filter({ hasText: "hero-banner.png" });
        await expect(still).toContainText("Not a GIF. To make a GIF from images, use Create GIF.");
        await expect(notAdded.locator("li").filter({ hasText: "notes.txt" })).toContainText(/only takes .*files\./);
        await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(1);
        await expect(page.getByRole("listbox").getByRole("option")).toContainText("spinner.gif");

        await still.getByRole("button", { name: "Open Create GIF", exact: true }).click();
        await expect(page.locator(".workspace-command").filter({ hasText: "Create GIF" })).toHaveAttribute("aria-pressed", "true");
        expect(await requestsOf(page, "extract_gif_frames")).toEqual([]);
    });
});
