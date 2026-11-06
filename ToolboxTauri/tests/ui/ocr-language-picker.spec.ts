import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type OcrLanguage = { code: string; name: string; packId: string | null; status: string; downloadBytes: number | null };

type PickerWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __languages: OcrLanguage[];
    /** Installs in the order the picker asked for them, held until the test settles them. */
    __installs: Array<{ packId: string; runId: string; settle: () => void }>;
    __emit: (event: string, payload: unknown) => void;
};

const MB = 1024 * 1024;
const english: OcrLanguage = { code: "eng", name: "English", packId: null, status: "installed", downloadBytes: null };
const french: OcrLanguage = { code: "fra", name: "French", packId: "ocr-french", status: "installed", downloadBytes: 3 * MB };
const hindi: OcrLanguage = { code: "hin", name: "Hindi", packId: "ocr-hindi", status: "notInstalled", downloadBytes: 3 * MB };
const tamil: OcrLanguage = { code: "tam", name: "Tamil", packId: "ocr-tamil", status: "damaged", downloadBytes: 5 * MB };

const pagePreview = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'/%3E";

test.beforeEach(async ({ page }) => {
    // OCR is still gated off in this build (#575); the picker is exercised as the tool will ship.
    await page.route("**/shared/tool-capabilities.json*", async (route) => {
        const response = await route.fetch();
        const body = (await response.text()).replace(/(ocr_pdf[^}]*?nativeAvailability\\?"\s*:\s*\\?")unavailable/, "$1available");
        await route.fulfill({ response, body });
    });
    await page.addInitScript(({ pagePreview, english }) => {
        const win = window as unknown as PickerWindow;
        win.__invocations = [];
        win.__installs = [];
        win.__languages = [english];
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__emit = (event, payload) => {
            for (const [id, listener] of listeners) {
                if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
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
                if (command === "ocr_languages") return win.__languages;
                if (command === "install_pack") {
                    const { packId, runId } = args as { packId: string; runId: string };
                    return new Promise((resolve) => {
                        win.__installs.push({ packId, runId, settle: () => resolve({ packs: [], installedCount: 1, totalBytes: 0 }) });
                    });
                }
                if (command === "inspect_pdf") {
                    return { pages: [{ index: 0, x: 0, y: 0, width: 612, height: 792, preview: pagePreview }] };
                }
                if (command === "plugin:dialog|open") return "/scans/ration-card.pdf";
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "get_quick_actions") return [];
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:app|version") return "9.4.2";
                if (command === "ocr_pdf") return [];
                if (command.startsWith("plugin:")) return null;
                return null;
            },
            transformCallback: (callback: (event: unknown) => void) => {
                const id = nextId++;
                callbacks.set(id, callback);
                return id;
            },
            unregisterCallback: (id: number) => callbacks.delete(id),
            convertFileSrc: (path: string) => path,
        } as unknown as typeof window.__TAURI_INTERNALS__;
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, { pagePreview, english });
});

const openOcr = async (page: Page, languages?: OcrLanguage[]) => {
    if (languages) await page.addInitScript((languages) => { (window as unknown as PickerWindow).__languages = languages; }, languages);
    await page.goto("/");
    await openToolBySearch(page, "OCR PDF");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("checkbox", { name: "Page 1" })).toBeVisible();
};

const emit = (page: Page, event: string, payload: unknown) =>
    page.evaluate(([event, payload]) => (window as unknown as PickerWindow).__emit(event as string, payload), [event, payload] as const);

const setLanguages = (page: Page, languages: OcrLanguage[]) =>
    page.evaluate((languages) => { (window as unknown as PickerWindow).__languages = languages; }, languages);

const lastOcrRequest = (page: Page) => page.evaluate(() =>
    (window as unknown as PickerWindow).__invocations.filter(({ command }) => command === "ocr_pdf").at(-1)?.args);

test("English reads scans with no language packs installed", async ({ page }) => {
    await openOcr(page);
    const picker = page.getByRole("combobox", { name: "Language" });
    await expect(picker).toHaveText("English");
    await expect(page.getByLabel("English status")).toHaveText("Installed");

    await picker.click();
    await expect(page.getByRole("option", { name: "English, installed" })).toBeVisible();
    await expect(page.getByRole("listbox").getByRole("group", { name: "Download to use" })).toHaveCount(0);
    await page.keyboard.press("Escape");

    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { paths: ["/scans/ration-card.pdf"], languages: ["eng"] } });
});

test("a language from a pack installs from the picker and then runs in that language", async ({ page }) => {
    await openOcr(page, [english, french, hindi, tamil]);
    const picker = page.getByRole("combobox", { name: "Language" });
    await picker.click();
    const installed = page.getByRole("listbox").getByRole("group", { name: "Installed", exact: true });
    const downloadable = page.getByRole("listbox").getByRole("group", { name: "Download to use" });
    await expect(installed.getByRole("option")).toHaveText(["English", "French"]);
    await expect(installed.getByRole("option", { name: "French, installed" })).toBeVisible();
    await expect(downloadable.getByRole("option", { name: "Hindi, not installed, 3 MB download" })).toBeVisible();
    await expect(downloadable.getByRole("option", { name: "Tamil, not installed, 5 MB download" })).toBeVisible();

    await downloadable.getByRole("option", { name: /^Hindi/ }).click();
    await expect(picker).toHaveText("Hindi");
    await expect(page.getByLabel("Hindi status")).toHaveText("Not installed");
    await expect(page.getByText("Hindi needs a 3 MB download.")).toBeVisible();
    await page.getByRole("button", { name: "Install Hindi" }).click();
    expect(await page.evaluate(() => (window as unknown as PickerWindow).__installs.map(({ packId }) => packId))).toEqual(["ocr-hindi"]);

    const runId = await page.evaluate(() => (window as unknown as PickerWindow).__installs[0].runId);
    await emit(page, "pack-progress", { runId, packId: "ocr-hindi", phase: "downloading", receivedBytes: 1.86 * MB, totalBytes: 3 * MB });
    await expect(page.getByRole("status").filter({ hasText: "Downloading 62%" })).toBeVisible();

    await setLanguages(page, [english, french, { ...hindi, status: "installed" }, tamil]);
    await emit(page, "pack-changed", { packId: "ocr-hindi" });
    await page.evaluate(() => (window as unknown as PickerWindow).__installs[0].settle());
    await expect(page.getByLabel("Hindi status")).toHaveText("Installed");
    await expect(page.getByRole("button", { name: "Install Hindi" })).toHaveCount(0);
    await picker.click();
    await expect(page.getByRole("option", { name: "Hindi, installed" })).toBeVisible();
    await page.keyboard.press("Escape");

    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { paths: ["/scans/ration-card.pdf"], languages: ["hin"] } });
});

test("a damaged language pack offers Repair", async ({ page }) => {
    await openOcr(page, [english, tamil]);
    await page.getByRole("combobox", { name: "Language" }).click();
    await page.getByRole("option", { name: /^Tamil/ }).click();
    await expect(page.getByLabel("Tamil status")).toHaveText("Damaged");
    await page.getByRole("button", { name: "Repair Tamil" }).click();
    expect(await page.evaluate(() => (window as unknown as PickerWindow).__installs.map(({ packId }) => packId))).toEqual(["ocr-tamil"]);
});
