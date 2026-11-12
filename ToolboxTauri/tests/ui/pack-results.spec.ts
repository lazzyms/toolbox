import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import type { JobOutcome } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";

type PackResultsWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __results: JobOutcome[];
    /** Installs in the order a row asked for them, held until the test settles them. */
    __installs: Array<{ packId: string; runId: string; settle: (error?: unknown) => void }>;
    __emit: (event: string, payload: unknown) => void;
};

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const MB = 1024 * 1024;

const succeeded = (inputPath: string, output: string): JobOutcome => ({ inputPath, outputPaths: [output], detail: "", state: "succeeded", failure: null, reason: null, warnings: [] });

const needsPack = (inputPath: string, message: string, pointer: { packId: string; name: string; reason: "notInstalled" | "damaged" | "incompatible" }): JobOutcome => ({
    inputPath,
    outputPaths: [],
    detail: "",
    state: "failed",
    failure: { kind: "unavailable", message, pointer: { kind: "pack", ...pointer } },
    reason: null,
    warnings: [],
});

const hindi = { packId: "ocr-hindi", name: "Hindi", reason: "notInstalled" } as const;
const hindiMessage = "Needs the Hindi text recognition pack, which isn't installed. Nothing was saved.";

/** A mixed batch, as the board shows it: one file read, two waiting on Hindi, one on a damaged Tamil pack, one on an old Gujarati pack, one plain failure. */
const mixedBatch: JobOutcome[] = [
    succeeded("/scans/site-survey-scan.pdf", "/scans/site-survey-scan-ocr.txt"),
    needsPack("/scans/ration-card-hindi.pdf", hindiMessage, hindi),
    needsPack("/scans/school-notice-hindi.pdf", hindiMessage, hindi),
    needsPack("/scans/receipt-gujarati.pdf", "Needs a newer Gujarati pack. The installed one doesn't work with this version of Toolbox. Nothing was saved.", { packId: "ocr-gujarati", name: "Gujarati", reason: "incompatible" }),
    needsPack("/scans/letter-tamil.pdf", "The Tamil pack is damaged, so it was not used. Nothing was saved.", { packId: "ocr-tamil", name: "Tamil", reason: "damaged" }),
    { inputPath: "/scans/tenancy-addendum.pdf", outputPaths: [], detail: "", state: "failed", failure: { kind: "invalidInput", message: "Not a readable PDF.", pointer: null }, reason: null, warnings: [] },
];

test.beforeEach(async ({ page }) => {
    await page.addInitScript((fixturePath) => {
        const win = window as unknown as PackResultsWindow;
        win.__invocations = [];
        win.__installs = [];
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
                if (command === "install_pack") {
                    const { packId, runId } = args as { packId: string; runId: string };
                    return new Promise((resolve, reject) => {
                        win.__installs.push({ packId, runId, settle: (error) => error === undefined ? resolve({ packs: [], installedCount: 1, totalBytes: 0 }) : reject(error) });
                    });
                }
                if (command === "plugin:dialog|open") return fixturePath;
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "get_last_save_folder") return "/Users/test/Documents";
                if (command === "check_output_folder") return "ready";
                if (command === "get_quick_actions") return [];
                if (command === "inspect_image_preview") return { width: 640, height: 480, dataUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E" };
                if (command === "inspect_image_metadata") return [];
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:app|version") return "9.4.2";
                if (command.startsWith("plugin:")) return null;
                if (command.startsWith("inspect_") || command.startsWith("get_") || command.startsWith("set_")) return null;
                return win.__results;
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
    }, fixturePath);
});

const runBatch = async (page: Page, results: JobOutcome[]) => {
    await page.goto("/");
    await page.evaluate((results) => { (window as unknown as PackResultsWindow).__results = results; }, results);
    await openToolBySearch(page, "Compress");
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await page.getByRole("button", { name: "Compress 1 file", exact: true }).click();
    return page.getByRole("list", { name: "Results" }).getByRole("listitem");
};

const emit = (page: Page, event: string, payload: unknown) =>
    page.evaluate(([event, payload]) => (window as unknown as PackResultsWindow).__emit(event as string, payload), [event, payload] as const);

const installs = (page: Page) => page.evaluate(() => (window as unknown as PackResultsWindow).__installs.map(({ packId }) => packId));

const summary = (page: Page) => page.getByRole("status").filter({ hasText: "Done." });

test("files that need a pack stay in the list with the pack's own action and no Retry, and run again once it is installed", async ({ page }) => {
    const rows = await runBatch(page, mixedBatch);
    await expect(rows).toHaveCount(6);
    await expect(summary(page)).toContainText("Done. 1 compressed, 1 failed, 4 need a pack");

    const [ration, notice, receipt, letter] = [rows.nth(1), rows.nth(2), rows.nth(3), rows.nth(4)];
    for (const row of [ration, notice, receipt, letter]) {
        await expect(row).toHaveAttribute("data-tone", "fail");
        await expect(row).toContainText("Needs a pack");
        await expect(row.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
        await expect(row.getByRole("button", { name: /^(Open|Show in)/ })).toHaveCount(0);
    }
    await expect(ration).toContainText(hindiMessage);
    await expect(ration.getByRole("button", { name: "Install Hindi pack for ration-card-hindi.pdf", exact: true })).toHaveText("Install Hindi pack");
    await expect(receipt.getByRole("button", { name: "Update Gujarati pack for receipt-gujarati.pdf", exact: true })).toBeVisible();
    await expect(letter.getByRole("button", { name: "Repair Tamil pack for letter-tamil.pdf", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

    await ration.getByRole("button", { name: "Install Hindi pack for ration-card-hindi.pdf", exact: true }).click();
    await expect.poll(() => installs(page)).toEqual(["ocr-hindi"]);
    const runId = await page.evaluate(() => (window as unknown as PackResultsWindow).__installs[0].runId);

    // Both Hindi rows follow the one install, and keep their files.
    await emit(page, "pack-progress", { runId, packId: "ocr-hindi", phase: "downloading", receivedBytes: 7 * MB, totalBytes: 12 * MB });
    for (const row of [ration, notice]) {
        await expect(row.getByRole("progressbar", { name: "Installing Hindi pack" })).toHaveAttribute("aria-valuenow", "58");
        await expect(row).toContainText("7 of 12 MB");
        await expect(row.getByRole("button", { name: /Install Hindi pack/ })).toHaveCount(0);
    }
    await expect(summary(page)).toContainText("Installing the Hindi pack. The 2 Hindi files stay in the list.");
    // Progress for another run is not this install's.
    await emit(page, "pack-progress", { runId: "someone-else", packId: "ocr-hindi", phase: "downloading", receivedBytes: 1 * MB, totalBytes: 12 * MB });
    await expect(ration).toContainText("7 of 12 MB");

    await emit(page, "pack-changed", { packId: "ocr-hindi" });
    await page.evaluate(() => (window as unknown as PackResultsWindow).__installs[0].settle());

    await expect(rows).toHaveCount(6);
    for (const [row, name] of [[ration, "ration-card-hindi.pdf"], [notice, "school-notice-hindi.pdf"]] as const) {
        await expect(row).toContainText(name);
        await expect(row).toHaveAttribute("data-tone", "ok");
        await expect(row).toContainText("Ready to run");
        await expect(row).toContainText("The Hindi pack is installed now. Run again to read this file.");
        await expect(row).toContainText("Pack installed");
        await expect(row.getByRole("progressbar")).toHaveCount(0);
        await expect(row.getByRole("button")).toHaveCount(0);
    }
    await expect(summary(page)).toContainText("Hindi pack installed. 2 files are ready to run again.");
    // The other packs are untouched, and nothing offers Retry.
    await expect(receipt).toContainText("Needs a pack");
    await expect(letter.getByRole("button", { name: "Repair Tamil pack for letter-tamil.pdf", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
});

test("a pack install that fails says why on the row and offers the pack action again", async ({ page }) => {
    const rows = await runBatch(page, mixedBatch);
    const letter = rows.nth(4);
    await letter.getByRole("button", { name: "Repair Tamil pack for letter-tamil.pdf", exact: true }).click();
    await expect(letter.getByRole("progressbar", { name: "Repairing Tamil pack" })).toBeVisible();
    await page.evaluate(() => (window as unknown as PackResultsWindow).__installs[0].settle({ kind: "offline", message: "You're offline. Connect to the internet, then try again." }));

    await expect(letter.getByRole("alert")).toHaveText("You're offline. Connect to the internet, then try again.");
    await expect(letter).toContainText("Needs a pack");
    await expect(letter.getByRole("button", { name: "Repair Tamil pack for letter-tamil.pdf", exact: true })).toBeEnabled();
    await expect(letter.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
});
