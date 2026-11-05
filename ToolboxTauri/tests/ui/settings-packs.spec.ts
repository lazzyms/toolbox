import { expect, test, type Page } from "@playwright/test";
import type { PackEntry, PackFileSummary, PackInstallError, PackList } from "../../src/views/PacksSection";

type Outcome<T> = { ok: T } | { error: PackInstallError };

type PacksWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __packs: PackList | string;
    __dialog: Array<string | null>;
    __inspect: Outcome<PackFileSummary>;
    __install: Outcome<PackList>;
    __finishInstall?: () => void;
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
        win.__dialog = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "list_packs") {
                    if (typeof win.__packs === "string") throw win.__packs;
                    return win.__packs;
                }
                if (command === "plugin:dialog|open") return win.__dialog.shift() ?? null;
                if (command === "inspect_pack_file") {
                    if ("error" in win.__inspect) throw win.__inspect.error;
                    return win.__inspect.ok;
                }
                if (command === "install_pack_file") {
                    // Held until the test finishes it, so the checking state can be seen.
                    await new Promise<void>((resolve) => { win.__finishInstall = resolve; });
                    if ("error" in win.__install) throw win.__install.error;
                    return win.__install.ok;
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

const hindiFile = "/Users/test/Downloads/text-recognition-hindi-2.0.toolboxpack";
const hindiSummary: PackFileSummary = { fileName: "text-recognition-hindi-2.0.toolboxpack", packId: "ocr-hindi", name: "Text recognition, Hindi", version: "2.0", installedBytes: 31 * MB, license };
const afterInstall: PackList = {
    ...everyStatus,
    packs: everyStatus.packs.map((entry) => entry.id === "ocr-hindi" ? { ...entry, status: "installed", installedVersion: "2.0", diskBytes: 31 * MB } : entry),
    installedCount: 5,
    totalBytes: 446 * MB,
};

const arrangeFile = async (page: Page, files: Array<string | null>, inspect: Outcome<PackFileSummary>, install: Outcome<PackList> = { ok: afterInstall }) => {
    await page.addInitScript(({ files, inspect, install }) => {
        const win = window as unknown as PacksWindow;
        win.__dialog = files;
        win.__inspect = inspect;
        win.__install = install;
    }, { files, inspect, install });
};

const calls = (page: Page, command: string) => page.evaluate((command) => (window as unknown as PacksWindow).__invocations.filter((call) => call.command === command).map((call) => call.args), command);

test("Install from file opens the native dialog, confirms the pack, shows it being checked and then installed", async ({ page }) => {
    await arrangeFile(page, [hindiFile], { ok: hindiSummary });
    const region = await openPacks(page, everyStatus);
    await region.getByRole("button", { name: "Install from file…" }).click();

    const [dialogArgs] = await calls(page, "plugin:dialog|open");
    expect(dialogArgs).toEqual({ options: { multiple: false, directory: false, filters: [{ name: "Toolbox pack", extensions: ["toolboxpack"] }] } });
    expect(await calls(page, "inspect_pack_file")).toEqual([{ path: hindiFile }]);

    const confirm = page.getByRole("dialog", { name: "Install this pack?" });
    await expect(confirm).toContainText("Toolbox checks the file's signature and contents before installing it. Nothing is downloaded, and your files never leave this device.");
    await expect(confirm.getByRole("term")).toHaveText(["File", "Pack", "Version", "Size", "License"]);
    await expect(confirm.getByRole("definition")).toHaveText(["text-recognition-hindi-2.0.toolboxpack", "Text recognition, Hindi", "2.0", "31 MB on this disk", license]);
    expect(await calls(page, "install_pack_file")).toEqual([]);
    await confirm.getByRole("button", { name: "Install", exact: true }).click();
    await expect(confirm).toBeHidden();

    const row = region.getByRole("listitem", { name: "Text recognition, Hindi, Checking the pack file…" });
    await expect(row.getByRole("progressbar", { name: "Checking Text recognition, Hindi" })).toBeVisible();
    await expect(row.getByRole("status")).toHaveText("Checking the pack file…");
    await expect(region.getByRole("button", { name: "Install from file…" })).toBeDisabled();
    expect(await calls(page, "install_pack_file")).toEqual([{ path: hindiFile }]);

    await page.evaluate(() => (window as unknown as PacksWindow).__finishInstall?.());
    const installed = region.getByRole("listitem", { name: "Text recognition, Hindi, Installed, version 2.0" });
    await expect(installed).toContainText("Used by OCR PDF. 31 MB on this disk.");
    await expect(installed.getByRole("progressbar")).toHaveCount(0);
    await expect(region.getByRole("status").last()).toHaveText("5 installed, 446 MB used.");
    await expect(region.getByRole("button", { name: "Install from file…" })).toBeEnabled();
});

test("closing the file dialog or cancelling the confirmation installs nothing", async ({ page }) => {
    await arrangeFile(page, [null, hindiFile], { ok: hindiSummary });
    const region = await openPacks(page, everyStatus);
    const installFromFile = region.getByRole("button", { name: "Install from file…" });
    await installFromFile.click();
    await expect.poll(() => calls(page, "plugin:dialog|open")).toHaveLength(1);
    expect(await calls(page, "inspect_pack_file")).toEqual([]);

    await installFromFile.click();
    await page.getByRole("dialog", { name: "Install this pack?" }).getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(await calls(page, "install_pack_file")).toEqual([]);
    await expect(region.getByRole("listitem", { name: "Text recognition, Hindi, Not installed" })).toBeVisible();
});

const notSigned = { title: "This file isn't a Toolbox pack", text: "It isn't signed by Toolbox, so it can't be installed. Nothing was installed. Only use pack files from the Toolbox releases page." };
const failedCheck = { title: "This pack file didn't pass its safety check", text: "It may be damaged or changed since it was made, so it was not installed. Copy the file again from the Toolbox releases page, then try again." };
const otherVersion = { title: "This pack file doesn't work with this version of Toolbox", text: "It was made for a different version of Toolbox, so it can't be installed. Nothing was installed. Get the pack file for this version from the Toolbox releases page." };
const installErrors: Array<[PackInstallError, { title: string; text: string }]> = [
    [{ kind: "badSignature", message: "" }, notSigned],
    [{ kind: "wrongKey", message: "" }, notSigned],
    [{ kind: "notAPack", message: "" }, notSigned],
    [{ kind: "checksumMismatch", message: "" }, failedCheck],
    [{ kind: "fileChecksumMismatch", details: { path: "tessdata/hin.traineddata" }, message: "" }, failedCheck],
    [{ kind: "fileSizeMismatch", details: { path: "tessdata/hin.traineddata" }, message: "" }, failedCheck],
    [{ kind: "overLimit", details: { limit: "compressionRatio" }, message: "" }, failedCheck],
    [{ kind: "unsafeEntry", details: { path: "../x" }, message: "" }, failedCheck],
    [{ kind: "malformed", message: "" }, failedCheck],
    [{ kind: "incompatibleFormat", message: "" }, otherVersion],
    [{ kind: "incompatibleApp", message: "" }, otherVersion],
    [{ kind: "unavailable", message: "" }, { title: "Toolbox couldn't install this pack", text: "Toolbox couldn't save the pack on this computer. Nothing was installed. Check that there's free disk space, then try again." }],
];

for (const [error, copy] of installErrors) {
    test(`a pack file rejected with ${error.kind} shows its error and leaves the pack as it was`, async ({ page }) => {
        await arrangeFile(page, [hindiFile], { ok: hindiSummary }, { error });
        const region = await openPacks(page, everyStatus);
        await region.getByRole("button", { name: "Install from file…" }).click();
        await page.getByRole("dialog", { name: "Install this pack?" }).getByRole("button", { name: "Install", exact: true }).click();
        await page.evaluate(() => (window as unknown as PacksWindow).__finishInstall?.());

        const alert = page.getByRole("alertdialog", { name: copy.title });
        await expect(alert).toContainText(copy.text);
        await expect(alert.getByRole("definition")).toHaveText(["text-recognition-hindi-2.0.toolboxpack"]);
        await alert.getByRole("button", { name: "OK" }).click();
        await expect(alert).toBeHidden();
        await expect(region.getByRole("listitem", { name: "Text recognition, Hindi, Not installed" })).toBeVisible();
        await expect(region.getByRole("progressbar")).toHaveCount(0);
    });
}

test("a pack file for another computer is refused before installing, and another file can be chosen", async ({ page }) => {
    const windowsFile = "/Users/test/Downloads/office-documents-3.2-windows-arm.toolboxpack";
    await arrangeFile(page, [windowsFile, hindiFile], { error: { kind: "wrongTarget", details: { target: "windows-aarch64" }, message: "" } });
    const region = await openPacks(page, everyStatus);
    await region.getByRole("button", { name: "Install from file…" }).click();

    const alert = page.getByRole("alertdialog", { name: "This pack file is for another kind of computer" });
    await expect(alert).toContainText("It was made for Windows, so it can't be installed here. Nothing was installed. Get the pack file for Mac from the Toolbox releases page.");
    await expect(alert.getByRole("definition")).toHaveText(["office-documents-3.2-windows-arm.toolboxpack"]);
    expect(await calls(page, "install_pack_file")).toEqual([]);

    await page.evaluate((summary) => { (window as unknown as PacksWindow).__inspect = { ok: summary }; }, hindiSummary);
    await alert.getByRole("button", { name: "Choose another file…" }).click();
    await expect(page.getByRole("dialog", { name: "Install this pack?" })).toBeVisible();
    expect(await calls(page, "inspect_pack_file")).toEqual([{ path: windowsFile }, { path: hindiFile }]);
});

test("a file that isn't a pack is refused when it is inspected", async ({ page }) => {
    await arrangeFile(page, ["/Users/test/Downloads/notes.toolboxpack"], { error: { kind: "notAPack", message: "" } });
    const region = await openPacks(page, everyStatus);
    await region.getByRole("button", { name: "Install from file…" }).click();
    const alert = page.getByRole("alertdialog", { name: notSigned.title });
    await expect(alert).toContainText(notSigned.text);
    await expect(alert.getByRole("definition")).toHaveText(["notes.toolboxpack"]);
});
