import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

type Facts = { pageCount: number | null; modifiedMs: number | null; sizeBytes: number | null };

type MergeWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResult: string | string[] | null;
    __facts: Record<string, Facts>;
    // The output name the native side reports writing, when it differs from the chosen name.
    __writtenName: string | null;
    // A merge that fails on this input, worded as the native aggregate failure.
    __failingInput: string | null;
    // Outcomes the native side reports for the next merge, one per entry, in place of the default success.
    __mergeOutcomes: unknown[] | null;
    __drop: (paths: string[]) => void;
};

const day = (iso: string) => Date.parse(`${iso}T12:00:00Z`);

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as MergeWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__dialogResult = null;
        win.__facts = {};
        win.__writtenName = null;
        win.__failingInput = null;
        win.__mergeOutcomes = null;
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        win.__drop = (paths) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "toolbox://shell-event") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: "dropped-files", paths } });
            }
        };
        const name = (path: string) => path.split("/").pop() ?? path;
        const folder = (path: string) => path.slice(0, path.lastIndexOf("/"));
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
                if (command === "describe_intake_files") {
                    return (args as { paths: string[] }).paths.map((path) => ({ path, ...(win.__facts[path] ?? { pageCount: null, modifiedMs: null, sizeBytes: null }) }));
                }
                if (command === "plugin:dialog|open") return win.__dialogResult;
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "shell_ready") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                if (command !== "merge_pdfs") throw `Unexpected command ${command}`;
                const { request } = args as { request: { paths: string[]; outputPath: string } };
                if (win.__mergeOutcomes) return win.__mergeOutcomes;
                const failing = win.__failingInput;
                if (failing && request.paths.includes(failing)) {
                    const failedAt = request.paths.indexOf(failing);
                    const repeated = request.paths.filter((path) => path === failing).length > 1;
                    const cause = repeated ? `${name(failing)} (entry ${failedAt + 1})` : name(failing);
                    return request.paths.map((inputPath, index) => ({
                        inputPath, outputPaths: [], detail: "", state: "failed", reason: null, warnings: [],
                        failure: {
                            kind: "processing",
                            message: index === failedAt ? `Could not read ${failing}: invalid file header` : `Stopped because ${cause} failed. Nothing was saved.`,
                            pointer: null,
                        },
                    }));
                }
                const chosenName = name(request.outputPath);
                const finalName = win.__writtenName ?? chosenName;
                const warnings = finalName === chosenName ? [] : [{ code: "renamedToAvoidReplacing", message: "native wording", details: { chosenName, finalName } }];
                return [{ inputPath: request.paths[0], outputPaths: [`${folder(request.outputPath)}/${finalName}`], detail: "", state: "succeeded", failure: null, reason: null, warnings }];
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

const openMerge = async (page: Page, facts: Record<string, Facts> = {}) => {
    await page.goto("/");
    await page.evaluate((facts) => { (window as unknown as MergeWindow).__facts = facts; }, facts);
    await openToolBySearch(page, "Merge PDF");
};

const drop = async (page: Page, paths: string[]) => {
    await expect.poll(() => page.evaluate(() => (window as unknown as MergeWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as MergeWindow).__drop(paths), paths);
};

const rows = (page: Page) => page.getByRole("listbox", { name: "Merge order, top joins first" }).getByRole("option");
const mergeButton = (page: Page) => page.locator(".workspace-primary-action");

const mergeRequests = async (page: Page) => {
    const invocations = await page.evaluate(() => (window as unknown as MergeWindow).__invocations);
    return invocations.filter(({ command }) => command === "merge_pdfs").map(({ args }) => (args as { request: { paths: string[]; outputPath: string } }).request);
};

const mergeAndSave = async (page: Page) => {
    const before = (await mergeRequests(page)).length;
    await mergeButton(page).click();
    await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => (await mergeRequests(page)).length).toBe(before + 1);
    return (await mergeRequests(page)).at(-1)!.paths;
};

const files = {
    a: "/local/merge/a.pdf",
    b: "/local/merge/b.pdf",
    c: "/local/merge/c.pdf",
};
const facts: Record<string, Facts> = {
    [files.a]: { pageCount: 2, modifiedMs: day("2026-10-02"), sizeBytes: 1_400_000 },
    [files.b]: { pageCount: 1, modifiedMs: day("2026-09-15"), sizeBytes: 380_000 },
    [files.c]: { pageCount: 5, modifiedMs: day("2026-10-20"), sizeBytes: 52_000 },
};

test("dropped PDFs list in drop order with each file's pages, size and date", async ({ page }) => {
    await openMerge(page, facts);
    await drop(page, [files.c, files.a, files.b]);

    await expect(rows(page)).toHaveText([/c\.pdf/, /a\.pdf/, /b\.pdf/]);
    await expect(rows(page).nth(0)).toHaveAttribute("aria-label", "1. c.pdf, 5 pages");
    await expect(rows(page).nth(2)).toHaveAttribute("aria-label", "3. b.pdf, 1 page");
    await expect(rows(page).nth(1)).toContainText("/local/merge · Modified Oct 2, 2026");
    await expect(rows(page).nth(1)).toContainText("2 pages");
    await expect(rows(page).nth(1)).toContainText("1.4 MB");
    await expect(rows(page).nth(2)).toContainText("380 KB");
    await expect(page.getByText("8 pages in all")).toBeVisible();
    await expect(page.getByText("Drag a row by its handle, or select it and press ⌘+↑ or ⌘+↓. The top file comes first.")).toBeVisible();

    expect(await mergeAndSave(page)).toEqual([files.c, files.a, files.b]);
});

test("Merge needs two PDFs, says why, and points to Organize Pages", async ({ page }) => {
    await openMerge(page, facts);
    await expect(page.getByText("Files join top to bottom, every page of each. A file can be listed twice.")).toBeVisible();
    await expect(page.getByText("Need only some pages, or a page order? Use Organize Pages.")).toBeVisible();
    await expect(mergeButton(page)).toBeDisabled();
    await expect(page.getByText("Add at least 2 PDFs.")).toBeVisible();

    await drop(page, [files.a]);
    await expect(mergeButton(page)).toBeDisabled();
    await expect(page.getByText("Merge needs at least 2 PDFs.")).toBeVisible();
    await expect(page.getByText("Add at least one more PDF. You can add the same file again to repeat it.")).toBeVisible();

    await drop(page, [files.b]);
    await expect(mergeButton(page)).toBeEnabled();
    await expect(page.getByText("Merge needs at least 2 PDFs.")).toHaveCount(0);

    await page.getByRole("button", { name: "Open Organize Pages" }).click();
    // Organize works on one PDF at a time, so the two listed files arrive in its chooser.
    const chooser = page.getByRole("dialog", { name: "Organize works on one PDF at a time" });
    await expect(chooser).toBeVisible();
    await expect(chooser.getByRole("radio", { name: "a.pdf" })).toBeChecked();
    await expect(chooser.getByRole("radio", { name: "b.pdf" })).toBeVisible();
    await chooser.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(chooser).toBeHidden();
    await expect(page.getByRole("button", { name: "Add Organize PDF to favorites" })).toBeVisible();
    expect(await mergeRequests(page)).toEqual([]);
});

test("drag, the keyboard, the move buttons and both sorts set the order in the request", async ({ page }) => {
    await openMerge(page, facts);
    await drop(page, [files.a, files.b, files.c]);

    await page.getByRole("button", { name: "Drag c.pdf to reorder" }).hover();
    await page.mouse.down();
    await rows(page).nth(0).hover();
    await page.mouse.up();
    await expect(rows(page).nth(0)).toHaveAttribute("aria-label", "1. c.pdf, 5 pages");
    expect(await mergeAndSave(page)).toEqual([files.c, files.a, files.b]);

    const b = rows(page).filter({ hasText: "b.pdf" }).getByRole("button", { name: /^b\.pdf/ });
    await b.click();
    await b.press("Meta+ArrowUp");
    await expect(page.getByRole("status").filter({ hasText: "Moved" })).toHaveText("Moved b.pdf to position 2.");
    expect(await mergeAndSave(page)).toEqual([files.c, files.b, files.a]);

    await page.getByRole("button", { name: "Move c.pdf down" }).click();
    expect(await mergeAndSave(page)).toEqual([files.b, files.c, files.a]);

    const sort = page.getByLabel("Sort");
    await sort.selectOption({ label: "Name, A to Z" });
    expect(await mergeAndSave(page)).toEqual([files.a, files.b, files.c]);
    await sort.selectOption({ label: "Name, Z to A" });
    expect(await mergeAndSave(page)).toEqual([files.c, files.b, files.a]);
    await sort.selectOption({ label: "Date modified, oldest first" });
    expect(await mergeAndSave(page)).toEqual([files.b, files.a, files.c]);
    await sort.selectOption({ label: "Date modified, newest first" });
    expect(await mergeAndSave(page)).toEqual([files.c, files.a, files.b]);
    await sort.selectOption({ label: "Order added" });
    expect(await mergeAndSave(page)).toEqual([files.a, files.b, files.c]);
});

test("a repeated file is sent twice, and removing one copy keeps the other", async ({ page }) => {
    await openMerge(page, facts);
    await drop(page, [files.a, files.b]);
    await drop(page, [files.a]);

    await expect(rows(page)).toHaveCount(3);
    await expect(rows(page).nth(0)).toContainText("Copy 1 of 2");
    await expect(rows(page).nth(2)).toContainText("Copy 2 of 2");
    await expect(page.getByRole("region", { name: "Not added" })).toHaveCount(0);
    await expect(page.getByText("5 pages in all")).toBeVisible();
    expect(await mergeAndSave(page)).toEqual([files.a, files.b, files.a]);

    await page.getByRole("button", { name: "Remove a.pdf from list" }).nth(1).click();
    await expect(rows(page)).toHaveCount(2);
    await expect(rows(page).nth(0)).toHaveAttribute("aria-label", "1. a.pdf, 2 pages");
    await expect(rows(page).nth(0)).not.toContainText("Copy");
    expect(await mergeAndSave(page)).toEqual([files.a, files.b]);
});

test("cancelling the save dialog sends nothing, and Save sends the chosen path and shows the name written", async ({ page }) => {
    await openMerge(page, facts);
    await drop(page, [files.a, files.b]);

    await mergeButton(page).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);
    expect(await mergeRequests(page)).toEqual([]);
    await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveCount(0);

    await page.evaluate(() => {
        const win = window as unknown as MergeWindow;
        win.__dialogResult = "/Users/test/Merged";
        win.__writtenName = "Packet-1.pdf";
    });
    await mergeButton(page).click();
    await dialog.getByRole("button", { name: "Change…" }).click();
    await dialog.getByLabel("File name").fill("Packet.pdf");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => (await mergeRequests(page)).length).toBe(1);
    expect((await mergeRequests(page))[0]).toMatchObject({ paths: [files.a, files.b], outputPath: "/Users/test/Merged/Packet.pdf" });

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(1);
    await expect(results.nth(0).locator(".result-row-name")).toHaveText("Packet-1.pdf");
    await expect(results.nth(0).getByText("Saved as Packet-1.pdf. Renamed to avoid replacing an existing file.")).toBeVisible();
});

test("a failed merge names the input that stopped it on every row", async ({ page }) => {
    await openMerge(page, facts);
    const damaged = "/local/merge/site-photos.pdf";
    await page.evaluate((path) => { (window as unknown as MergeWindow).__failingInput = path; }, damaged);
    await drop(page, [files.a, damaged, files.b]);

    expect(await mergeAndSave(page)).toEqual([files.a, damaged, files.b]);
    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(3);
    await expect(results.nth(1)).toContainText(`Could not read ${damaged}`);
    await expect(results.nth(0)).toContainText("Stopped because site-photos.pdf failed. Nothing was saved.");
    await expect(results.nth(2)).toContainText("Stopped because site-photos.pdf failed. Nothing was saved.");
    await expect(rows(page)).toHaveCount(3);
});

const succeeded = (inputPath: string, detail: string, warnings: unknown[]) => ({
    inputPath, outputPaths: ["/local/merge/Merged.pdf"], detail, state: "succeeded", failure: null, reason: null, warnings,
});
const signatureRemoved = { code: "signatureRemoved", message: "native wording", details: {} };
const notCarried = (code: "bookmarksNotCarried" | "tagsNotCarried") => ({ code, message: "native wording", details: { into: "the merged file" } });

test("a signed entry's warning shows on its own row only, and what wasn't carried is stated", async ({ page }) => {
    await openMerge(page, facts);
    await page.evaluate((outcomes) => { (window as unknown as MergeWindow).__mergeOutcomes = outcomes; }, [
        succeeded(files.a, "Pages 1–2 of Merged.pdf.", [notCarried("bookmarksNotCarried")]),
        succeeded(files.b, "Page 3 of Merged.pdf.", [signatureRemoved, notCarried("tagsNotCarried")]),
        succeeded(files.c, "Pages 4–8 of Merged.pdf.", []),
    ]);
    await drop(page, [files.a, files.b, files.c]);
    await mergeAndSave(page);

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(3);
    const signature = "Saved. The digital signature was removed, because changing the file breaks it.";
    await expect(results.nth(1)).toContainText(signature);
    await expect(results.nth(0)).not.toContainText(signature);
    await expect(results.nth(2)).not.toContainText(signature);
    await expect(results.nth(0)).toContainText("Bookmarks weren’t carried into the merged file.");
    await expect(results.nth(1)).toContainText("Accessibility tags weren’t carried into the merged file.");
    await expect(results.nth(2)).toContainText("Pages 4–8 of Merged.pdf.");
    await expect(results.nth(2)).not.toContainText("carried");
});

test("a password-protected entry points to Lock & Unlock, and the pointer opens it", async ({ page }) => {
    await openMerge(page, facts);
    const locked = "/local/merge/locked.pdf";
    await page.evaluate(({ a, locked }) => { (window as unknown as MergeWindow).__mergeOutcomes = [
        { inputPath: a, outputPaths: [], detail: "", state: "failed", reason: null, warnings: [],
            failure: { kind: "processing", message: "Stopped because locked.pdf failed. Nothing was saved.", pointer: null } },
        { inputPath: locked, outputPaths: [], detail: "", state: "failed", reason: null, warnings: [],
            failure: { kind: "invalidInput", message: "Password-protected, so it can't be read. Remove the password, then add it again.", pointer: { kind: "tool", toolId: "lock-unlock" } } },
    ]; }, { a: files.a, locked });
    await drop(page, [files.a, locked]);
    await mergeAndSave(page);

    const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
    await expect(results).toHaveCount(2);
    await expect(results.nth(1)).toContainText("Password-protected, so it can't be read. Remove the password, then add it again.");
    await expect(results.nth(0).getByRole("button", { name: "Open Lock & Unlock" })).toHaveCount(0);
    await results.nth(1).getByRole("button", { name: "Open Lock & Unlock" }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
});
