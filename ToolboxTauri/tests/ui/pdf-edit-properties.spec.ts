import { expect, test, type Page } from "@playwright/test";

const deckPath = "/Users/test/Documents/deck.pdf";
const minutesPath = "/Users/test/Documents/minutes.pdf";
const scanPath = "/Users/test/Scans/scan.pdf";
const lockedPath = "/Users/test/Documents/locked.pdf";
const brokenPath = "/Users/test/Documents/broken.pdf";
const protectedMessage = "Edit Properties can't edit a password-protected PDF yet. Nothing was saved.";

const none = { keywords: null, creator: null, producer: null };
const current: Record<string, unknown> = {
  [deckPath]: { ...none, title: "Q3 board pack", author: "R. Okafor", subject: "Board", creationDate: "2026-09-30T17:12:00Z", modificationDate: "2026-10-01T09:00:00Z" },
  [minutesPath]: { ...none, title: "Board minutes", author: "R. Okafor", subject: null, creationDate: "2026-09-14T16:02:00+01:00", modificationDate: "2026-09-15T08:40:00+01:00" },
  [scanPath]: { ...none, title: null, author: null, subject: null, creationDate: null, modificationDate: null },
};
const keptDates = { creationDate: { action: "keep" }, modificationDate: { action: "keep" } };

type HarnessWindow = Window & { calls: Array<{ command: string; args: any }>; picked?: string[] };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ current, lockedPath, protectedMessage }) => {
    const w = window as unknown as HarnessWindow;
    w.calls = [];
    const outcome = (inputPath: string, rest: object) => ({ inputPath, outputPaths: [], detail: "", failure: null, reason: null, warnings: [], ...rest });
    (window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === "plugin:dialog|open") return w.picked;
        if (command === "plugin:path|resolve_directory") return "/Users/test";
        if (command === "check_output_folder") return "ready";
        if (command === "read_pdf_properties") {
          return args.request.paths.map((path: string) => current[path]
            ? outcome(path, { state: "succeeded", detail: JSON.stringify(current[path]) })
            : outcome(path, { state: "failed", failure: { kind: "invalidInput", message: path === lockedPath ? protectedMessage : "Could not read the PDF: invalid file header", pointer: null } }));
        }
        if (command === "edit_pdf_properties") {
          return args.request.paths.map((path: string) => path.endsWith("scan.pdf")
            ? outcome(path, { state: "skipped", reason: { code: "noEffectiveChange", message: "No effective change: title and author are already empty. No copy was made.", pointer: null } })
            : current[path]
            ? outcome(path, { state: "succeeded", outputPaths: [path.replace(/\.pdf$/, "-edited.pdf")], detail: "Title set, author cleared." })
            : outcome(path, { state: "failed", failure: { kind: "invalidInput", message: path === lockedPath ? protectedMessage : "Could not read the PDF: invalid file header", pointer: null } }));
        }
        return null;
      },
    };
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { current, lockedPath, protectedMessage });
  await page.goto("/tests/ui/harness/pdf-editor-batch.html?tool=pdf-properties");
});

const calls = (page: Page, command: string) =>
  page.evaluate((command) => (window as unknown as HarnessWindow).calls.filter((call) => call.command === command), command);
const addFiles = async (page: Page, paths: string[]) => {
  await page.evaluate((paths) => { (window as unknown as HarnessWindow).picked = paths; }, paths);
  await page.getByRole("button", { name: "Choose files to process" }).click();
};
const field = (page: Page, name: string) => page.getByRole("group", { name, exact: true });
const choose = (page: Page, name: string, action: "Keep" | "Set" | "Clear") => field(page, name).getByRole("radio", { name: action, exact: true }).click();
const row = (page: Page, name: string) => page.getByRole("option", { name, exact: true });
const reason = (page: Page) => page.getByTestId("pdf-properties-reason");
const results = (page: Page) => page.getByRole("list", { name: "Results" }).getByRole("listitem");

test("Edit Properties sits under Details after Privacy Check", async ({ page }) => {
  const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
  await expect(rail.getByRole("button", { name: "Edit Properties" })).toHaveAttribute("aria-pressed", "true");
  await expect(rail.getByRole("button", { name: "Edit Properties" })).toHaveAttribute("title", "Set, keep or clear title, author, subject and dates. Saves edited copies.");
  await expect(rail.getByRole("button", { name: "Privacy Check" }).locator("+ button")).toHaveAccessibleName("Edit Properties");
});

test("with no files every field keeps and Run is off", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Edit Properties" })).toBeVisible();
  for (const name of ["Title", "Author", "Subject", "Created", "Modified"]) {
    await expect(field(page, name).getByRole("radio", { name: "Keep", exact: true })).toHaveAttribute("aria-checked", "true");
    await expect(field(page, name)).toContainText("Add PDFs to see the current value.");
  }
  await expect(reason(page)).toHaveText("Add PDFs to edit.");
  await expect(page.getByRole("button", { name: /^Save edited copies/ })).toBeDisabled();
});

test("a batch shows each file's current values, summarises them per field and sends one patch for all files", async ({ page }) => {
  await addFiles(page, [deckPath, minutesPath, scanPath]);

  expect((await calls(page, "read_pdf_properties")).map(({ args }) => args)).toEqual([{ request: { paths: [deckPath, minutesPath, scanPath] } }]);
  await expect(row(page, "deck.pdf").getByLabel("Title now: Q3 board pack")).toBeVisible();
  await expect(row(page, "deck.pdf").getByLabel("Author now: R. Okafor")).toBeVisible();
  await expect(row(page, "deck.pdf").getByLabel("Modified now: 2026-10-01")).toBeVisible();
  await expect(row(page, "scan.pdf").getByLabel("Title now: (none)")).toBeVisible();
  await expect(row(page, "scan.pdf").getByLabel("Modified now: (none)")).toBeVisible();

  await expect(page.getByText("Changes apply to all 3 PDFs. Keep leaves each file's own value, even where files differ.")).toBeVisible();
  await expect(field(page, "Title")).toContainText("Now: 2 different titles, 1 file has none. Each keeps its own.");
  await expect(field(page, "Author")).toContainText("Now: 2 files have the same author, 1 file has none. Each keeps its own.");
  await expect(field(page, "Subject")).toContainText("Now: 1 file has a subject, 2 have none. Each keeps its own.");
  await expect(field(page, "Created")).toContainText("Now: 2 different dates, 1 file has none. Each keeps its own.");
  await expect(field(page, "Modified")).toContainText("Now: 2 different dates, 1 file has none. Each keeps its own.");
  await expect(reason(page)).toHaveText("Every field is set to Keep, so nothing would change. Choose Set or Clear for a field.");
  const save = page.getByRole("button", { name: /^Save 3 edited copies/ });
  await expect(save).toBeDisabled();

  await choose(page, "Title", "Set");
  await expect(reason(page)).toHaveText("Enter a title, or choose Keep or Clear.");
  await expect(save).toBeDisabled();
  await field(page, "Title").getByRole("textbox", { name: "New Title" }).fill("Q3 board pack");
  await expect(field(page, "Title")).toContainText("Now: 2 different titles, 1 file has none");
  await expect(field(page, "Title")).not.toContainText("Each keeps its own.");
  await expect(field(page, "Title")).toContainText("Replaces the title in all 3 files.");
  await choose(page, "Author", "Clear");
  await expect(field(page, "Author")).toContainText("Removed from all 3 files, including their XMP copies.");
  await expect(field(page, "Author").getByRole("textbox")).toHaveCount(0);
  await expect(reason(page)).toHaveText("Sets the title and clears the author. A file that already matches is skipped.");
  await expect(page.getByTestId("run-destination-example")).toContainText("deck-edited.pdf");

  await expect(save).toBeEnabled();
  await save.click();

  const runs = await calls(page, "edit_pdf_properties");
  expect(runs).toHaveLength(1);
  expect(runs[0].args).toEqual({
    runId: expect.any(String),
    request: {
      paths: [deckPath, minutesPath, scanPath],
      patch: { title: { action: "set", value: "Q3 board pack" }, author: { action: "clear" }, subject: { action: "keep" }, ...keptDates },
      outputLocation: "alongsideInput",
    },
  });

  await expect(results(page)).toHaveCount(3);
  for (const name of ["deck.pdf", "minutes.pdf"]) {
    const saved = results(page).filter({ hasText: name });
    await expect(saved).toHaveAttribute("data-state", "succeeded");
    await expect(saved).toContainText("Title set, author cleared.");
    await expect(saved.getByRole("button", { name: `Open ${name.replace(".pdf", "-edited.pdf")}`, exact: true })).toBeVisible();
  }
  const skipped = results(page).filter({ hasText: "scan.pdf" });
  await expect(skipped).toHaveAttribute("data-state", "skipped");
  await expect(skipped).toContainText("No effective change: title and author are already empty. No copy was made.");
  await expect(skipped.getByRole("button")).toHaveCount(0);
});

test("one file shows its own values and describes the change", async ({ page }) => {
  await addFiles(page, [minutesPath]);
  await expect(page.getByText("Current values of minutes.pdf are shown under each field.")).toBeVisible();
  await expect(field(page, "Title")).toContainText("Now: Board minutes");
  await expect(field(page, "Subject")).toContainText("Now: (none)");
  await expect(field(page, "Subject")).not.toContainText("Each keeps its own.");
  await expect(field(page, "Created")).toContainText("Now: 2026-09-14 16:02");
  await expect(field(page, "Modified")).toContainText("Now: 2026-09-15 08:40");
  await choose(page, "Author", "Set");
  await field(page, "Author").getByRole("textbox", { name: "New Author" }).fill("Board secretary");
  await choose(page, "Subject", "Clear");
  await expect(field(page, "Subject")).toContainText("Removed from the file, including its XMP copy.");
  await expect(reason(page)).toHaveText("Sets the author and clears the subject. Everything else is kept.");
  await page.getByRole("button", { name: /^Save edited copy/ }).click();
  const [run] = await calls(page, "edit_pdf_properties");
  expect(run.args.request.patch).toEqual({ title: { action: "keep" }, author: { action: "set", value: "Board secretary" }, subject: { action: "clear" }, ...keptDates });
});

test("a protected or broken file fails on its own row while the others save", async ({ page }) => {
  await addFiles(page, [lockedPath, deckPath, brokenPath]);
  await expect(row(page, "locked.pdf")).toContainText("Locked");
  await expect(row(page, "deck.pdf").getByLabel("Title now: Q3 board pack")).toBeVisible();
  await choose(page, "Title", "Clear");
  await page.getByRole("button", { name: /^Save 3 edited copies/ }).click();

  await expect(results(page)).toHaveCount(3);
  await expect(results(page).filter({ hasText: "locked.pdf" })).toHaveAttribute("data-state", "failed");
  await expect(results(page).filter({ hasText: "locked.pdf" })).toContainText(protectedMessage);
  await expect(results(page).filter({ hasText: "broken.pdf" })).toHaveAttribute("data-state", "failed");
  await expect(results(page).filter({ hasText: "deck.pdf" })).toHaveAttribute("data-state", "succeeded");
});

test("files stay when switching to Privacy Check and back", async ({ page }) => {
  await addFiles(page, [deckPath, scanPath]);
  await expect(row(page, "deck.pdf").getByLabel("Title now: Q3 board pack")).toBeVisible();
  const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
  await rail.getByRole("button", { name: "Privacy Check" }).click();
  await expect(page.getByRole("heading", { name: "Privacy Check" })).toBeVisible();
  await expect(row(page, "deck.pdf")).toBeVisible();
  await rail.getByRole("button", { name: "Edit Properties" }).click();
  await expect(row(page, "deck.pdf").getByLabel("Title now: Q3 board pack")).toBeVisible();
  await expect(row(page, "scan.pdf")).toBeVisible();
});

test.describe("dates in a fixed time zone", () => {
  test.use({ timezoneId: "Asia/Kolkata" });

  test("an invalid date is flagged in the field and blocks Save until it is fixed", async ({ page }) => {
    await addFiles(page, [deckPath, minutesPath]);
    const save = page.getByRole("button", { name: /^Save 2 edited copies/ });
    await choose(page, "Created", "Set");
    const created = field(page, "Created").getByRole("textbox", { name: "New Created" });
    await expect(created).toHaveAttribute("placeholder", "YYYY-MM-DD HH:MM, your local time");
    await expect(reason(page)).toHaveText("Enter a creation date, or choose Keep or Clear.");

    await created.fill("2026-02-30 09:00");
    const problem = field(page, "Created").getByRole("alert");
    await expect(problem).toHaveText("2026-02-30 isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.");
    await expect(created).toHaveAttribute("aria-invalid", "true");
    await expect(created).toHaveAccessibleDescription("2026-02-30 isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.");
    await expect(reason(page)).toHaveText("Fix the creation date to save.");
    await expect(save).toBeDisabled();

    await created.fill("28/02/2026 09:00");
    await expect(problem).toHaveText("Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.");
    await created.fill("2026-02-28 24:10");
    await expect(problem).toHaveText("24:10 isn't a real time. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.");
    await choose(page, "Modified", "Set");
    await field(page, "Modified").getByRole("textbox", { name: "New Modified" }).fill("2025-02-29 10:00");
    await expect(field(page, "Modified").getByRole("alert")).toHaveText("2025-02-29 isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.");
    await expect(reason(page)).toHaveText("Fix the creation date to save.");

    await created.fill("2026-02-28 09:00");
    await expect(field(page, "Created").getByRole("alert")).toHaveCount(0);
    await expect(created).not.toHaveAttribute("aria-invalid", "true");
    await expect(reason(page)).toHaveText("Fix the modification date to save.");
    await expect(save).toBeDisabled();
    expect(await calls(page, "edit_pdf_properties")).toHaveLength(0);
  });

  test("set dates are sent with the local time zone, clear removes them and keep leaves them out", async ({ page }) => {
    await addFiles(page, [deckPath, minutesPath]);
    await choose(page, "Created", "Set");
    await field(page, "Created").getByRole("textbox", { name: "New Created" }).fill(" 2026-02-28 09:00 ");
    await expect(field(page, "Created")).toContainText("Now: 2 different dates");
    await expect(field(page, "Created")).not.toContainText("Each keeps its own.");
    await expect(field(page, "Created")).toContainText("Replaces the creation date in all 2 files.");
    await choose(page, "Modified", "Clear");
    await expect(field(page, "Modified")).toContainText("Removed from all 2 files, including their XMP copies.");
    await expect(reason(page)).toHaveText("Sets the creation date and clears the modification date. A file that already matches is skipped.");
    await page.getByRole("button", { name: /^Save 2 edited copies/ }).click();

    const [run] = await calls(page, "edit_pdf_properties");
    expect(run.args.request.patch).toEqual({
      title: { action: "keep" }, author: { action: "keep" }, subject: { action: "keep" },
      creationDate: { action: "set", value: "2026-02-28T09:00:00+05:30" },
      modificationDate: { action: "clear" },
    });
  });
});

test.describe("dates in a time zone with daylight saving", () => {
  test.use({ timezoneId: "America/New_York" });

  test("each date carries the offset in force on that day", async ({ page }) => {
    await addFiles(page, [minutesPath]);
    await choose(page, "Created", "Set");
    await field(page, "Created").getByRole("textbox", { name: "New Created" }).fill("2026-01-15 08:30");
    await choose(page, "Modified", "Set");
    await field(page, "Modified").getByRole("textbox", { name: "New Modified" }).fill("2026-07-04 23:59");
    await expect(reason(page)).toHaveText("Sets the creation date and modification date. Everything else is kept.");
    await page.getByRole("button", { name: /^Save edited copy/ }).click();

    const [run] = await calls(page, "edit_pdf_properties");
    expect(run.args.request.patch).toEqual({
      title: { action: "keep" }, author: { action: "keep" }, subject: { action: "keep" },
      creationDate: { action: "set", value: "2026-01-15T08:30:00-05:00" },
      modificationDate: { action: "set", value: "2026-07-04T23:59:00-04:00" },
    });
  });
});
