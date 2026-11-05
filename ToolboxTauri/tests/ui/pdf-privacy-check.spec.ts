import { expect, test, type Locator, type Page } from "@playwright/test";

const cleanPath = "/Users/test/Documents/clean.pdf";
const leakyPath = "/Users/test/Documents/lease.pdf";
const ndaPath = "/Users/test/Documents/nda.pdf";
const brokenPath = "/Users/test/Documents/broken.pdf";
const lockedPath = "/Users/test/Documents/locked.pdf";
const lockedPassword = "locked-secret";

const noProperties = { title: null, author: null, subject: null, keywords: null, creator: null, producer: null, creationDate: null, modificationDate: null };
const nothingHidden = {
  incrementalUpdates: 0,
  embeddedFiles: [],
  actions: [],
  annotations: [],
  filledFormFields: [],
  optionalContent: { layers: [], hidden: [] },
  links: [],
  signatures: [],
  leftovers: { thumbnailPages: [], pagePieces: [] },
};
const cleanReport = {
  pageCount: 1,
  documentInformation: noProperties,
  xmp: { summary: null, conflicts: [] },
  encryption: null,
  identifiers: { documentId: null, producer: null },
  ...nothingHidden,
  unsupported: [],
};
const leakyReport = {
  pageCount: 14,
  documentInformation: { ...noProperties, author: "Asha Patel", creator: "Microsoft Word", producer: "Acrobat Distiller 9.0.1 (Windows)", creationDate: "2026-03-02T09:14:05+01:00" },
  xmp: {
    summary: { ...noProperties, author: "A. Patel (draft)", properties: ["dc:creator", "xmpMM:DocumentID"] },
    conflicts: [{ property: "author", info: "Asha Patel", xmp: "A. Patel (draft)" }],
  },
  encryption: { openPassword: false, version: 2, revision: 3, keyBits: 128, encryptsMetadata: true, allowed: ["print", "copyForAccessibility"], denied: ["printHighQuality", "modify", "copy", "annotate", "fillForms", "assemble"] },
  identifiers: {
    documentId: { permanent: "9f3c0011223344556677a71e", changing: "9f3c0011223344556677a71e", unchanged: true },
    producer: { name: "Acrobat Distiller 9.0.1 (Windows)", version: "9.0.1" },
  },
  incrementalUpdates: 2,
  embeddedFiles: [
    { name: "rent-schedule.xlsx", size: 49152, mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", page: 1 },
    { name: "notes.txt", size: 12, mimeType: null, page: null },
  ],
  actions: [
    { trigger: "documentScript", page: null, name: "init", kinds: ["JavaScript"] },
    { trigger: "beforePrint", page: null, name: null, kinds: ["JavaScript", "Launch"] },
    { trigger: "pageOpen", page: 3, name: null, kinds: ["URI"] },
    { trigger: "annotation", page: 2, name: null, kinds: ["SubmitForm", "Custom"] },
  ],
  annotations: [
    { subtype: "FreeText", count: 2, authors: ["J. Morgan"] },
    { subtype: "Square", count: 1, authors: [] },
    { subtype: "Text", count: 3, authors: ["Asha Patel", "J. Morgan"] },
  ],
  filledFormFields: ["Tenant name", "Start date", "Deposit"],
  optionalContent: {
    layers: [{ name: "Draft notes", visible: false }, { name: "Final", visible: true }],
    hidden: [{ page: 2, subtype: "Text", reason: "hidden" }, { page: 14, subtype: "Square", reason: "offPage" }],
  },
  links: [{ host: "elmrow-lettings.example", count: 3 }, { host: "gov.uk", count: 1 }],
  signatures: [],
  leftovers: { thumbnailPages: [1, 2], pagePieces: [{ page: 1, applications: ["Illustrator", "Photoshop"] }] },
  unsupported: ["The document ID could not be read."],
};
const ndaReport = {
  ...cleanReport,
  pageCount: 6,
  incrementalUpdates: null,
  annotations: [{ subtype: "Text", count: 1, authors: [] }],
  optionalContent: { layers: [], hidden: [{ page: 1, subtype: "Stamp", reason: "hidden" }] },
  links: [{ host: "file:", count: 1 }],
  signatures: [
    { field: "approver", signer: "Acme Ltd", signedAt: "2026-02-11T09:30:00+01:00" },
    { field: "witness", signer: null, signedAt: null },
  ],
  unsupported: ["The file's save history could not be read."],
};

type HarnessWindow = Window & { calls: Array<{ command: string; args: any }>; navigations: unknown[]; picked?: string[] };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ cleanPath, leakyPath, ndaPath, brokenPath, lockedPath, lockedPassword, cleanReport, leakyReport, ndaReport }) => {
    const w = window as unknown as HarnessWindow;
    w.calls = [];
    const reports: Record<string, unknown> = { [cleanPath]: cleanReport, [leakyPath]: leakyReport, [ndaPath]: ndaReport };
    (window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === "plugin:dialog|open") return w.picked ?? [cleanPath, leakyPath, ndaPath, brokenPath];
        if (command === "plugin:path|resolve_directory") return "/Users/test";
        if (command === "check_output_folder") return "ready";
        if (command === "inspect_pdf_privacy") {
          const { paths, password, passwordOverrides } = args.request;
          return paths.map((inputPath: string) => inputPath === lockedPath && (passwordOverrides?.[inputPath] || password) !== lockedPassword
            ? { inputPath, outputPaths: [], state: "failed", detail: "", failure: { kind: "wrongPassword", message: "The password didn't open this file.", pointer: null }, reason: null, warnings: [] }
            : inputPath === lockedPath
            ? { inputPath, outputPaths: [], state: "succeeded", detail: JSON.stringify({ ...cleanReport, encryption: { openPassword: true, version: 4, revision: 4, keyBits: 128, encryptsMetadata: true, allowed: [], denied: [] } }), failure: null, reason: null, warnings: [] }
            : reports[inputPath]
            ? { inputPath, outputPaths: [], state: "succeeded", detail: JSON.stringify(reports[inputPath]), failure: null, reason: null, warnings: [] }
            : { inputPath, outputPaths: [], state: "failed", detail: "", failure: { kind: "processing", message: "Could not read the PDF: invalid file header", pointer: null }, reason: null, warnings: [] });
        }
        return null;
      },
    };
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { cleanPath, leakyPath, ndaPath, brokenPath, lockedPath, lockedPassword, cleanReport, leakyReport, ndaReport });
  // A custom save folder proves the read-only tool never checks or uses a destination.
  await page.goto("/tests/ui/harness/pdf-editor-batch.html?tool=pdf-privacy&folder=/Users/test/Reports");
});

const calls = (page: Page, command: string) =>
  page.evaluate((command) => (window as unknown as HarnessWindow).calls.filter((call) => call.command === command), command);
const report = (page: Page, name: string) => page.getByRole("region", { name, exact: true });
const category = (file: Locator, name: string) => file.getByRole("listitem").filter({ has: file.page().getByRole("heading", { name, exact: true }) });
const categoryNames = [
  "Document information", "XMP metadata", "Earlier versions (incremental updates)", "Embedded files", "JavaScript and automatic actions",
  "Comments and their authors", "Filled form fields", "Hidden layers (optional content)", "Links and the hosts they reach", "Digital signatures",
  "Encryption and permissions", "Document IDs", "Thumbnails and page piece data", "Unsupported items",
];
type Expected = [name: string, severity: string, count: string, why: string, items: Array<[string, string]>];
const expectFound = async (file: Locator, expected: Expected[]) => {
  for (const [name, severity, count, why, items] of expected) {
    const row = category(file, name);
    await expect(row.getByText(severity, { exact: true })).toBeVisible();
    await expect(row.getByText(count, { exact: true })).toBeVisible();
    await expect(row.getByText(`Why it matters: ${why}`)).toBeVisible();
    await expect(row.locator("dt")).toHaveText(items.map(([key]) => key));
    await expect(row.locator("dd")).toHaveText(items.map(([, value]) => value));
    await expect(row.getByText("None found")).toHaveCount(0);
  }
};
const expectNoneFound = async (file: Locator, names: string[]) => {
  for (const name of names) {
    await expect(category(file, name)).toHaveText(`${name}None found`);
    await expect(category(file, name).getByText("Why it matters")).toHaveCount(0);
  }
};

test("Privacy Check sits under Details in the PDF Editor rail", async ({ page }) => {
  const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
  await expect(rail.locator(".workspace-command-group")).toHaveText(["Details", "More"]);
  await expect(rail.locator(".workspace-command-group").first().locator("+ button")).toHaveAccessibleName("Privacy Check");
  await expect(rail.getByRole("button", { name: "Privacy Check" })).toHaveAttribute("aria-pressed", "true");
  await expect(rail.getByRole("button", { name: "Privacy Check" })).toHaveAttribute("title", "See what each PDF reveals besides its pages. Changes nothing.");
});

test("Privacy Check sends the paths with no password and shows one report or error per file without saving anything", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Privacy Check" })).toBeVisible();
  await expect(page.getByRole("button", { name: /^Check PDFs/ })).toBeDisabled();
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await expect(page.getByText("Nothing is changed or saved. Your original stays unchanged.")).toBeVisible();
  await expect(page.getByText("Reads each PDF and makes a report. No file is changed or saved.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Change for this run" })).toHaveCount(0);

  await page.getByRole("button", { name: /^Check 4 PDFs/ }).click();

  const runs = await calls(page, "inspect_pdf_privacy");
  expect(runs).toHaveLength(1);
  expect(runs[0].args).toEqual({ runId: expect.any(String), request: { paths: [cleanPath, leakyPath, ndaPath, brokenPath], password: "", passwordOverrides: {} } });
  expect(await calls(page, "check_output_folder")).toEqual([]);
  expect(await calls(page, "plugin:dialog|save")).toEqual([]);

  await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 3 reports, 1 failedNothing was changed in any PDF.");

  const clean = report(page, "clean.pdf");
  await expect(clean.getByText("~/Documents · 1 page")).toBeVisible();
  await expect(clean.getByRole("list", { name: "Report categories" }).getByRole("heading")).toHaveText(categoryNames);
  await expectNoneFound(clean, categoryNames);
  await expect(clean.getByText("14 categories with none found. Everything was checked.")).toBeVisible();
  await expect(clean.getByRole("button")).toHaveCount(0);
  await expect(clean.getByRole("alert")).toHaveCount(0);

  const leaky = report(page, "lease.pdf");
  await expect(leaky.getByText("~/Documents · 14 pages")).toBeVisible();
  await expect(leaky.getByRole("list", { name: "Report categories" }).getByRole("heading")).toHaveText(categoryNames);
  await expect(leaky.getByText("3 high")).toBeVisible();
  await expect(leaky.getByText("6 medium")).toBeVisible();
  await expect(leaky.getByText("4 low")).toBeVisible();
  await expect(leaky.getByText("1 category with none found. 1 item wasn't checked.")).toBeVisible();
  await expectFound(leaky, [
    ["Document information", "Medium", "4 fields", "Anyone you send the file to can see who made it, with which apps, and when.", [
      ["Author", "Asha Patel"], ["Created with", "Microsoft Word"], ["Saved as PDF by", "Acrobat Distiller 9.0.1 (Windows)"], ["Created", "2026-03-02 09:14"],
    ]],
    ["XMP metadata", "Medium", "2 items", "A second copy of the properties. It can keep old names after the visible ones change.", [
      ["Author differs", 'XMP says "A. Patel (draft)"'], ["Stored in XMP", "dc:creator, xmpMM:DocumentID"],
    ]],
    ["Earlier versions (incremental updates)", "High", "2 earlier saves", "Earlier saves are still inside the file. Text you deleted may be recoverable.", [
      ["Earlier saves kept", "2"],
    ]],
    ["Embedded files", "High", "2 files", "Attached files travel with the PDF, even though they aren't on any page.", [
      ["rent-schedule.xlsx", "48 KB, application/vnd.openxmlformats-officedocument.spreadsheetml.sheet, attached to page 1"],
      ["notes.txt", "12 bytes, attached to the file"],
    ]],
    ["JavaScript and automatic actions", "High", "4 actions", "Scripts and actions can run when the PDF opens, or contact other sites.", [
      ['Document script "init"', "Runs a script"],
      ["Before printing", "Runs a script, starts another app or file"],
      ["When a page opens, page 3", "Opens a website"],
      ["From a field or comment, page 2", "Sends form answers, custom action"],
    ]],
    ["Comments and their authors", "Medium", "6 comments, 2 authors", "Comments show reviewers' names and can hold notes nobody sees on the page.", [
      ["Authors", "Asha Patel, J. Morgan"], ["Free text", "2, by J. Morgan"], ["Square", "1, no author"], ["Text", "3, by Asha Patel, J. Morgan"],
    ]],
    ["Filled form fields", "Low", "3 fields", "Typed answers stay in the file, including fields that are hidden or empty-looking.", [
      ["Filled", "Tenant name, Start date, Deposit"],
    ]],
    ["Hidden layers (optional content)", "Medium", "2 layers, 1 off, 2 hidden on the page", "Hidden layers can hold content that doesn't show until someone turns them on.", [
      ["Off by default", "Draft notes"], ["On by default", "Final"], ["Page 2", "Text annotation set to hidden"], ["Page 14", "Square annotation placed off the page"],
    ]],
    ["Links and the hosts they reach", "Low", "4 links, 2 hosts", "Opening a link tells that site the file was opened, and from where.", [
      ["Hosts", "elmrow-lettings.example (3), gov.uk"],
    ]],
    ["Encryption and permissions", "Low", "Restricted", "Shows who can open the file and what readers are allowed to do with it.", [
      ["No open password", "Anyone can open it"],
      ["Not allowed", "High-quality printing, editing, copying text, adding comments, filling forms, assembling pages"],
      ["Allowed", "Printing, copying for accessibility"],
      ["Encryption", "128-bit, version 2, revision 3"],
    ]],
    ["Document IDs", "Low", "1 ID, 1 app version", "A fixed ID can link this file to other copies or earlier versions of it.", [
      ["Document ID", "9f3c…a71e, unchanged since the first save"], ["App version", '9.0.1, from "Acrobat Distiller 9.0.1 (Windows)"'],
    ]],
    ["Thumbnails and page piece data", "Medium", "2 thumbnails, 1 page with pieces", "Stored thumbnails and page pieces can show content that was later removed.", [
      ["Thumbnails", "Pages 1, 2"], ["Page 1 pieces", "Saved by Illustrator, Photoshop"],
    ]],
    ["Unsupported items", "Medium", "1 item", "These parts weren't checked, so the report can't say what they hold.", [
      ["Item 1", "The document ID could not be read."],
    ]],
  ]);
  await expectNoneFound(leaky, ["Digital signatures"]);
  await expect(leaky.getByRole("button")).toHaveText(["Open Edit PDF"]);
  await expect(category(leaky, "Filled form fields").getByRole("button", { name: "Open Edit PDF" })).toBeVisible();

  const nda = report(page, "nda.pdf");
  await expect(nda.getByText("~/Documents · 6 pages")).toBeVisible();
  await expect(nda.getByText("0 high")).toBeVisible();
  await expect(nda.getByText("3 medium")).toBeVisible();
  await expect(nda.getByText("2 low")).toBeVisible();
  await expect(nda.getByText("8 categories with none found. 1 item wasn't checked.")).toBeVisible();
  await expectFound(nda, [
    ["Comments and their authors", "Medium", "1 comment", "Comments show reviewers' names and can hold notes nobody sees on the page.", [["Text", "1, no author"]]],
    ["Hidden layers (optional content)", "Medium", "1 hidden on the page", "Hidden layers can hold content that doesn't show until someone turns them on.", [["Page 1", "Stamp annotation set to hidden"]]],
    ["Links and the hosts they reach", "Low", "1 link, 1 host", "Opening a link tells that site the file was opened, and from where.", [["Hosts", "file:"]]],
    ["Digital signatures", "Low", "2 signatures", "A signature names who signed and when. Changing the file removes it.", [
      ["Signed by", "Acme Ltd, 2026-02-11. Any change to the file removes it."], ["Signed by", 'Field "witness". Any change to the file removes it.'],
    ]],
    ["Unsupported items", "Medium", "1 item", "These parts weren't checked, so the report can't say what they hold.", [["Item 1", "The file's save history could not be read."]]],
  ]);
  const unchecked = category(nda, "Earlier versions (incremental updates)");
  await expect(unchecked).toHaveText("Earlier versions (incremental updates)Couldn't be read. See Unsupported items.");
  await expect(unchecked).toHaveAttribute("data-found", "unchecked");
  await expectNoneFound(nda, ["Document information", "XMP metadata", "Embedded files", "JavaScript and automatic actions", "Filled form fields", "Encryption and permissions", "Document IDs", "Thumbnails and page piece data"]);
  await expect(nda.getByRole("button")).toHaveCount(0);

  const broken = report(page, "broken.pdf");
  await expect(broken.getByText("~/Documents · Not read")).toBeVisible();
  await expect(broken.getByRole("alert")).toHaveText("Failed: Could not read the PDF: invalid file header");
  await expect(broken.getByRole("list", { name: "Report categories" })).toHaveCount(0);
  await expect(clean.getByRole("alert")).toHaveCount(0);
  await expect(leaky.getByRole("alert")).toHaveCount(0);
  await expect(nda.getByRole("alert")).toHaveCount(0);
});

test("A filled form field pointer opens Edit PDF with that report's file", async ({ page }) => {
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await page.getByRole("button", { name: /^Check 4 PDFs/ }).click();
  await category(report(page, "lease.pdf"), "Filled form fields").getByRole("button", { name: "Open Edit PDF" }).click();

  expect(await page.evaluate(() => (window as unknown as HarnessWindow).navigations)).toEqual([{ utilityId: "pdf-edit", initialPaths: [leakyPath] }]);
  await expect(page.getByRole("toolbar", { name: "PDF Editor tools" }).getByRole("button", { name: "Edit PDF" })).toHaveAttribute("aria-pressed", "true");
});

test("The form field pointer is hidden when Edit PDF is turned off", async ({ page }) => {
  await page.goto("/tests/ui/harness/pdf-editor-batch.html?tool=pdf-privacy&disabled=pdf-edit");
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await page.getByRole("button", { name: /^Check 4 PDFs/ }).click();

  const forms = category(report(page, "lease.pdf"), "Filled form fields");
  await expect(forms.getByText("Filled", { exact: true })).toBeVisible();
  await expect(forms.getByRole("button")).toHaveCount(0);
});

const pick = async (page: Page, paths: string[]) => {
  await page.evaluate((paths) => { (window as unknown as HarnessWindow).picked = paths; }, paths);
  await page.getByRole("button", { name: "Choose files to process" }).click();
};
const fileRow = (page: Page, name: string) => page.locator(".file-selection-item", { hasText: name });
const setOwnPassword = async (page: Page, name: string, password: string) => {
  await fileRow(page, name).click();
  await page.getByRole("button", { name: `Set own password for ${name}`, exact: true }).click();
  const editor = page.getByRole("group", { name: `Own password for ${name}` });
  await editor.getByLabel("Own open password", { exact: true }).fill(password);
  await editor.getByRole("button", { name: "Save password" }).click();
  await expect(editor).toHaveCount(0);
};
const pageSecrets = (page: Page) => page.evaluate(() => {
  const storage = (store: Storage) => Object.keys(store).map((key) => `${key}=${store.getItem(key)}`);
  return [document.body.innerText, ...storage(localStorage), ...storage(sessionStorage), document.cookie].join("\n");
});

test("An open password for all files is offered, and a file can have its own", async ({ page }) => {
  await pick(page, [cleanPath, lockedPath]);
  const batch = page.getByLabel("Open password for all files", { exact: true });
  await expect(batch).toHaveAttribute("placeholder", "Only for PDFs that need one");
  await expect(batch).toHaveAttribute("type", "password");
  await expect(page.getByText("Only used for PDFs that need a password to open.")).toBeVisible();
  await expect(page.getByText("Never saved. Cleared when you clear the list.")).toBeVisible();
  await expect(page.getByText("Password for all files, if needed")).toHaveCount(2);

  await setOwnPassword(page, "locked.pdf", lockedPassword);
  await expect(page.getByText("Only used for PDFs that need a password to open. 1 PDF uses its own password.")).toBeVisible();
  await expect(page.getByText("Own password", { exact: true })).toHaveCount(1);
  await expect(page.getByText("Password for all files, if needed")).toHaveCount(1);
});

test("Privacy Check sends the batch open password with only the run's own passwords, and the own password wins", async ({ page }) => {
  await pick(page, [cleanPath, lockedPath, leakyPath]);
  await page.getByLabel("Open password for all files", { exact: true }).fill("batch-secret");
  await setOwnPassword(page, "locked.pdf", lockedPassword);
  await page.getByRole("button", { name: /^Check 3 PDFs/ }).click();

  const runs = await calls(page, "inspect_pdf_privacy");
  expect(runs.map((run) => run.args.request)).toEqual([
    { paths: [cleanPath, lockedPath, leakyPath], password: "batch-secret", passwordOverrides: { [lockedPath]: lockedPassword } },
  ]);
  await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 3 reportsNothing was changed in any PDF.");
  const locked = report(page, "locked.pdf");
  await expect(locked.getByRole("alert")).toHaveCount(0);
  await expect(category(locked, "Encryption and permissions").getByText("Needed to open it", { exact: true })).toBeVisible();
  expect(await pageSecrets(page)).not.toContain(lockedPassword);
  expect(await pageSecrets(page)).not.toContain("batch-secret");
});

test("A wrong password fails only that file, and retrying sends only that file with its new password", async ({ page }) => {
  await pick(page, [cleanPath, lockedPath, leakyPath]);
  await page.getByLabel("Open password for all files", { exact: true }).fill("batch-secret");
  await page.getByRole("button", { name: /^Check 3 PDFs/ }).click();

  await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 2 reports, 1 failedNothing was changed in any PDF.");
  const locked = report(page, "locked.pdf");
  await expect(locked.getByRole("alert")).toHaveText("Failed: The password didn't open this file. Check it, or try the password this file was sent with.");
  await expect(locked.getByRole("list", { name: "Report categories" })).toHaveCount(0);
  await expect(report(page, "clean.pdf").getByRole("alert")).toHaveCount(0);
  await expect(report(page, "lease.pdf").getByRole("alert")).toHaveCount(0);
  await expect(report(page, "clean.pdf").getByRole("button", { name: "Try another password" })).toHaveCount(0);

  const tryAnother = locked.getByRole("button", { name: "Try another password" });
  await expect(tryAnother).toHaveAttribute("aria-expanded", "false");
  await tryAnother.click();
  await expect(tryAnother).toHaveAttribute("aria-expanded", "true");
  const retry = page.getByRole("group", { name: "Retry locked.pdf with another password" });
  await expect(retry).toContainText("Only this file is checked again. Its new password isn't saved.");
  await retry.getByLabel("Password for locked.pdf", { exact: true }).fill(lockedPassword);
  await retry.getByRole("button", { name: "Retry this file" }).click();

  await expect(locked.getByRole("alert")).toHaveCount(0);
  await expect(locked.getByRole("list", { name: "Report categories" })).toBeVisible();
  await expect(retry).toHaveCount(0);
  await expect(report(page, "clean.pdf")).toBeVisible();
  await expect(report(page, "lease.pdf")).toBeVisible();
  expect((await calls(page, "inspect_pdf_privacy")).map((run) => run.args.request)).toEqual([
    { paths: [cleanPath, lockedPath, leakyPath], password: "batch-secret", passwordOverrides: {} },
    { paths: [lockedPath], password: "", passwordOverrides: { [lockedPath]: lockedPassword } },
  ]);
  await expect(fileRow(page, "locked.pdf")).not.toContainText("Own password");
  expect(await pageSecrets(page)).not.toContain(lockedPassword);
});

test("PDFs that need no password are checked without one, and clearing the list forgets every password", async ({ page }) => {
  await pick(page, [cleanPath, lockedPath]);
  await page.getByRole("button", { name: /^Check 2 PDFs/ }).click();
  expect((await calls(page, "inspect_pdf_privacy"))[0].args.request).toEqual({ paths: [cleanPath, lockedPath], password: "", passwordOverrides: {} });
  await expect(report(page, "clean.pdf").getByRole("alert")).toHaveCount(0);
  await expect(report(page, "locked.pdf").getByRole("alert")).toHaveText("Failed: The password didn't open this file. Check it, or try the password this file was sent with.");

  await page.getByLabel("Open password for all files", { exact: true }).fill("batch-secret");
  await setOwnPassword(page, "locked.pdf", lockedPassword);
  await page.getByRole("button", { name: "Clear list" }).click();
  await pick(page, [cleanPath, lockedPath]);
  await expect(page.getByLabel("Open password for all files", { exact: true })).toHaveValue("");
  await expect(page.getByText("Own password", { exact: true })).toHaveCount(0);
});
