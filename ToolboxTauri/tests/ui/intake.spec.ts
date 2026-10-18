import { expect, test, type Page } from "@playwright/test";

type PathKind = "file" | "folder" | "unreadable";

type IntakeWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __pathKinds: Record<string, PathKind>;
    __dialogResult: string | string[] | null;
    __processingDelayMs: number;
    __drop: (paths: string[]) => void;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as IntakeWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        win.__invocations = [];
        win.__pathKinds = {};
        win.__dialogResult = null;
        win.__processingDelayMs = 0;
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        win.__drop = (paths) => {
            for (const [id, listener] of listeners) {
                if (listener.event === "tauri://drag-drop") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { paths, position: { x: 10, y: 10 } } });
            }
        };
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
                if (command === "inspect_intake_paths") {
                    return (args as { paths: string[] }).paths.map((path) => ({ path, kind: win.__pathKinds[path] ?? "file" }));
                }
                if (command === "plugin:dialog|open") return win.__dialogResult;
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:")) return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                if (win.__processingDelayMs) await new Promise((resolve) => window.setTimeout(resolve, win.__processingDelayMs));
                const { request } = args as { request: { paths: string[] } };
                return request.paths.map((inputPath) => ({ inputPath, outputPaths: [`${inputPath}.out`], detail: "Test output", state: "succeeded", failure: null, reason: null, warnings: [] }));
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

const open = async (page: Page, title: string) => {
    await page.goto("/");
    await page.getByRole("button", { name: `Open ${title}`, exact: true }).click();
};

const drop = async (page: Page, paths: string[]) => {
    await expect.poll(() => page.evaluate(() => (window as unknown as IntakeWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "tauri://drag-drop"))).toBe(true);
    await page.evaluate((paths) => (window as unknown as IntakeWindow).__drop(paths), paths);
};

const setKinds = (page: Page, kinds: Record<string, PathKind>) =>
    page.evaluate((kinds) => { (window as unknown as IntakeWindow).__pathKinds = kinds; }, kinds);

const runAndSave = async (page: Page) => {
    await page.locator(".workspace-primary-action").click();
    await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
};

const rows = (page: Page) => page.getByRole("listbox").getByRole("option");

const requestFor = async (page: Page, command: string) => {
    const invocations = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations);
    return invocations.filter((invocation) => invocation.command === command).map(({ args }) => (args as { request: { paths: string[] } }).request);
};

test("dropped paths are classified, and only files the active tool takes are sent", async ({ page }) => {
    await open(page, "Protect PDF");
    await setKinds(page, { "/local/Scans": "folder", "/local/locked.pdf": "unreadable" });

    await drop(page, ["/local/lease.pdf", "/local/notes.txt", "/local/Scans", "/local/locked.pdf", "/local/lease.pdf", "/local/budget.docx", "/local/plan.pdf"]);

    await expect(rows(page)).toHaveText([/^lease\.pdf/, /^budget\.docx/, /^plan\.pdf/]);
    const notAdded = page.getByRole("region", { name: "Not added" });
    await expect(notAdded.getByText("4 items not added")).toBeVisible();
    await expect(notAdded.getByRole("listitem")).toHaveText([
        /notes\.txt.*only takes \.pdf, \.doc, \.docx, \.xls, \.xlsx, \.ppt, \.pptx files\./,
        /Scans.*Folders aren't added\. Open the folder and drop the files inside it\./,
        /locked\.pdf.*Couldn't open it\./,
        /lease\.pdf.*Already in the list, so it was added once\./,
    ]);
    await expect(rows(page).filter({ hasText: "budget.docx" })).toContainText("Won't run");
    await expect(page.getByText("1 file won't run with Protect PDF. It stays in the list for other actions.")).toBeVisible();

    await page.locator('input[type="password"]').fill("test-password");
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByText("Test output").first()).toBeVisible();
    expect((await requestFor(page, "protect_pdf")).map((request) => request.paths)).toEqual([["/local/lease.pdf", "/local/plan.pdf"]]);

    await notAdded.getByRole("button", { name: "Dismiss" }).click();
    await expect(notAdded).toHaveCount(0);
});

test("ordered tools keep repeated drops in the order they arrive", async ({ page }) => {
    await open(page, "Merge PDF");
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await drop(page, ["/local/a.pdf"]);

    await expect(rows(page)).toHaveCount(3);
    await expect(rows(page).nth(2)).toContainText("Added twice");
    const warningColor = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--warning").trim());
    const badgeColor = await rows(page).nth(2).getByText("Added twice").evaluate((element) => getComputedStyle(element).color);
    const probe = await page.evaluate((color) => { const span = document.createElement("span"); span.style.color = color; document.body.append(span); const value = getComputedStyle(span).color; span.remove(); return value; }, warningColor);
    expect(badgeColor).toBe(probe);
    await expect(page.getByRole("region", { name: "Not added" })).toHaveCount(0);

    await runAndSave(page);
    await expect(page.getByText(/^Done\./)).toBeVisible();
    expect((await requestFor(page, "merge_pdfs")).map((request) => request.paths)).toEqual([["/local/a.pdf", "/local/b.pdf", "/local/a.pdf"]]);
});

test("the picker filters by the active tool and allows one file only for single-input tools", async ({ page }) => {
    await open(page, "Merge PDF");
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/a.pdf", "/local/b.pdf"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(rows(page)).toHaveCount(2);

    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "Images to PDF", exact: true }).click();
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/scan.png"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(rows(page)).toHaveCount(3);

    await page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "Compress PDF", exact: true }).click();
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = "/local/c.pdf"; });
    await page.getByRole("button", { name: "Choose files to process" }).click();

    const dialogs = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations
        .filter(({ command }) => command === "plugin:dialog|open")
        .map(({ args }) => (args as { options: { multiple: boolean; filters: Array<{ extensions: string[] }> } }).options));
    expect(dialogs.map(({ multiple, filters }) => ({ multiple, extensions: filters[0].extensions }))).toEqual([
        { multiple: true, extensions: ["pdf"] },
        { multiple: true, extensions: ["png", "jpg", "jpeg", "webp", "heic"] },
        { multiple: false, extensions: ["pdf"] },
    ]);
    await expect(rows(page)).toHaveText([/^c\.pdf/]);
});

test("click, Shift-click and Cmd-click select rows, and removal only edits the list", async ({ page }) => {
    await open(page, "Protect PDF");
    await drop(page, ["/local/one.pdf", "/local/two.pdf", "/local/three.pdf", "/local/four.pdf", "/local/five.pdf"]);
    const item = (name: string) => rows(page).filter({ hasText: name }).getByRole("button", { name: new RegExp(`^${name}`) });

    await item("one.pdf").click();
    await item("three.pdf").click({ modifiers: ["Shift"] });
    await expect(page.getByText("3 selected")).toBeVisible();
    await item("two.pdf").click({ modifiers: ["Meta"] });
    await expect(page.getByText("2 selected")).toBeVisible();
    await expect(rows(page).filter({ hasText: "two.pdf" })).toHaveAttribute("aria-selected", "false");

    const before = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations.length);
    await page.getByRole("button", { name: "Remove from list" }).click();
    await expect(rows(page)).toHaveText([/^two\.pdf/, /^four\.pdf/, /^five\.pdf/]);

    await item("five.pdf").click();
    await item("five.pdf").press("Delete");
    await expect(rows(page)).toHaveText([/^two\.pdf/, /^four\.pdf/]);
    await page.getByRole("button", { name: "Remove two.pdf from list" }).click();
    await expect(rows(page)).toHaveText([/^four\.pdf/]);
    const after = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations.slice(0));
    expect(after.slice(before).map(({ command }) => command)).toEqual([]);
    await expect(page.getByText("Shift-click or ⌘-click to select. Delete removes files from the list, not from disk.")).toBeVisible();
});

test("drag and keyboard reorder set the order sent in the request", async ({ page }) => {
    await open(page, "Merge PDF");
    await drop(page, ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"]);

    await page.getByRole("button", { name: "Drag c.pdf to reorder" }).hover();
    await page.mouse.down();
    await rows(page).nth(0).hover();
    await page.mouse.up();
    await expect(rows(page)).toHaveText([/c\.pdf/, /a\.pdf/, /b\.pdf/]);
    await expect(page.getByRole("status").filter({ hasText: "Moved" })).toHaveText("Moved c.pdf to position 1.");

    const b = rows(page).filter({ hasText: "b.pdf" }).getByRole("button", { name: /^b\.pdf/ });
    await b.click();
    await b.press("Meta+ArrowUp");
    await expect(rows(page)).toHaveText([/c\.pdf/, /b\.pdf/, /a\.pdf/]);
    await expect(page.getByRole("status").filter({ hasText: "Moved" })).toHaveText("Moved b.pdf to position 2.");
    await expect(rows(page).nth(1)).toHaveAttribute("aria-label", "2 of 3, b.pdf");

    await page.getByRole("button", { name: "Move a.pdf up" }).click();
    await expect(rows(page)).toHaveText([/c\.pdf/, /a\.pdf/, /b\.pdf/]);
    await expect(page.getByRole("button", { name: "Move c.pdf up" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Move b.pdf down" })).toBeDisabled();

    await runAndSave(page);
    await expect(page.getByText(/^Done\./)).toBeVisible();
    expect((await requestFor(page, "merge_pdfs")).map((request) => request.paths)).toEqual([["/local/c.pdf", "/local/a.pdf", "/local/b.pdf"]]);
});

test("intake, options and the file list lock while a run is in progress", async ({ page }) => {
    await open(page, "Merge PDF");
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await page.evaluate(() => { (window as unknown as IntakeWindow).__processingDelayMs = 1500; });

    await runAndSave(page);
    await expect(page.getByRole("button", { name: "Choose files to process" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Clear list" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Remove from list" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Move a.pdf down" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Drag a.pdf to reorder" })).toBeDisabled();
    await expect(rows(page).first().getByRole("button", { name: /^a\.pdf/ })).toBeDisabled();
    await expect(page.getByRole("toolbar", { name: "PDF conversion tools" }).getByRole("button", { name: "Split PDF", exact: true })).toBeDisabled();
    await drop(page, ["/local/late.pdf"]);
    await expect(rows(page)).toHaveCount(2);

    await expect(page.getByText(/^Done\./)).toBeVisible();
    await expect(page.getByRole("button", { name: "Choose files to process" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "Move a.pdf down" })).toBeEnabled();
    await drop(page, ["/local/late.pdf"]);
    await expect(rows(page)).toHaveCount(3);
});
