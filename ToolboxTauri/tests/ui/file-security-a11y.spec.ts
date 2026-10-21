import { expect, test, type Locator, type Page } from "@playwright/test";

type Outcome = { inputPath: string; outputPaths: string[]; detail: string; state: string; failure: unknown; reason: unknown; warnings: unknown[] };

type SecurityWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialog: unknown[];
    __drop: (paths: string[]) => void;
    __emit: (payload: unknown) => void;
    __finish: (outcomes: Outcome[]) => void;
};

type RunArgs = { runId: string; request: { paths: string[]; password: string; passwordOverrides: Record<string, string>; outputLocation: unknown } };

const outcome = (inputPath: string, fields: Partial<Outcome>): Outcome => ({
    inputPath, outputPaths: [], detail: "", state: "succeeded", failure: null, reason: null, warnings: [], ...fields,
});
const failure = (kind: string, message: string, details?: unknown) => ({ kind, message, pointer: null, ...(details ? { details } : {}) });

test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
        const win = window as unknown as SecurityWindow;
        const callbacks = new Map<number, (event: unknown) => void>();
        const listeners = new Map<number, { event: string; handler: number }>();
        let nextId = 1;
        let finish: ((outcomes: unknown) => void) | null = null;
        win.__invocations = [];
        win.__dialog = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        const dispatch = (name: string, payload: unknown) => {
            for (const [id, listener] of listeners) {
                if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
            }
        };
        win.__drop = (paths) => dispatch("toolbox://shell-event", { kind: "dropped-files", paths });
        win.__emit = (payload) => dispatch("batch-progress", payload);
        win.__finish = (outcomes) => finish?.(outcomes);
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
                if (command === "plugin:dialog|open") return win.__dialog.shift() ?? null;
                if (command === "inspect_intake_paths") return (args as { paths: string[] }).paths.map((path) => ({ path, kind: "file" }));
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "cancel_run") return true;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command.startsWith("plugin:") || command === "shell_ready" || command === "open_output_path" || command === "reveal_output_path") return null;
                if (command.startsWith("inspect_")) throw "No preview in this test.";
                return new Promise((resolve) => { finish = resolve; });
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

const invocations = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as SecurityWindow).__invocations.filter((call) => call.command === command).map((call) => call.args), command);
const queueDialog = (page: Page, result: unknown) => page.evaluate((result) => { (window as unknown as SecurityWindow).__dialog.push(result); }, result);
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as SecurityWindow).__emit(payload), payload);
const finish = (page: Page, outcomes: Outcome[]) => page.evaluate((outcomes) => (window as unknown as SecurityWindow).__finish(outcomes), outcomes);

const openSecurity = async (page: Page) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Open Lock & Unlock", exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as unknown as SecurityWindow).__invocations.some(({ command, args }) =>
        command === "plugin:event|listen" && (args as { event: string }).event === "toolbox://shell-event"))).toBe(true);
};

const mode = (page: Page) => page.getByRole("radiogroup", { name: "Mode" });

const startRun = async (page: Page, choice: "Lock" | "Unlock", paths: string[]) => {
    await openSecurity(page);
    await mode(page).getByRole("radio", { name: choice, exact: true }).click();
    await page.evaluate((paths) => (window as unknown as SecurityWindow).__drop(paths), paths);
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(paths.length);
    for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await field.fill("test-password");
    await page.getByRole("button", { name: `${choice} ${paths.length} files` }).click();
    const command = choice === "Lock" ? "protect_files" : "remove_password";
    await expect.poll(async () => (await invocations(page, command)).length).toBe(1);
    const [{ runId }] = await invocations(page, command) as RunArgs[];
    return { runId };
};

// Presses Tab until the target holds focus, so every step is reached the way a keyboard user reaches it.
const tabTo = async (page: Page, target: Locator) => {
    for (let step = 0; step < 80; step += 1) {
        if (await target.evaluate((element) => element === document.activeElement)) return;
        await page.keyboard.press("Tab");
    }
    throw new Error("Tab never reached the target.");
};

for (const { choice, verb } of [{ choice: "Lock", verb: "Locking" }, { choice: "Unlock", verb: "Unlocking" }] as const) {
    test(`${choice} shows progress and streamed rows from per-file events while the options stay locked`, async ({ page }) => {
        const paths = ["/local/report.pdf", "/local/lease.docx", "/local/budget.xlsx"];
        const { runId } = await startRun(page, choice, paths);

        const status = page.getByRole("status").filter({ hasText: new RegExp(`^${verb}`) });
        const rows = page.getByRole("list", { name: "Files and results" }).getByRole("listitem");
        await expect(status.locator("strong")).toHaveText(`${verb} 0 of 3`);
        await expect(page.getByRole("group", { name: "Running batch" })).toContainText("Mode, passwords and files are locked until the batch finishes.");
        for (const radio of await mode(page).getByRole("radio").all()) await expect(radio).toBeDisabled();
        for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await expect(field).toBeDisabled();
        await expect(page.getByRole("button", { name: "Choose files to process" })).toBeDisabled();

        await emit(page, { runId, index: 0, phase: "started" });
        await expect(rows.nth(0)).toHaveText(new RegExp(`^report\\.pdfWriting and checking the copy${verb}$`));
        await expect(rows.nth(1)).toHaveText(/^lease\.docxWaiting$/);

        await emit(page, { runId, index: 0, phase: "finished", outcome: outcome(paths[0], { outputPaths: ["/local/report-out.pdf"] }) });
        await emit(page, { runId, index: 1, phase: "started" });
        await expect(status.locator("strong")).toHaveText(`${verb} 1 of 3`);
        await expect(rows.nth(0)).not.toContainText("Writing and checking the copy");
        await expect(rows.nth(1)).toContainText(`Writing and checking the copy${verb}`);
        expect(await page.locator(".live-run-bar > span").evaluateAll((spans) => spans.map((span) => span.getAttribute("data-tone")))).toEqual(["ok", "wait", "wait"]);

        await finish(page, paths.map((path) => outcome(path, { outputPaths: [`${path}.out`] })));
        await expect(page.getByRole("status").filter({ hasText: /^Done\./ }).locator("strong")).toHaveText(`Done. 3 ${choice.toLowerCase()}ed`);
        await expect(mode(page).getByRole("radio", { name: choice, exact: true })).toBeEnabled();
    });
}

test("the whole Lock flow runs by keyboard only", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("Meta+K");
    await page.keyboard.type("Lock");
    await tabTo(page, page.getByRole("button", { name: "Open Lock & Unlock", exact: true }));
    await page.keyboard.press("Enter");
    await expect(mode(page)).toBeVisible();

    await queueDialog(page, ["/local/report.pdf", "/local/lease.docx"]);
    await page.keyboard.press("Meta+O");
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(2);

    await tabTo(page, mode(page).getByRole("radio", { checked: true }));
    await page.keyboard.press("ArrowLeft");
    await expect(mode(page).getByRole("radio", { name: "Lock", exact: true })).toBeFocused();
    await page.keyboard.press("Space");
    await expect(mode(page).getByRole("radio", { name: "Lock", exact: true })).toBeChecked();

    await tabTo(page, page.getByRole("textbox", { name: "New password for all files", exact: true }));
    await page.keyboard.type("batch-secret");
    await tabTo(page, page.getByRole("textbox", { name: "Confirm new password", exact: true }));
    await page.keyboard.type("batch-secret");

    await tabTo(page, page.locator(".file-selection-item").filter({ hasText: "report.pdf" }));
    await page.keyboard.press("Space");
    await tabTo(page, page.getByRole("button", { name: "Set own password for report.pdf" }));
    await page.keyboard.press("Enter");
    const editor = page.getByRole("group", { name: "Own password for report.pdf" });
    await expect(editor.getByRole("textbox", { name: "Own new password", exact: true })).toBeFocused();
    await page.keyboard.type("own-secret");
    await page.keyboard.press("Tab");
    await expect(editor.getByRole("textbox", { name: "Confirm", exact: true })).toBeFocused();
    await page.keyboard.type("own-secret");
    await page.keyboard.press("Enter");
    await expect(editor).toHaveCount(0);
    await expect(page.getByTestId("password-usage")).toHaveText("Used for 1 file. 1 file has its own password.");

    await queueDialog(page, "/local/out");
    await tabTo(page, page.getByRole("button", { name: "Change for this run" }));
    await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: "Reset to default" })).toBeVisible();

    await queueDialog(page, ["/local/budget.xlsx"]);
    await tabTo(page, page.getByRole("button", { name: "Choose files to process" }));
    await page.keyboard.press("Enter");
    await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(3);

    await page.keyboard.press("Meta+Enter");
    await expect.poll(async () => (await invocations(page, "protect_files")).length).toBe(1);
    const [{ request }] = await invocations(page, "protect_files") as RunArgs[];
    expect(request).toMatchObject({
        paths: ["/local/report.pdf", "/local/lease.docx", "/local/budget.xlsx"],
        password: "batch-secret",
        passwordOverrides: { "/local/report.pdf": "own-secret" },
        outputLocation: { customFolder: "/local/out" },
    });
});

for (const { choice, command } of [{ choice: "Lock", command: "protect_files" }, { choice: "Unlock", command: "remove_password" }] as const) {
    test(`${choice} adds files with Cmd+O and runs with Cmd+Enter`, async ({ page }) => {
        await openSecurity(page);
        await mode(page).getByRole("radio", { name: choice, exact: true }).click();
        const run = page.getByRole("button", { name: `${choice} files` });
        await expect(run).toHaveAttribute("aria-keyshortcuts", "Meta+Enter");
        await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveAttribute("aria-keyshortcuts", "Meta+O");

        await queueDialog(page, ["/local/report.pdf", "/local/lease.docx"]);
        await page.locator("body").press("Meta+O");
        await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(2);
        await page.locator("body").press("Meta+Enter");
        expect(await invocations(page, command)).toHaveLength(0);

        for (const field of await page.locator('.workspace-control-panel input[type="password"]').all()) await field.fill("test-password");
        await page.locator("body").press("Meta+Enter");
        await expect.poll(async () => (await invocations(page, command)).length).toBe(1);
        const [{ request }] = await invocations(page, command) as RunArgs[];
        expect(request.paths).toEqual(["/local/report.pdf", "/local/lease.docx"]);
    });
}

test("every result state and warning has a spoken label, and the privacy note is announced in the options", async ({ page }) => {
    const paths = [
        "/local/plain.pdf", "/local/signed.pdf", "/local/renamed.pdf", "/local/open.pdf", "/local/wrong.pdf",
        "/local/broken.pdf", "/local/notes.pdf", "/local/old.doc", "/local/huge.pdf", "/local/stopped.pdf", "/local/later.pdf",
    ];
    const { runId } = await startRun(page, "Unlock", paths);
    await expect(page.getByRole("group", { name: "Running batch" })).toBeVisible();
    await finish(page, [
        outcome(paths[0], { outputPaths: ["/local/plain-unlocked.pdf"] }),
        outcome(paths[1], { outputPaths: ["/local/signed-unlocked.pdf"], warnings: [{ code: "signatureRemoved", message: "" }] }),
        outcome(paths[2], { outputPaths: ["/local/renamed-unlocked-1.pdf"], warnings: [{ code: "renamedToAvoidReplacing", message: "", details: { chosenName: "renamed-unlocked.pdf", finalName: "renamed-unlocked-1.pdf" } }] }),
        outcome(paths[3], { state: "skipped", reason: { code: "notProtected", message: "This file has no password.", pointer: null } }),
        outcome(paths[4], { state: "failed", failure: failure("wrongPassword", "Wrong password.") }),
        outcome(paths[5], { state: "failed", failure: failure("processing", "This PDF is damaged.") }),
        outcome(paths[6], { state: "failed", failure: failure("invalidInput", "This isn't a PDF or Office file.") }),
        outcome(paths[7], { state: "failed", failure: failure("unavailable", "Legacy .doc files can't be unlocked here.") }),
        outcome(paths[8], { state: "failed", failure: failure("limitExceeded", "This file is over the size limit.", { limit: "fileSize" }) }),
        outcome(paths[9], { state: "failed", failure: failure("cancelled", "Stopped before it finished.") }),
        outcome(paths[10], { state: "skipped", reason: { code: "cancelled", message: "Cancelled", pointer: null } }),
    ]);
    expect(runId).toMatch(/\S+/);

    await expect(page.getByRole("list", { name: "Results" })).toMatchAriaSnapshot(`
      - list "Results":
        - listitem:
          - text: plain.pdf plain-unlocked.pdf Unlocked
        - listitem:
          - text: "signed.pdf Saved. The digital signature was removed, because changing the file breaks it. Unlocked with a warning"
        - listitem:
          - text: renamed.pdf Saved as renamed-unlocked-1.pdf. Renamed to avoid replacing an existing file. Unlocked with a warning
        - listitem:
          - text: open.pdf This file has no password. Skipped
        - listitem:
          - text: wrong.pdf The password didn't open this file. Check it, or try the password this file was sent with. Failed, wrong password
        - listitem:
          - text: broken.pdf This PDF is damaged. Failed while processing
        - listitem:
          - text: notes.pdf This isn't a PDF or Office file. Failed, the input can't be used
        - listitem:
          - text: old.doc Legacy .doc files can't be unlocked here. Unavailable
        - listitem:
          - text: huge.pdf This file is over the size limit. Failed, over the limit
        - listitem:
          - text: stopped.pdf Stopped before it finished. Failed, cancelled while it was running
        - listitem:
          - text: later.pdf Cancelled Skipped
    `);
    await expect(page.locator(".workspace-control-panel")).toContainText("Runs on this device with no network. Passwords are never saved and are cleared when you switch mode or leave.");
});
