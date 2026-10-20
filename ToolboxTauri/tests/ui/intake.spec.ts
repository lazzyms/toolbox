import { expect, test, type Page } from "@playwright/test";

type PathKind = "file" | "folder" | "unreadable";

type IntakeWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __pathKinds: Record<string, PathKind>;
    __dialogResult: string | string[] | null;
    __processingDelayMs: number;
    __wrongPasswordPaths: string[];
    __invalidInputPaths: string[];
    __skippedPaths: Record<string, { code: string; message: string }>;
    __failures: Record<string, { kind: string; message: string }>;
    __warnings: Record<string, Array<{ code: string; message: string }>>;
    __outputSettings: unknown;
    __sent: string[];
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
        win.__wrongPasswordPaths = [];
        win.__invalidInputPaths = [];
        win.__skippedPaths = {};
        win.__failures = {};
        win.__warnings = {};
        win.__outputSettings = { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
        // Everything that leaves the page or lands in the console, so a test can prove no password does.
        win.__sent = [];
        const record = (value: unknown) => win.__sent.push(typeof value === "string" ? value : JSON.stringify(value) ?? String(value));
        const realFetch = window.fetch.bind(window);
        window.fetch = (input, init) => {
            record(String(input));
            record(init?.body ?? "");
            return realFetch(input, init);
        };
        navigator.sendBeacon = (url, data) => {
            record(String(url));
            record(data ?? "");
            return true;
        };
        const realSend = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.send = function (body) {
            record(body ?? "");
            return realSend.call(this, body);
        };
        for (const level of ["log", "info", "warn", "error", "debug"] as const) {
            const real = console[level].bind(console);
            console[level] = (...values: unknown[]) => {
                values.forEach(record);
                real(...values);
            };
        }
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
                if (command === "get_output_settings") return win.__outputSettings;
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:")) return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                if (win.__processingDelayMs) await new Promise((resolve) => window.setTimeout(resolve, win.__processingDelayMs));
                const { request } = args as { request: { paths: string[] } };
                return request.paths.map((inputPath) => win.__failures[inputPath]
                    ? { inputPath, outputPaths: [], detail: "", state: "failed", failure: { ...win.__failures[inputPath], pointer: null }, reason: null, warnings: [] }
                    : win.__skippedPaths[inputPath]
                    ? { inputPath, outputPaths: [], detail: "", state: "skipped", failure: null, reason: { ...win.__skippedPaths[inputPath], pointer: null }, warnings: [] }
                    : win.__wrongPasswordPaths.includes(inputPath)
                    ? { inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "wrongPassword", message: "Wrong password or unsupported encryption.", pointer: null }, reason: null, warnings: [] }
                    : win.__invalidInputPaths.includes(inputPath)
                        ? { inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "invalidInput", message: "Only PDF, Word, Excel, and PowerPoint files can be unlocked.", pointer: null }, reason: null, warnings: [] }
                        : { inputPath, outputPaths: [`${inputPath}.out`], detail: "Test output", state: "succeeded", failure: null, reason: null, warnings: win.__warnings[inputPath] ?? [] });
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
    await open(page, "Image Metadata");
    await setKinds(page, { "/local/Scans": "folder", "/local/locked.png": "unreadable" });

    await drop(page, ["/local/photo.png", "/local/notes.txt", "/local/Scans", "/local/locked.png", "/local/photo.png", "/local/clip.gif", "/local/scan.tiff"]);

    await expect(rows(page)).toHaveText([/^photo\.png/, /^clip\.gif/, /^scan\.tiff/]);
    const notAdded = page.getByRole("region", { name: "Not added" });
    await expect(notAdded.getByText("4 items not added")).toBeVisible();
    await expect(notAdded.getByRole("listitem")).toHaveText([
        /notes\.txt.*only takes .*files\./,
        /Scans.*Folders aren't added\. Open the folder and drop the files inside it\./,
        /locked\.png.*Couldn't open it\./,
        /photo\.png.*Already in the list, so it was added once\./,
    ]);
    await expect(rows(page).filter({ hasText: "clip.gif" })).toContainText("Won't run");
    await expect(page.getByText("1 file won't run with Image Metadata. It stays in the list for other actions.")).toBeVisible();

    await page.locator(".workspace-primary-action").click();
    await expect(page.getByText("Test output").first()).toBeVisible();
    expect((await requestFor(page, "image_metadata")).map((request) => request.paths)).toEqual([["/local/photo.png", "/local/scan.tiff"]]);

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

    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "Images to PDF", exact: true }).click();
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/scan.png"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(rows(page)).toHaveCount(3);

    await page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "Split PDF", exact: true }).click();
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
    await open(page, "Lock & Unlock");
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

test("in the file list Cmd+A selects every file, arrows move the selection and Backspace removes it", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/one.pdf", "/local/two.pdf", "/local/three.pdf"]);
    const item = (name: string) => rows(page).filter({ hasText: name }).getByRole("button", { name: new RegExp(`^${name}`) });

    await page.locator("body").press("Meta+a");
    await expect(page.getByText(/selected$/)).toHaveCount(0);

    await item("one.pdf").click();
    await item("one.pdf").press("Meta+a");
    await expect(page.getByText("3 selected")).toBeVisible();
    for (const row of await rows(page).all()) await expect(row).toHaveAttribute("aria-selected", "true");

    await page.keyboard.press("ArrowDown");
    await expect(page.getByText("1 selected")).toBeVisible();
    await expect(rows(page).filter({ hasText: "two.pdf" })).toHaveAttribute("aria-selected", "true");
    await expect(item("two.pdf")).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(rows(page).filter({ hasText: "one.pdf" })).toHaveAttribute("aria-selected", "true");
    await expect(item("one.pdf")).toBeFocused();

    await page.keyboard.press("Backspace");
    await expect(rows(page)).toHaveText([/^two\.pdf/, /^three\.pdf/]);
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
    await expect(page.getByRole("toolbar", { name: "PDF Conversion tools" }).getByRole("button", { name: "Split PDF", exact: true })).toBeDisabled();
    await drop(page, ["/local/late.pdf"]);
    await expect(rows(page)).toHaveCount(2);

    await expect(page.getByText(/^Done\./)).toBeVisible();
    await expect(page.getByRole("button", { name: "Choose files to process" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "Move a.pdf down" })).toBeEnabled();
    await drop(page, ["/local/late.pdf"]);
    await expect(rows(page)).toHaveCount(3);
});

type PasswordRequestShape = { paths: string[]; password: string; passwordOverrides: Record<string, string> };

const passwordRequests = async (page: Page, command: string) =>
    (await requestFor(page, command)).map((request) => {
        const { paths, password, passwordOverrides } = request as unknown as PasswordRequestShape;
        return { paths, password, passwordOverrides };
    });

const row = (page: Page, name: string) => rows(page).filter({ hasText: name });

const setOwnPassword = async (page: Page, name: string, password: string, mode: "lock" | "unlock") => {
    await row(page, name).locator(".file-selection-item").click();
    await page.getByRole("button", { name: `Set own password for ${name}`, exact: true }).click();
    const editor = page.getByRole("group", { name: `Own password for ${name}` });
    await editor.getByLabel(mode === "lock" ? "Own new password" : "Own current password", { exact: true }).fill(password);
    if (mode === "lock") await editor.getByLabel("Confirm", { exact: true }).fill(password);
    await editor.getByRole("button", { name: "Save password" }).click();
    await expect(editor).toHaveCount(0);
};

const fillBatchLock = async (page: Page, password: string) => {
    await page.getByLabel("New password for all files", { exact: true }).fill(password);
    await page.getByLabel("Confirm new password", { exact: true }).fill(password);
};

const results = (page: Page) => page.getByRole("list", { name: "Results" }).getByRole("listitem");

const modes = (page: Page) => page.getByRole("radiogroup", { name: "Mode" });

const openLock = async (page: Page) => {
    await open(page, "Lock & Unlock");
    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
};

const run = (page: Page) => page.locator(".workspace-primary-action");

test("a file's own password is sent as an override and every other file uses the batch password", async ({ page }) => {
    await openLock(page);
    await drop(page, ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"]);
    await fillBatchLock(page, "batch-secret");
    await expect(page.getByTestId("password-usage")).toHaveText("Used for all 3 files.");

    await page.getByLabel("Confirm new password", { exact: true }).fill("batch-secreT");
    await expect(page.getByRole("alert")).toHaveText("Passwords don't match. Type the new password again.");
    await expect(page.locator(".workspace-primary-action")).toBeDisabled();
    await page.getByLabel("Confirm new password", { exact: true }).fill("batch-secret");

    await setOwnPassword(page, "b.pdf", "own-secret", "lock");
    await expect(row(page, "b.pdf")).toContainText("Own password");
    await expect(row(page, "a.pdf")).toContainText("Batch password");
    await expect(row(page, "a.pdf")).not.toContainText("Own password");
    await expect(page.getByTestId("password-usage")).toHaveText("Used for 2 files. 1 file has its own password.");

    await page.locator(".workspace-primary-action").click();
    await expect(results(page)).toHaveCount(3);
    expect(await passwordRequests(page, "protect_files")).toEqual([{
        paths: ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"],
        password: "batch-secret",
        passwordOverrides: { "/local/b.pdf": "own-secret" },
    }]);

    await row(page, "b.pdf").locator(".file-selection-item").click();
    await page.getByRole("button", { name: "Use batch password for b.pdf", exact: true }).click();
    await expect(page.getByRole("status").filter({ hasText: "b.pdf uses the batch password again." })).toBeVisible();
    await expect(row(page, "b.pdf")).not.toContainText("Own password");
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await expect(row(page, "b.pdf")).toContainText("Own password");

    await page.getByRole("button", { name: "Use batch password for b.pdf", exact: true }).click();
    await expect(page.getByTestId("password-usage")).toHaveText("Used for all 3 files.");
    await page.locator(".workspace-primary-action").click();
    await expect(results(page)).toHaveCount(3);
    expect((await passwordRequests(page, "protect_files")).at(-1)).toEqual({
        paths: ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"],
        password: "batch-secret",
        passwordOverrides: {},
    });
});

test("Lock saves a file's own password only once it is confirmed, and Run waits until every file has a password", async ({ page }) => {
    await openLock(page);
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await expect(page.getByTestId("password-readiness")).toHaveText("2 files have no password yet");

    await row(page, "a.pdf").locator(".file-selection-item").click();
    await page.getByRole("button", { name: "Set own password for a.pdf", exact: true }).click();
    const editor = page.getByRole("group", { name: "Own password for a.pdf" });
    const save = editor.getByRole("button", { name: "Save password" });
    await expect(save).toBeDisabled();
    await editor.getByLabel("Own new password", { exact: true }).fill("own-a");
    await expect(save).toBeDisabled();
    await editor.getByLabel("Confirm", { exact: true }).fill("own-b");
    await expect(editor).toContainText("Passwords don't match. Type the new password again.");
    await expect(save).toBeDisabled();
    await editor.getByLabel("Confirm", { exact: true }).press("Enter");
    await expect(editor).toBeVisible();
    await editor.getByRole("button", { name: "Cancel" }).click();
    await expect(editor).toHaveCount(0);
    await expect(row(page, "a.pdf")).not.toContainText("Own password");

    await setOwnPassword(page, "a.pdf", "own-a", "lock");
    await expect(row(page, "a.pdf")).toContainText("Own password");
    await expect(page.getByTestId("password-readiness")).toHaveText("1 file has no password yet");
    await expect(run(page)).toBeDisabled();
    await setOwnPassword(page, "b.pdf", "own-b", "lock");
    await expect(page.getByTestId("password-readiness")).toHaveCount(0);
    await expect(run(page)).toBeEnabled();
    await run(page).click();
    await expect(results(page)).toHaveCount(2);
    expect(await passwordRequests(page, "protect_files")).toEqual([{
        paths: ["/local/a.pdf", "/local/b.pdf"],
        password: "",
        passwordOverrides: { "/local/a.pdf": "own-a", "/local/b.pdf": "own-b" },
    }]);
});

test("Lock runs only once both new passwords match and sends one protect_files request", async ({ page }) => {
    await openLock(page);
    await drop(page, ["/local/lease.pdf", "/local/plan.pdf"]);
    await expect(run(page)).toHaveAccessibleName("Lock 2 files");
    await expect(run(page)).toBeDisabled();
    await expect(page.getByTestId("run-destination-example")).toHaveText("lease-protected.pdf");

    await page.getByLabel("New password for all files", { exact: true }).fill("batch-secret");
    await expect(run(page)).toBeDisabled();
    await page.getByLabel("Confirm new password", { exact: true }).fill("batch-secre");
    await expect(page.getByRole("alert")).toHaveText("Passwords don't match. Type the new password again.");
    await expect(page.getByRole("alert").locator("svg")).toHaveCount(1);
    await expect(page.getByTestId("password-readiness")).toHaveText("Passwords don't match");
    await expect(run(page)).toBeDisabled();

    await page.getByLabel("Confirm new password", { exact: true }).fill("batch-secret");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(run(page)).toBeEnabled();
    await run(page).click();
    await expect(results(page)).toHaveCount(2);
    await expect(page.locator(".result-summary strong")).toHaveText("Done. 2 locked");

    const invocations = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations);
    expect(invocations.filter(({ command }) => command === "protect_files")).toEqual([{
        command: "protect_files",
        args: {
            runId: expect.any(String),
            request: { paths: ["/local/lease.pdf", "/local/plan.pdf"], password: "batch-secret", passwordOverrides: {}, outputLocation: "alongsideInput" },
        },
    }]);
    expect(invocations.some(({ command }) => command === "remove_password" || command === "protect_pdf")).toBe(false);
});

test("every file with its own password can run without a batch password", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf"]);
    await expect(page.getByTestId("password-readiness")).toHaveText("1 file has no password yet");
    await expect(page.locator(".workspace-primary-action")).toBeDisabled();
    await setOwnPassword(page, "a.pdf", "own-secret", "unlock");
    await expect(page.locator(".workspace-primary-action")).toBeEnabled();
    await page.locator(".workspace-primary-action").click();
    await expect(results(page)).toHaveCount(1);
    expect(await passwordRequests(page, "remove_password")).toEqual([{ paths: ["/local/a.pdf"], password: "", passwordOverrides: { "/local/a.pdf": "own-secret" } }]);
});

test("a wrong password row retries only that file with the password typed for it", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"]);
    await page.evaluate(() => { (window as unknown as IntakeWindow).__wrongPasswordPaths = ["/local/b.pdf"]; });
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await setOwnPassword(page, "c.pdf", "own-secret", "unlock");
    await page.locator(".workspace-primary-action").click();

    const failedRow = results(page).filter({ hasText: "b.pdf" });
    await expect(failedRow).toHaveAttribute("data-state", "failed");
    await expect(failedRow).toContainText("The password didn't open this file. Check it, or try the password this file was sent with.");
    await expect(failedRow).toContainText("Failed, wrong password");
    await expect(failedRow).not.toContainText("batch-secret");
    await expect(failedRow.getByRole("button", { name: "Retry", exact: true })).toHaveCount(0);

    const tryAnother = failedRow.getByRole("button", { name: "Try another password" });
    await expect(tryAnother).toHaveAttribute("aria-expanded", "false");
    await tryAnother.click();
    await expect(tryAnother).toHaveAttribute("aria-expanded", "true");
    const retry = page.getByRole("group", { name: "Retry b.pdf with another password" });
    await expect(retry).toContainText("Only this file runs again. Its new password isn't saved.");
    await retry.getByLabel("Password for b.pdf", { exact: true }).fill("retry-secret");
    await page.evaluate(() => { (window as unknown as IntakeWindow).__wrongPasswordPaths = []; });
    await retry.getByRole("button", { name: "Retry this file" }).click();

    await expect(failedRow).toHaveAttribute("data-state", "succeeded");
    await expect(results(page)).toHaveCount(3);
    await expect(results(page).filter({ hasText: "a.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "c.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(retry).toHaveCount(0);
    await expect(page.getByRole("button", { name: /^Retry \d+ failed/ })).toHaveCount(0);
    expect(await passwordRequests(page, "remove_password")).toEqual([
        { paths: ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"], password: "batch-secret", passwordOverrides: { "/local/c.pdf": "own-secret" } },
        { paths: ["/local/b.pdf"], password: "", passwordOverrides: { "/local/b.pdf": "retry-secret" } },
    ]);
    await expect(row(page, "b.pdf")).not.toContainText("Own password");
});

test("Retry failed files runs only the wrong-password and processing failures with the current passwords and keeps every finished row", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf", "/local/d.pdf"]);
    await page.evaluate(() => {
        const win = window as unknown as IntakeWindow;
        win.__wrongPasswordPaths = ["/local/b.pdf"];
        win.__failures = { "/local/c.pdf": { kind: "processing", message: "The helper stopped." } };
        win.__invalidInputPaths = ["/local/d.pdf"];
    });
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await run(page).click();
    await expect(results(page)).toHaveCount(4);

    const retryFailed = page.getByRole("button", { name: "Retry 2 failed files", exact: true });
    await expect(retryFailed).toBeEnabled();
    await setOwnPassword(page, "b.pdf", "own-secret", "unlock");
    await expect(results(page)).toHaveCount(4);
    await page.evaluate(() => {
        const win = window as unknown as IntakeWindow;
        win.__wrongPasswordPaths = [];
        win.__failures = {};
    });
    await retryFailed.click();

    await expect(results(page).filter({ hasText: "b.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "c.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "a.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "d.pdf" })).toHaveAttribute("data-state", "failed");
    await expect(page.getByRole("button", { name: /^Retry \d+ failed/ })).toHaveCount(0);
    expect(await passwordRequests(page, "remove_password")).toEqual([
        { paths: ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf", "/local/d.pdf"], password: "batch-secret", passwordOverrides: {} },
        { paths: ["/local/b.pdf", "/local/c.pdf"], password: "batch-secret", passwordOverrides: { "/local/b.pdf": "own-secret" } },
    ]);
});

test("Retry failed files waits until every failed file has a password", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await page.evaluate(() => { (window as unknown as IntakeWindow).__wrongPasswordPaths = ["/local/b.pdf"]; });
    await setOwnPassword(page, "a.pdf", "own-a", "unlock");
    await setOwnPassword(page, "b.pdf", "own-b", "unlock");
    await run(page).click();
    await expect(results(page)).toHaveCount(2);

    await row(page, "b.pdf").locator(".file-selection-item").click();
    await page.getByRole("button", { name: "Use batch password for b.pdf", exact: true }).click();
    await expect(page.getByRole("button", { name: "Retry 1 failed file", exact: true })).toBeDisabled();
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await expect(page.getByRole("button", { name: "Retry 1 failed file", exact: true })).toBeEnabled();
});

test("passwords clear when a file closes, when the list empties and when the view is left", async ({ page }) => {
    await openLock(page);
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await fillBatchLock(page, "batch-secret");
    await setOwnPassword(page, "b.pdf", "own-secret", "lock");

    await page.getByRole("button", { name: "Remove b.pdf from list", exact: true }).click();
    await drop(page, ["/local/b.pdf"]);
    await expect(row(page, "b.pdf")).toContainText("Batch password");
    await expect(row(page, "b.pdf")).not.toContainText("Own password");
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("batch-secret");

    await page.getByRole("button", { name: "Clear list", exact: true }).click();
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("");
    await expect(page.getByLabel("Confirm new password", { exact: true })).toHaveValue("");

    await drop(page, ["/local/a.pdf"]);
    await fillBatchLock(page, "batch-secret");
    await setOwnPassword(page, "a.pdf", "own-secret", "lock");
    await page.getByRole("button", { name: "← All tools" }).click();
    await openLock(page);
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("");
    await drop(page, ["/local/a.pdf"]);
    await expect(row(page, "a.pdf")).toContainText("Batch password");
    await expect(row(page, "a.pdf")).not.toContainText("Own password");
});

test("passwords never reach storage, the network, the console or any other command", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await page.evaluate(() => { (window as unknown as IntakeWindow).__wrongPasswordPaths = ["/local/a.pdf"]; });
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await setOwnPassword(page, "b.pdf", "own-secret", "unlock");
    await page.locator(".workspace-primary-action").click();
    await results(page).filter({ hasText: "a.pdf" }).getByRole("button", { name: "Try another password" }).click();
    await page.getByLabel("Password for a.pdf", { exact: true }).fill("retry-secret");
    await page.getByRole("button", { name: "Retry this file" }).click();
    await expect(results(page)).toHaveCount(2);
    await page.getByRole("button", { name: "← All tools" }).click();

    const seen = await page.evaluate(() => {
        const win = window as unknown as IntakeWindow;
        const storage = (store: Storage) => Array.from({ length: store.length }, (_, index) => `${store.key(index)}=${store.getItem(store.key(index) ?? "")}`);
        return {
            stored: [...storage(localStorage), ...storage(sessionStorage), document.cookie].join("\n"),
            sent: win.__sent.join("\n"),
            otherCommands: JSON.stringify(win.__invocations.filter(({ command }) => command !== "remove_password")),
            page: document.body.innerText,
        };
    });
    for (const secret of ["batch-secret", "own-secret", "retry-secret"]) {
        for (const [where, text] of Object.entries(seen)) expect(text, `${secret} found in ${where}`).not.toContain(secret);
    }
});

test("every File Security search term finds one Lock & Unlock entry that opens the view", async ({ page }) => {
    await page.goto("/");
    const search = page.getByRole("textbox", { name: "Search tools" });
    for (const term of ["lock", "unlock", "protect", "password", "encrypt", "decrypt"]) {
        await search.fill(term);
        await expect(page.getByRole("button", { name: "Open File Security", exact: true }), term).toHaveCount(1);
        await expect(page.locator('button[data-command-result="lock-unlock"]'), term).toHaveCount(1);
    }
    await page.getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    await expect(page.getByRole("radiogroup", { name: "Mode" }).getByRole("radio", { name: "Unlock", exact: true })).toHaveAttribute("aria-checked", "true");
});

test("a saved Remove Password favorite and recent open File Security once migrated and leave no retired id behind", async ({ page }) => {
    await page.addInitScript(() => {
        if (localStorage.getItem("toolbox-tool-id-migration")) return;
        localStorage.setItem("toolbox-favorites", JSON.stringify(["pdf-unlock", "pdf-merge"]));
        localStorage.setItem("toolbox-recent", JSON.stringify(["pdf-unlock"]));
    });
    await page.goto("/");
    const chips = page.getByRole("group", { name: "Favorite tools" });
    await expect(chips.getByRole("button")).toHaveText(["Lock & Unlock", "Merge PDF"]);
    await chips.getByRole("button", { name: "Lock & Unlock" }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();

    await page.reload();
    const saved = await page.evaluate(() => ({
        favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null"),
        recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null"),
    }));
    expect(saved).toEqual({ favorites: ["lock-unlock", "pdf-merge"], recent: ["lock-unlock"] });
});

test("a saved Protect PDF favorite and recent open File Security after the earlier migration already ran", async ({ page }) => {
    await page.addInitScript(() => {
        if (localStorage.getItem("toolbox-tool-id-migration") === "3") return;
        localStorage.setItem("toolbox-tool-id-migration", "1");
        localStorage.setItem("toolbox-favorites", JSON.stringify(["pdf-protect", "pdf-merge"]));
        localStorage.setItem("toolbox-recent", JSON.stringify(["pdf-protect"]));
    });
    await page.goto("/");
    const chips = page.getByRole("group", { name: "Favorite tools" });
    await expect(chips.getByRole("button")).toHaveText(["Lock & Unlock", "Merge PDF"]);
    await chips.getByRole("button", { name: "Lock & Unlock" }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();

    await page.reload();
    const saved = await page.evaluate(() => ({
        favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null"),
        recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null"),
        marker: localStorage.getItem("toolbox-tool-id-migration"),
    }));
    expect(saved).toEqual({ favorites: ["lock-unlock", "pdf-merge"], recent: ["lock-unlock"], marker: "3" });
});

test("a saved Protect Office Files favorite and recent open File Security after migration", async ({ page }) => {
    await page.addInitScript(() => {
        if (localStorage.getItem("toolbox-tool-id-migration") === "3") return;
        localStorage.setItem("toolbox-tool-id-migration", "2");
        localStorage.setItem("toolbox-favorites", JSON.stringify(["pdf-merge", "office-protect"]));
        localStorage.setItem("toolbox-recent", JSON.stringify(["office-protect", "pdf-merge"]));
    });
    await page.goto("/");
    const chips = page.getByRole("group", { name: "Favorite tools" });
    await expect(chips.getByRole("button")).toHaveText(["Merge PDF", "Lock & Unlock"]);
    await chips.getByRole("button", { name: "Lock & Unlock" }).click();
    await expect(page.locator(".workspace-title")).toHaveText("File Security");
    await expect(page.getByRole("heading", { name: "Lock & Unlock", exact: true })).toBeVisible();
    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();

    await page.reload();
    const saved = await page.evaluate(() => ({
        favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null"),
        recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null"),
        marker: localStorage.getItem("toolbox-tool-id-migration"),
    }));
    expect(saved).toEqual({ favorites: ["pdf-merge", "lock-unlock"], recent: ["lock-unlock", "pdf-merge"], marker: "3" });
});

test("Lock sends a mixed PDF, Word, Excel and PowerPoint batch in one protect_files request and shows one locked row per file", async ({ page }) => {
    await openLock(page);
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();
    await expect(page.getByText("Each file gets a copy ending in -protected that opens only with the new password. Your originals stay as they are.")).toBeVisible();
    await drop(page, ["/local/lease.pdf", "/local/offer.docx", "/local/budget.xlsx"]);
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/deck.pptx"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    const dialogs = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations
        .filter(({ command }) => command === "plugin:dialog|open")
        .map(({ args }) => (args as { options: { filters: Array<{ extensions: string[] }> } }).options.filters[0].extensions));
    expect(dialogs).toEqual([["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"]]);
    await expect(rows(page)).toHaveText([/^lease\.pdf/, /^offer\.docx/, /^budget\.xlsx/, /^deck\.pptx/]);
    await expect(rows(page).filter({ hasText: "Won't run" })).toHaveCount(0);
    await fillBatchLock(page, "batch-secret");
    await expect(run(page)).toHaveAccessibleName("Lock 4 files");
    await run(page).click();

    await expect(results(page)).toHaveCount(4);
    await expect(page.locator(".result-summary strong")).toHaveText("Done. 4 locked");
    for (const name of ["lease.pdf", "offer.docx", "budget.xlsx", "deck.pptx"]) await expect(results(page).filter({ hasText: name })).toHaveAttribute("data-state", "succeeded");
    expect(await requestFor(page, "protect_files")).toEqual([{
        paths: ["/local/lease.pdf", "/local/offer.docx", "/local/budget.xlsx", "/local/deck.pptx"],
        password: "batch-secret",
        passwordOverrides: {},
        outputLocation: "alongsideInput",
    }]);
    expect(await requestFor(page, "remove_password")).toEqual([]);
});

test("Lock flags legacy Office files as failing, still sends them, and shows each as its own Failed row", async ({ page }) => {
    const legacy = {
        "/local/minutes.doc": "Legacy .doc files can't be locked. Save it as .docx in Word, then add it again.",
        "/local/ledger.xls": "Legacy .xls files can't be locked. Save it as .xlsx in Excel, then add it again.",
        "/local/old-deck.ppt": "Legacy .ppt files can't be locked. Save it as .pptx in PowerPoint, then add it again.",
    };
    await openLock(page);
    await page.evaluate((legacy) => {
        (window as unknown as IntakeWindow).__failures = Object.fromEntries(Object.entries(legacy).map(([path, message]) => [path, { kind: "invalidInput", message }]));
    }, legacy);
    await drop(page, ["/local/minutes.doc", "/local/deck.pptx", "/local/ledger.xls", "/local/old-deck.ppt"]);
    for (const [path, message] of Object.entries(legacy)) {
        const flagged = row(page, path.split("/").pop() ?? path);
        await expect(flagged).toContainText("Won't run");
        await expect(flagged).toContainText(message);
        await expect(flagged.getByRole("button", { name: /password/i })).toHaveCount(0);
    }
    await expect(row(page, "deck.pptx")).not.toContainText("Won't run");
    await expect(page.locator(".file-selection-selected")).toHaveText("3 won't run in Lock");
    await expect(page.getByText("3 files can't be locked. Lock needs .docx, .xlsx or .pptx. They'd fail if you ran now.", { exact: true })).toBeVisible();
    await expect(page.getByTestId("password-readiness")).toHaveText("1 file has no password yet");
    await expect(page.getByTestId("failing-files")).toHaveCount(0);
    await fillBatchLock(page, "batch-secret");
    await expect(page.getByTestId("failing-files")).toHaveText("3 flagged files will fail");
    await expect(run(page)).toHaveAccessibleName("Lock 4 files");
    await run(page).click();

    expect((await requestFor(page, "protect_files")).map((request) => request.paths)).toEqual([["/local/minutes.doc", "/local/deck.pptx", "/local/ledger.xls", "/local/old-deck.ppt"]]);
    await expect(results(page)).toHaveCount(4);
    await expect(results(page).filter({ hasText: "deck.pptx" })).toHaveAttribute("data-state", "succeeded");
    for (const [path, message] of Object.entries(legacy)) {
        const failed = results(page).filter({ hasText: path.split("/").pop() ?? path });
        await expect(failed).toHaveAttribute("data-state", "failed");
        await expect(failed).toHaveAttribute("data-tone", "fail");
        await expect(failed.locator(".result-row-state")).toContainText("Failed");
        await expect(failed).toContainText(message);
        await expect(failed.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
    }
    await expect(page.getByRole("button", { name: /^Retry \d+ failed/ })).toHaveCount(0);
});

const fileDialogFilters = (page: Page) => page.evaluate(() => (window as unknown as IntakeWindow).__invocations
    .filter(({ command, args }) => command === "plugin:dialog|open" && !(args as { options: { directory?: boolean } }).options.directory)
    .map(({ args }) => (args as { options: { filters: Array<{ extensions: string[] }> } }).options.filters[0].extensions));

test("the flagged files follow the mode, both pickers take legacy Office files, and flagged files count toward Run", async ({ page }) => {
    const notADocument = "Not a document. Lock and Unlock take PDF, Word, Excel and PowerPoint files.";
    const banner = page.getByText("2 files can't be locked. Lock needs .docx, .xlsx or .pptx. They'd fail if you ran now.", { exact: true });
    const header = page.locator(".file-selection-selected");
    await open(page, "Lock & Unlock");
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/minutes.doc", "/local/deck.pptx"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await drop(page, ["/local/site-photo.jpg", "/local/ledger.xls", "/local/lease.pdf"]);
    const notAdded = page.getByRole("region", { name: "Not added" });
    await expect(notAdded.getByRole("listitem")).toHaveText([`site-photo.jpg${notADocument}`]);
    await expect(rows(page)).toHaveText([/^minutes\.doc/, /^deck\.pptx/, /^ledger\.xls/, /^lease\.pdf/]);
    await expect(rows(page).filter({ hasText: "Won't run" })).toHaveCount(0);
    await expect(header).toHaveText("All 4 can be unlocked");
    await expect(banner).toHaveCount(0);

    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(row(page, "minutes.doc")).toContainText("Won't run");
    await expect(row(page, "minutes.doc")).toContainText("Legacy .doc files can't be locked. Save it as .docx in Word, then add it again.");
    await expect(row(page, "ledger.xls")).toContainText("Won't run");
    await expect(row(page, "ledger.xls")).toContainText("Legacy .xls files can't be locked. Save it as .xlsx in Excel, then add it again.");
    for (const name of ["deck.pptx", "lease.pdf"]) await expect(row(page, name)).not.toContainText("Won't run");
    await expect(header).toHaveText("2 won't run in Lock");
    await expect(banner).toBeVisible();
    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = ["/local/scan.png"]; });
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(notAdded.getByRole("listitem")).toHaveText([`scan.png${notADocument}`]);
    expect(await fileDialogFilters(page)).toEqual([
        ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"],
        ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"],
    ]);

    await fillBatchLock(page, "batch-secret");
    await expect(run(page)).toBeEnabled();
    await expect(run(page)).toHaveAccessibleName("Lock 4 files");
    await expect(page.getByTestId("failing-files")).toHaveText("2 flagged files will fail");

    await modes(page).getByRole("radio", { name: "Unlock", exact: true }).click();
    await expect(rows(page).filter({ hasText: "Won't run" })).toHaveCount(0);
    await expect(banner).toHaveCount(0);
    await expect(run(page)).toHaveAccessibleName("Unlock 4 files");

    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    expect((await fileDialogFilters(page)).at(-1)).toEqual(["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"]);
    await page.getByRole("button", { name: "Remove 2 flagged files" }).click();
    await expect(rows(page)).toHaveText([/^deck\.pptx/, /^lease\.pdf/]);
    await expect(banner).toHaveCount(0);
    await expect(header).toHaveText("All 2 can be locked");
    await fillBatchLock(page, "batch-secret");
    await run(page).click();

    await expect(results(page)).toHaveCount(2);
    expect((await requestFor(page, "protect_files")).map((request) => request.paths)).toEqual([["/local/deck.pptx", "/local/lease.pdf"]]);
    expect(await requestFor(page, "remove_password")).toEqual([]);
});

test("Lock & Unlock shows the app-wide destination and sends a per-run folder in both modes without writing Settings", async ({ page }) => {
    await page.addInitScript(() => {
        (window as unknown as IntakeWindow).__outputSettings = { mode: "customFolder", folder: "/Users/test/Documents/Toolbox" };
    });
    await openLock(page);
    const destination = page.getByRole("region", { name: "Destination" });
    await expect(destination.getByTestId("run-destination-path")).toHaveText("~/Documents/Toolbox");
    await expect(destination.getByText("This run only")).toHaveCount(0);
    await drop(page, ["/local/lease.pdf", "/local/budget.xlsx"]);

    await page.evaluate(() => { (window as unknown as IntakeWindow).__dialogResult = "/Users/test/Delivery/Acme"; });
    await destination.getByRole("button", { name: "Change for this run" }).click();
    await expect(destination.getByTestId("run-destination-path")).toHaveText("~/Delivery/Acme");
    await expect(destination.getByText("This run only")).toBeVisible();
    const folderDialog = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations
        .filter(({ command }) => command === "plugin:dialog|open").pop()?.args as { options: { directory: boolean; multiple: boolean } });
    expect(folderDialog.options).toMatchObject({ directory: true, multiple: false });

    await fillBatchLock(page, "batch-secret");
    await run(page).click();
    await expect(results(page)).toHaveCount(2);

    await modes(page).getByRole("radio", { name: "Unlock", exact: true }).click();
    await expect(destination.getByTestId("run-destination-path")).toHaveText("~/Delivery/Acme");
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await run(page).click();
    await expect(results(page)).toHaveCount(2);

    const perRun = { customFolder: "/Users/test/Delivery/Acme" };
    expect((await requestFor(page, "protect_files")).map((request) => (request as { outputLocation?: unknown }).outputLocation)).toEqual([perRun]);
    expect((await requestFor(page, "remove_password")).map((request) => (request as { outputLocation?: unknown }).outputLocation)).toEqual([perRun]);
    const invocations = await page.evaluate(() => (window as unknown as IntakeWindow).__invocations.map(({ command }) => command));
    expect(invocations).not.toContain("set_output_settings");
    expect(await page.evaluate(() => (window as unknown as IntakeWindow).__outputSettings)).toEqual({ mode: "customFolder", folder: "/Users/test/Documents/Toolbox" });

    await destination.getByRole("button", { name: "Reset to default" }).click();
    await expect(destination.getByTestId("run-destination-path")).toHaveText("~/Documents/Toolbox");
});

test("Lock without its PDF helper shows the PDFs as Unavailable and the Word and Excel files as locked", async ({ page }) => {
    const missing = { kind: "unavailable", message: "This PDF tool is unavailable because a helper it needs is missing from this copy of Toolbox. Reinstall Toolbox to restore it." };
    await openLock(page);
    await drop(page, ["/local/lease.pdf", "/local/offer.docx", "/local/scan.pdf", "/local/budget.xlsx"]);
    await page.evaluate((missing) => {
        (window as unknown as IntakeWindow).__failures = { "/local/lease.pdf": missing, "/local/scan.pdf": missing };
    }, missing);
    await fillBatchLock(page, "batch-secret");
    await run(page).click();

    await expect(results(page)).toHaveCount(4);
    expect((await requestFor(page, "protect_files")).map((request) => request.paths)).toEqual([["/local/lease.pdf", "/local/offer.docx", "/local/scan.pdf", "/local/budget.xlsx"]]);
    for (const name of ["offer.docx", "budget.xlsx"]) await expect(results(page).filter({ hasText: name })).toHaveAttribute("data-state", "succeeded");
    for (const name of ["lease.pdf", "scan.pdf"]) {
        const failed = results(page).filter({ hasText: name });
        await expect(failed).toHaveAttribute("data-state", "failed");
        await expect(failed).toHaveAttribute("data-tone", "skip");
        await expect(failed.locator(".result-row-state")).toContainText("Unavailable");
        await expect(failed).toContainText(missing.message);
    }
});

test("Lock shows already-protected files as Skipped rows with their reason, apart from Unavailable failures in the summary", async ({ page }) => {
    const already = { code: "alreadyProtected", message: "Already password-protected. Unlock it first, then lock it again." };
    const missing = { kind: "unavailable", message: "This PDF tool is unavailable because a helper it needs is missing from this copy of Toolbox. Reinstall Toolbox to restore it." };
    await openLock(page);
    await drop(page, ["/local/lease.pdf", "/local/invoice.pdf", "/local/offer.docx", "/local/payroll.docx", "/local/statement.pdf", "/local/deck.pptx"]);
    await page.evaluate(({ already, missing }) => {
        const win = window as unknown as IntakeWindow;
        win.__skippedPaths = { "/local/invoice.pdf": already, "/local/payroll.docx": already };
        win.__failures = { "/local/statement.pdf": missing };
    }, { already, missing });
    await fillBatchLock(page, "batch-secret");
    await run(page).click();

    await expect(results(page)).toHaveCount(6);
    await expect(page.locator(".result-summary strong")).toHaveText("Done. 3 locked, 2 skipped, 1 failed");
    for (const name of ["lease.pdf", "offer.docx", "deck.pptx"]) await expect(results(page).filter({ hasText: name })).toHaveAttribute("data-state", "succeeded");
    for (const name of ["invoice.pdf", "payroll.docx"]) {
        const skipped = results(page).filter({ hasText: name });
        await expect(skipped).toHaveAttribute("data-state", "skipped");
        await expect(skipped).toHaveAttribute("data-tone", "skip");
        await expect(skipped).toContainText(already.message);
        await expect(skipped.locator(".result-row-state")).toContainText("Skipped");
        await expect(skipped.getByRole("button")).toHaveCount(0);
    }
    const unavailable = results(page).filter({ hasText: "statement.pdf" });
    await expect(unavailable).toHaveAttribute("data-state", "failed");
    await expect(unavailable).toHaveAttribute("data-tone", "skip");
    await expect(unavailable.locator(".result-row-state")).toContainText("Unavailable");
    await expect(unavailable).toContainText(missing.message);
    await expect(unavailable.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
});

test("Unlock sends the ordered batch with one password and the shell's output folder, and shows one row per file", async ({ page }) => {
    await page.addInitScript(() => {
        (window as unknown as IntakeWindow).__outputSettings = { mode: "customFolder", folder: "/Users/test/Unlocked" };
    });
    await open(page, "Lock & Unlock");
    await expect(page.locator(".workspace-primary-action")).toHaveText(/^Unlock files/);
    await drop(page, ["/local/lease.pdf", "/local/budget.xlsx", "/local/site-photo.jpg", "/local/deck.pptx", "/local/renamed.docx", "/local/notes.txt", "/local/minutes.doc"]);
    await expect(rows(page)).toHaveText([/lease\.pdf/, /budget\.xlsx/, /deck\.pptx/, /renamed\.docx/, /minutes\.doc/]);
    await page.evaluate(() => {
        const win = window as unknown as IntakeWindow;
        win.__wrongPasswordPaths = ["/local/budget.xlsx"];
        win.__invalidInputPaths = ["/local/renamed.docx"];
    });
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    const run = page.locator(".workspace-primary-action");
    await expect(run).toBeEnabled();
    await expect(run).toHaveText(/^Unlock 5 files/);
    await run.click();

    await expect(results(page)).toHaveCount(5);
    expect(await requestFor(page, "remove_password")).toEqual([{
        paths: ["/local/lease.pdf", "/local/budget.xlsx", "/local/deck.pptx", "/local/renamed.docx", "/local/minutes.doc"],
        password: "batch-secret",
        passwordOverrides: {},
        outputLocation: { customFolder: "/Users/test/Unlocked" },
    }]);
    await expect(results(page).filter({ hasText: "lease.pdf" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "deck.pptx" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "minutes.doc" })).toHaveAttribute("data-state", "succeeded");
    await expect(results(page).filter({ hasText: "budget.xlsx" })).toContainText("Failed, wrong password");
    const unsupported = results(page).filter({ hasText: "renamed.docx" });
    await expect(unsupported).toHaveAttribute("data-state", "failed");
    await expect(unsupported).toContainText("Failed, the input can't be used");
    await expect(run).toBeEnabled();
});

for (const mode of ["Lock", "Unlock"] as const) {
    test(`${mode} shows the signature removed warning only on the signed PDF's row`, async ({ page }) => {
        const removed = "Saved. The digital signature was removed, because changing the file breaks it.";
        const done = mode === "Lock" ? "locked" : "unlocked";
        await open(page, "Lock & Unlock");
        await modes(page).getByRole("radio", { name: mode, exact: true }).click();
        await drop(page, ["/local/lease.pdf", "/local/vendor-nda.pdf", "/local/deck.pptx"]);
        await page.evaluate(() => {
            (window as unknown as IntakeWindow).__warnings = { "/local/vendor-nda.pdf": [{ code: "signatureRemoved", message: "native wording" }] };
        });
        if (mode === "Lock") await fillBatchLock(page, "batch-secret");
        else await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
        await run(page).click();

        await expect(results(page)).toHaveCount(3);
        const signed = results(page).filter({ hasText: "vendor-nda.pdf" });
        await expect(signed).toHaveAttribute("data-state", "succeeded");
        await expect(signed).toHaveAttribute("data-tone", "warn");
        await expect(signed).toContainText(removed);
        await expect(signed).not.toContainText("native wording");
        await expect(signed.locator(".result-row-state .sr-only")).toHaveText(`${mode}ed with a warning`);
        await expect(signed.getByRole("button", { name: "Open vendor-nda.pdf.out", exact: true })).toBeVisible();
        for (const name of ["lease.pdf", "deck.pptx"]) {
            const unsigned = results(page).filter({ hasText: name });
            await expect(unsigned).toHaveAttribute("data-tone", "ok");
            await expect(unsigned).not.toContainText(removed);
            await expect(unsigned.locator(".result-row-state .sr-only")).toHaveText(`${mode}ed`);
        }
        await expect(page.locator(".result-summary")).toContainText(`Done. 3 ${done}`);
        await expect(page.locator(".result-summary")).toContainText(`1 ${done} file has a warning.`);
    });
}

test("switching between Lock and Unlock clears every password and Lock takes Word and Excel files in place", async ({ page }) => {
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/a.pdf", "/local/b.pdf"]);
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await setOwnPassword(page, "b.pdf", "own-secret", "unlock");

    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("");
    await expect(page.getByLabel("Confirm new password", { exact: true })).toHaveValue("");
    await expect(row(page, "b.pdf")).not.toContainText("Own password");
    await expect(run(page)).toBeDisabled();
    await expect(page.getByText("Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Open Protect Office Files" })).toHaveCount(0);
    await fillBatchLock(page, "lock-secret");
    await setOwnPassword(page, "a.pdf", "own-lock-secret", "lock");

    await modes(page).getByRole("radio", { name: "Unlock", exact: true }).click();
    await expect(page.getByLabel("Current password for all files", { exact: true })).toHaveValue("");
    await expect(row(page, "a.pdf")).not.toContainText("Own password");
    await expect(rows(page)).toHaveCount(2);

    await modes(page).getByRole("radio", { name: "Lock", exact: true }).click();
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("");
    expect(await requestFor(page, "remove_password")).toEqual([]);
    expect(await requestFor(page, "protect_files")).toEqual([]);
});

test("Unlock shows files that need no work as Skipped rows with their reason, apart from failures in the summary", async ({ page }) => {
    const notProtected = "Not password-protected. Nothing to unlock, so no copy was made.";
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/lease.pdf", "/local/offer.docx", "/local/budget.xlsx", "/local/brochure.pdf", "/local/deck.pptx", "/local/payroll.xlsx", "/local/renamed.docx"]);
    await page.evaluate((notProtected) => {
        const win = window as unknown as IntakeWindow;
        win.__skippedPaths = { "/local/offer.docx": { code: "notProtected", message: notProtected } };
        win.__wrongPasswordPaths = ["/local/payroll.xlsx"];
        win.__invalidInputPaths = ["/local/renamed.docx"];
    }, notProtected);
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await page.locator(".workspace-primary-action").click();

    await expect(results(page)).toHaveCount(7);
    await expect(page.locator(".result-summary strong")).toHaveText("Done. 4 unlocked, 1 skipped, 2 failed");
    const skipped = results(page).filter({ hasText: "offer.docx" });
    await expect(skipped).toHaveAttribute("data-state", "skipped");
    await expect(skipped).toHaveAttribute("data-tone", "skip");
    await expect(skipped).toContainText(notProtected);
    await expect(skipped.locator(".result-row-state")).toContainText("Skipped");
    await expect(skipped.getByRole("button")).toHaveCount(0);
    const restricted = results(page).filter({ hasText: "brochure.pdf" });
    await expect(restricted).toHaveAttribute("data-state", "succeeded");
    await expect(restricted.locator(".result-row-state")).toContainText("Unlocked");
    await expect(restricted.getByRole("button", { name: "Open brochure.pdf.out", exact: true })).toBeVisible();
    await expect(restricted).not.toContainText(/PDF Editor|permission/i);
    for (const name of ["payroll.xlsx", "renamed.docx"]) {
        const failed = results(page).filter({ hasText: name });
        await expect(failed).toHaveAttribute("data-state", "failed");
        await expect(failed).toHaveAttribute("data-tone", "fail");
        await expect(failed.locator(".result-row-state")).toContainText("Failed");
    }
    const unlocked = results(page).filter({ hasText: "lease.pdf" });
    await expect(unlocked.getByRole("button", { name: "Open lease.pdf.out", exact: true })).toBeVisible();
    await expect(unlocked.getByRole("button", { name: /^Show in .*: lease\.pdf\.out$/ })).toBeVisible();
});

test("Unlock shows each verification and legacy-format failure with its own message on its own row", async ({ page }) => {
    const failures: Record<string, { kind: string; message: string }> = {
        "/local/budget.xls": { kind: "unavailable", message: "Uses an older encryption type Toolbox can't open. The password may be right." },
        "/local/memo.doc": { kind: "unavailable", message: "Older Office files only take passwords up to 15 characters, and this one is longer. Toolbox can't try it." },
        "/local/photos.ppt": { kind: "unavailable", message: "Legacy .ppt files with pictures can't be unlocked yet. The file isn't damaged." },
        "/local/offer.docx": { kind: "unavailable", message: "Uses an encryption type Toolbox can't open. The password may be right." },
        "/local/scan.pdf": { kind: "processing", message: "Not a readable PDF. The file may be damaged or incomplete. Nothing was saved." },
        "/local/report.pdf": { kind: "processing", message: "The unlocked copy didn't pass verification, so nothing was saved." },
        "/local/payroll.xlsx": { kind: "wrongPassword", message: "The password didn't open this file." },
    };
    await open(page, "Lock & Unlock");
    await drop(page, ["/local/budget.xls", "/local/lease.pdf", "/local/memo.doc", "/local/photos.ppt", "/local/offer.docx", "/local/scan.pdf", "/local/minutes.doc", "/local/report.pdf", "/local/payroll.xlsx"]);
    await page.evaluate((failures) => { (window as unknown as IntakeWindow).__failures = failures; }, failures);
    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await page.locator(".workspace-primary-action").click();

    await expect(results(page)).toHaveCount(9);
    for (const name of ["lease.pdf", "minutes.doc"]) await expect(results(page).filter({ hasText: name })).toHaveAttribute("data-state", "succeeded");
    for (const [path, { kind, message }] of Object.entries(failures)) {
        const name = path.split("/").pop() ?? path;
        const failed = results(page).filter({ hasText: name });
        await expect(failed).toHaveAttribute("data-state", "failed");
        if (kind !== "wrongPassword") await expect(failed).toContainText(message);
        for (const other of Object.values(failures)) {
            if (other.message !== message) await expect(failed).not.toContainText(other.message);
        }
        if (kind === "unavailable") {
            await expect(failed).toHaveAttribute("data-tone", "skip");
            await expect(failed.locator(".result-row-state")).toContainText("Unavailable");
            await expect(failed.getByRole("button", { name: /^Retry/ })).toHaveCount(0);
        }
        if (kind === "processing") {
            await expect(failed).toHaveAttribute("data-tone", "fail");
            await expect(failed.getByRole("button", { name: `Retry ${name}`, exact: true })).toBeVisible();
        }
        if (kind === "wrongPassword") {
            await expect(failed).toHaveAttribute("data-tone", "fail");
            await expect(failed).toContainText("Failed, wrong password");
            await expect(failed).toContainText("The password didn't open this file. Check it, or try the password this file was sent with.");
        }
    }
});
