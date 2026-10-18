import { expect, test, type Page } from "@playwright/test";

// Lock asks for the new password twice, so fill every password field in the options panel.
const fillPasswords = async (page: Page, password: string) => {
    for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await field.fill(password);
};

type PathKind = "file" | "folder" | "unreadable";

type IntakeWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __pathKinds: Record<string, PathKind>;
    __dialogResult: string | string[] | null;
    __processingDelayMs: number;
    __wrongPasswordPaths: string[];
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
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:")) return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                if (win.__processingDelayMs) await new Promise((resolve) => window.setTimeout(resolve, win.__processingDelayMs));
                const { request } = args as { request: { paths: string[] } };
                return request.paths.map((inputPath) => win.__wrongPasswordPaths.includes(inputPath)
                    ? { inputPath, outputPaths: [], detail: "", state: "failed", failure: { kind: "wrongPassword", message: "Wrong password or unsupported encryption.", pointer: null }, reason: null, warnings: [] }
                    : { inputPath, outputPaths: [`${inputPath}.out`], detail: "Test output", state: "succeeded", failure: null, reason: null, warnings: [] });
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

    await fillPasswords(page, "test-password");
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

test("in the file list Cmd+A selects every file, arrows move the selection and Backspace removes it", async ({ page }) => {
    await open(page, "Protect PDF");
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

test("a file's own password is sent as an override and every other file uses the batch password", async ({ page }) => {
    await open(page, "Protect PDF");
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
    expect(await passwordRequests(page, "protect_pdf")).toEqual([{
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
});

test("every file with its own password can run without a batch password", async ({ page }) => {
    await open(page, "Remove Password");
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
    await open(page, "Remove Password");
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
    await expect(page.getByRole("button", { name: /^Retry/ })).toHaveCount(0);

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
    await expect(retry).toHaveCount(0);
    expect(await passwordRequests(page, "remove_password")).toEqual([
        { paths: ["/local/a.pdf", "/local/b.pdf", "/local/c.pdf"], password: "batch-secret", passwordOverrides: { "/local/c.pdf": "own-secret" } },
        { paths: ["/local/b.pdf"], password: "retry-secret", passwordOverrides: {} },
    ]);
    await expect(row(page, "b.pdf")).not.toContainText("Own password");
});

test("passwords clear when a file closes, when the list empties and when the view is left", async ({ page }) => {
    await open(page, "Protect PDF");
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
    await page.getByRole("toolbar", { name: "File security tools" }).getByRole("button", { name: "Remove Password", exact: true }).click();
    await expect(page.getByLabel("Current password for all files", { exact: true })).toHaveValue("");

    await page.getByLabel("Current password for all files", { exact: true }).fill("batch-secret");
    await page.getByRole("button", { name: "← All tools" }).click();
    await page.getByRole("button", { name: "Open Protect PDF", exact: true }).click();
    await expect(page.getByLabel("New password for all files", { exact: true })).toHaveValue("");
});

test("passwords never reach storage, the network, the console or any other command", async ({ page }) => {
    await open(page, "Remove Password");
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
