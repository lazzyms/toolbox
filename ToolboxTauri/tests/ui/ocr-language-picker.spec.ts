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
const gujarati: OcrLanguage = { code: "guj", name: "Gujarati", packId: "ocr-gujarati", status: "notInstalled", downloadBytes: 14 * MB };

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

const languageList = (page: Page) => page.getByRole("list", { name: "Languages, in order of priority" });

/** Each row's accessible name, in priority order. */
const rows = (page: Page) => languageList(page).getByRole("listitem");

const addLanguage = async (page: Page, name: RegExp) => {
    await page.getByRole("combobox", { name: "Add a language" }).click();
    await page.getByRole("option", { name }).click();
};

const removedNotice = (page: Page) => page.getByRole("status").filter({ hasText: "removed from your languages" });

const remember = async (page: Page, codes: string[]) => {
    await page.goto("/");
    await page.evaluate((codes) => window.localStorage.setItem("toolbox-ocr-languages", JSON.stringify(codes)), codes);
};

const lastOcrRequest = (page: Page) => page.evaluate(() =>
    (window as unknown as PickerWindow).__invocations.filter(({ command }) => command === "ocr_pdf").at(-1)?.args);

test("English reads scans with no language packs installed", async ({ page }) => {
    await openOcr(page);
    await expect(rows(page)).toHaveCount(1);
    await expect(languageList(page).getByRole("listitem", { name: "English, installed, priority 1" })).toBeVisible();
    // One language can't be moved or removed.
    await expect(languageList(page).getByRole("button")).toHaveCount(0);
    await expect(page.getByText("Recognition runs on this device. Your last choice is remembered.")).toBeVisible();

    await page.getByRole("combobox", { name: "Add a language" }).click();
    await expect(page.getByRole("option", { name: "English, installed, already added" })).toBeDisabled();
    await expect(page.getByRole("listbox").getByRole("group", { name: "Download to use" })).toHaveCount(0);
    await page.keyboard.press("Escape");

    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { paths: ["/scans/ration-card.pdf"], languages: ["eng"] } });
});

test("the add list comes from the installed and installable languages the app reports", async ({ page }) => {
    await openOcr(page, [english, french, hindi, tamil]);
    expect(await page.evaluate(() => (window as unknown as PickerWindow).__invocations.some(({ command }) => command === "ocr_languages"))).toBe(true);
    await page.getByRole("combobox", { name: "Add a language" }).click();
    const installed = page.getByRole("listbox").getByRole("group", { name: "Installed", exact: true });
    const downloadable = page.getByRole("listbox").getByRole("group", { name: "Download to use" });
    await expect(installed.getByRole("option")).toHaveText(["EnglishAdded", "French"]);
    await expect(installed.getByRole("option", { name: "English, installed, already added" })).toBeDisabled();
    await expect(installed.getByRole("option", { name: "French, installed" })).toBeEnabled();
    await expect(downloadable.getByRole("option", { name: "Hindi, not installed, 3 MB download" })).toBeVisible();
    await expect(downloadable.getByRole("option", { name: "Tamil, not installed, 5 MB download" })).toBeVisible();
    await expect(page.getByText("Choosing one adds it to the list. It downloads when you click Install or run.")).toBeVisible();
});

test("several languages run in the order chosen, and a pack installs from its note", async ({ page }) => {
    await openOcr(page, [english, french, hindi, tamil]);
    await addLanguage(page, /^French/);
    await addLanguage(page, /^Hindi/);
    await expect(rows(page)).toHaveCount(3);
    expect(await rows(page).evaluateAll((items) => items.map((item) => item.getAttribute("aria-label")))).toEqual([
        "English, installed, priority 1",
        "French, installed, priority 2",
        "Hindi, not installed, priority 3",
    ]);

    // The first language moves down; every other one moves up.
    await page.getByRole("button", { name: "Move Hindi up" }).click();
    await page.getByRole("button", { name: "Move English down" }).click();
    await page.getByRole("button", { name: "Remove French" }).click();
    expect(await rows(page).evaluateAll((items) => items.map((item) => item.getAttribute("aria-label")))).toEqual([
        "Hindi, not installed, priority 1",
        "English, installed, priority 2",
    ]);
    await page.getByRole("combobox", { name: "Add a language" }).click();
    await expect(page.getByRole("option", { name: "Hindi, not installed, 3 MB download, already added" })).toBeDisabled();
    await expect(page.getByRole("option", { name: "French, installed" })).toBeEnabled();
    await page.keyboard.press("Escape");

    await expect(page.getByText("Hindi needs a 3 MB download.")).toBeVisible();
    await page.getByRole("button", { name: "Install Hindi" }).click();
    expect(await page.evaluate(() => (window as unknown as PickerWindow).__installs.map(({ packId }) => packId))).toEqual(["ocr-hindi"]);
    const runId = await page.evaluate(() => (window as unknown as PickerWindow).__installs[0].runId);
    await emit(page, "pack-progress", { runId, packId: "ocr-hindi", phase: "downloading", receivedBytes: 1.86 * MB, totalBytes: 3 * MB });
    await expect(languageList(page).getByRole("status")).toHaveText("Downloading 62%");
    await expect(languageList(page).getByRole("listitem", { name: "Hindi, downloading 62%, priority 1" })).toBeVisible();

    await setLanguages(page, [english, french, { ...hindi, status: "installed" }, tamil]);
    await emit(page, "pack-changed", { packId: "ocr-hindi" });
    await page.evaluate(() => (window as unknown as PickerWindow).__installs[0].settle());
    await expect(languageList(page).getByRole("listitem", { name: "Hindi, installed, priority 1" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Install Hindi" })).toHaveCount(0);

    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { paths: ["/scans/ration-card.pdf"], languages: ["hin", "eng"] } });
});

test("the last choice of languages is remembered across reloads", async ({ page }) => {
    await openOcr(page, [english, french, hindi]);
    await addLanguage(page, /^French/);
    await page.getByRole("button", { name: "Move French up" }).click();
    expect(await page.evaluate(() => window.localStorage.getItem("toolbox-ocr-languages"))).toBe(JSON.stringify(["fra", "eng"]));

    await page.reload();
    await openOcr(page);
    expect(await rows(page).evaluateAll((items) => items.map((item) => item.getAttribute("aria-label")))).toEqual([
        "French, installed, priority 1",
        "English, installed, priority 2",
    ]);
    await expect(removedNotice(page)).toHaveCount(0);
    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { languages: ["fra", "eng"] } });
});

test("a remembered language whose pack is no longer installed is dropped with a notice", async ({ page }) => {
    await remember(page, ["fra", "hin", "eng"]);
    await openOcr(page, [english, french, hindi]);
    expect(await rows(page).evaluateAll((items) => items.map((item) => item.getAttribute("aria-label")))).toEqual([
        "French, installed, priority 1",
        "English, installed, priority 2",
    ]);
    await expect(removedNotice(page)).toHaveText("Hindi was removed from your languages. Its pack isn't installed any more. Add it again from the list to download it.");
    expect(await page.evaluate(() => window.localStorage.getItem("toolbox-ocr-languages"))).toBe(JSON.stringify(["fra", "eng"]));
    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { languages: ["fra", "eng"] } });

    // Adding a language again clears the notice.
    await addLanguage(page, /^Hindi/);
    await expect(removedNotice(page)).toHaveCount(0);
});

test("when every remembered language is gone, English reads on its own", async ({ page }) => {
    await remember(page, ["hin", "guj"]);
    await openOcr(page, [english, hindi, gujarati]);
    expect(await rows(page).evaluateAll((items) => items.map((item) => item.getAttribute("aria-label")))).toEqual(["English, installed, priority 1"]);
    await expect(removedNotice(page)).toHaveText("Hindi and Gujarati were removed from your languages. Their packs aren't installed any more. Add them again from the list to download them.");
    await page.getByRole("button", { name: "Export OCR PDF" }).click();
    expect(await lastOcrRequest(page)).toMatchObject({ request: { languages: ["eng"] } });
});

test("a damaged language pack offers Repair", async ({ page }) => {
    await openOcr(page, [english, tamil]);
    await addLanguage(page, /^Tamil/);
    await expect(languageList(page).getByRole("listitem", { name: "Tamil, damaged, priority 2" })).toBeVisible();
    await page.getByRole("button", { name: "Repair Tamil" }).click();
    expect(await page.evaluate(() => (window as unknown as PickerWindow).__installs.map(({ packId }) => packId))).toEqual(["ocr-tamil"]);
});
