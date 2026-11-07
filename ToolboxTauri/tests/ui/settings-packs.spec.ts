import { expect, test, type Page } from "@playwright/test";
import type { PackEntry, PackList } from "../../src/views/PacksSection";

type PacksWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __packs: PackList | string;
};

const MB = 1024 * 1024;
const license = "Open-source license. Free to use, including for work.";
const pack = (fields: Partial<PackEntry> & Pick<PackEntry, "id" | "name" | "status">): PackEntry => ({
    description: "Reads text in scanned pages.",
    toolIds: ["pdf-ocr"],
    license,
    version: "2.0",
    downloadBytes: 12 * MB,
    installedBytes: 31 * MB,
    diskBytes: 0,
    installedVersion: null,
    ...fields,
});

const everyStatus: PackList = {
    packs: [
        pack({ id: "scripts", name: "More writing systems", status: "installed", toolIds: ["pdf-edit"], description: "Shows and exports text in more scripts.", version: "1.4", installedVersion: "1.4", diskBytes: 96 * MB }),
        pack({ id: "ocr-hindi", name: "Text recognition, Hindi", status: "notInstalled" }),
        pack({ id: "ocr-gujarati", name: "Text recognition, Gujarati", status: "updateAvailable", version: "3.2", installedVersion: "3.1", downloadBytes: 12 * MB, diskBytes: 300 * MB }),
        pack({ id: "ocr-french", name: "Text recognition, French", status: "damaged", installedVersion: "2.0", diskBytes: 9 * MB }),
        pack({ id: "ocr-german", name: "Text recognition, German", status: "incompatible", toolIds: ["pdf-ocr", "pdf-edit"], installedVersion: "2.6", downloadBytes: null, installedBytes: null, diskBytes: 10 * MB }),
    ],
    installedCount: 4,
    totalBytes: 415 * MB,
};

const openPacks = async (page: Page, result: PackList | string) => {
    await page.addInitScript((result) => { (window as unknown as PacksWindow).__packs = result; }, result);
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Packs", exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Packs", exact: true })).toBeVisible();
    return page.getByRole("region", { name: "Packs", exact: true });
};

const invocations = (page: Page) => page.evaluate(() => (window as unknown as PacksWindow).__invocations.map(({ command }) => command));

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as PacksWindow;
        win.__invocations = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "list_packs") {
                    if (typeof win.__packs === "string") throw win.__packs;
                    return win.__packs;
                }
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "plugin:app|version") return "9.4.2";
                return null;
            },
            transformCallback: () => 0,
            unregisterCallback: () => undefined,
            convertFileSrc: (path: string) => path,
        } as unknown as typeof window.__TAURI_INTERNALS__;
    });
});

test("Packs shows every status with sizes, license, tools, total disk usage and the privacy statement", async ({ page }) => {
    const region = await openPacks(page, everyStatus);
    await expect(page.getByText("Optional downloads for extra formats and languages.")).toBeVisible();
    await expect(region.getByRole("heading", { level: 2, name: "Optional packs" })).toBeVisible();
    await expect(region).toContainText("Each one downloads once from the Toolbox release server, is checked before it's used, and then works offline. Your files never leave this device.");

    const list = region.getByRole("list", { name: "Packs" });
    await expect(list.getByRole("listitem")).toHaveCount(5);
    const rows = {
        installed: list.getByRole("listitem", { name: "More writing systems, Installed, version 1.4" }),
        notInstalled: list.getByRole("listitem", { name: "Text recognition, Hindi, Not installed" }),
        update: list.getByRole("listitem", { name: "Text recognition, Gujarati, Version 3.1. Update to 3.2, 12 MB" }),
        damaged: list.getByRole("listitem", { name: "Text recognition, French, Damaged. A file in this pack was changed or is missing." }),
        incompatible: list.getByRole("listitem", { name: "Text recognition, German, Doesn't work with this version of Toolbox. Update Toolbox to use it." }),
    };
    await expect(rows.installed).toContainText("Used by Edit PDF. 96 MB on this disk.");
    await expect(rows.notInstalled).toContainText("Used by OCR PDF. 12 MB download.");
    await expect(rows.update).toContainText("Used by OCR PDF. 300 MB on this disk.");
    await expect(rows.damaged).toContainText("Used by OCR PDF. 9 MB on this disk.");
    await expect(rows.incompatible).toContainText("Used by OCR PDF and Edit PDF. 10 MB on this disk.");
    await expect(rows.damaged.getByRole("status")).toHaveCSS("color", await page.evaluate(() => {
        const probe = document.createElement("span");
        probe.style.color = "var(--destructive)";
        document.body.append(probe);
        const color = getComputedStyle(probe).color;
        probe.remove();
        return color;
    }));

    await expect(region.getByRole("status").last()).toHaveText("4 installed, 415 MB used.");

    // Read-only: rows offer details and nothing that installs, updates, repairs or removes.
    await expect(list.getByRole("button")).toHaveCount(5);
    for (const button of await list.getByRole("button").all()) await expect(button).toHaveAccessibleName(/^Details for /);

    const details = rows.notInstalled.getByRole("button", { name: "Details for Text recognition, Hindi" });
    await expect(details).toHaveAttribute("aria-expanded", "false");
    await details.click();
    await expect(details).toHaveAttribute("aria-expanded", "true");
    const facts = rows.notInstalled.getByRole("definition");
    await expect(facts).toHaveText(["Reads text in scanned pages.", "OCR PDF", "12 MB", "Nothing yet", "2.0", license]);

    await rows.update.getByRole("button", { name: "Details for Text recognition, Gujarati" }).click();
    await expect(rows.update.getByRole("definition")).toHaveText(["Reads text in scanned pages.", "OCR PDF", "12 MB", "300 MB", "3.1, 3.2 is ready", license]);

    // A pack with no payload for this computer has no download size to show.
    await rows.incompatible.getByRole("button", { name: "Details for Text recognition, German" }).click();
    await expect(rows.incompatible.getByRole("definition")).toHaveText(["Reads text in scanned pages.", "OCR PDF and Edit PDF", "10 MB", "2.6", license]);

    await details.click();
    await expect(rows.notInstalled.getByRole("definition")).toHaveCount(0);
});

test("opening Packs sends exactly one list-packs request and no install request", async ({ page }) => {
    const region = await openPacks(page, everyStatus);
    await expect(region.getByRole("listitem")).toHaveCount(5);
    await region.getByRole("button", { name: "Details for More writing systems" }).click();
    const commands = await invocations(page);
    expect(commands.filter((command) => command === "list_packs")).toHaveLength(1);
    expect(commands.filter((command) => /pack|install|download|repair|remove/i.test(command) && command !== "list_packs")).toEqual([]);
});

test("Packs with nothing installed says so", async ({ page }) => {
    const region = await openPacks(page, { packs: [pack({ id: "ocr-hindi", name: "Text recognition, Hindi", status: "notInstalled" })], installedCount: 0, totalBytes: 0 });
    await expect(region.getByRole("listitem", { name: "Text recognition, Hindi, Not installed" })).toBeVisible();
    await expect(region.getByRole("status").last()).toHaveText("No packs installed.");
});

test("Packs explains when the list cannot be read", async ({ page }) => {
    const region = await openPacks(page, "The pack catalog is not valid");
    await expect(region.getByRole("alert")).toHaveText("Could not load packs.");
    await expect(region.getByRole("listitem")).toHaveCount(0);
});
