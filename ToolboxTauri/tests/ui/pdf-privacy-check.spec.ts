import { expect, test, type Locator, type Page } from "@playwright/test";

const cleanPath = "/Users/test/Documents/clean.pdf";
const leakyPath = "/Users/test/Documents/lease.pdf";
const brokenPath = "/Users/test/Documents/broken.pdf";

const noProperties = { title: null, author: null, subject: null, keywords: null, creator: null, producer: null, creationDate: null, modificationDate: null };
const cleanReport = {
  pageCount: 1,
  documentInformation: noProperties,
  xmp: { summary: null, conflicts: [] },
  encryption: null,
  identifiers: { documentId: null, producer: null },
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
  unsupported: ["The document ID could not be read."],
};

type HarnessWindow = Window & { calls: Array<{ command: string; args: any }> };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ cleanPath, leakyPath, brokenPath, cleanReport, leakyReport }) => {
    const w = window as unknown as HarnessWindow;
    w.calls = [];
    const reports: Record<string, unknown> = { [cleanPath]: cleanReport, [leakyPath]: leakyReport };
    (window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === "plugin:dialog|open") return [cleanPath, leakyPath, brokenPath];
        if (command === "plugin:path|resolve_directory") return "/Users/test";
        if (command === "check_output_folder") return "ready";
        if (command === "inspect_pdf_privacy") {
          return args.request.paths.map((inputPath: string) => reports[inputPath]
            ? { inputPath, outputPaths: [], state: "succeeded", detail: JSON.stringify(reports[inputPath]), failure: null, reason: null, warnings: [] }
            : { inputPath, outputPaths: [], state: "failed", detail: "", failure: { kind: "processing", message: "Could not read the PDF: invalid file header", pointer: null }, reason: null, warnings: [] });
        }
        return null;
      },
    };
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { cleanPath, leakyPath, brokenPath, cleanReport, leakyReport });
  // A custom save folder proves the read-only tool never checks or uses a destination.
  await page.goto("/tests/ui/harness/pdf-editor-batch.html?tool=pdf-privacy&folder=/Users/test/Reports");
});

const calls = (page: Page, command: string) =>
  page.evaluate((command) => (window as unknown as HarnessWindow).calls.filter((call) => call.command === command), command);
const report = (page: Page, name: string) => page.getByRole("region", { name, exact: true });
const category = (file: Locator, name: string) => file.getByRole("listitem").filter({ has: file.page().getByRole("heading", { name, exact: true }) });
const categoryNames = ["Document information", "XMP metadata", "Encryption and permissions", "Document IDs", "Unsupported items"];

test("Privacy Check sits under Details in the PDF Editor rail", async ({ page }) => {
  const rail = page.getByRole("toolbar", { name: "PDF Editor tools" });
  await expect(rail.locator(".workspace-command-group")).toHaveText(["Details", "More"]);
  await expect(rail.locator(".workspace-command-group").first().locator("+ button")).toHaveAccessibleName("Privacy Check");
  await expect(rail.getByRole("button", { name: "Privacy Check" })).toHaveAttribute("aria-pressed", "true");
  await expect(rail.getByRole("button", { name: "Privacy Check" })).toHaveAttribute("title", "See what each PDF reveals besides its pages. Changes nothing.");
});

test("Privacy Check sends only the paths and shows one report or error per file without saving anything", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Privacy Check" })).toBeVisible();
  await expect(page.getByRole("button", { name: /^Check PDFs/ })).toBeDisabled();
  await page.getByRole("button", { name: "Choose files to process" }).click();
  await expect(page.getByText("Nothing is changed or saved. Your original stays unchanged.")).toBeVisible();
  await expect(page.getByText("Reads each PDF and makes a report. No file is changed or saved.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Change for this run" })).toHaveCount(0);

  await page.getByRole("button", { name: /^Check 3 PDFs/ }).click();

  const runs = await calls(page, "inspect_pdf_privacy");
  expect(runs).toHaveLength(1);
  expect(runs[0].args).toEqual({ runId: expect.any(String), request: { paths: [cleanPath, leakyPath, brokenPath] } });
  expect(await calls(page, "check_output_folder")).toEqual([]);
  expect(await calls(page, "plugin:dialog|save")).toEqual([]);

  await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 2 reports, 1 failedNothing was changed in any PDF.");

  const clean = report(page, "clean.pdf");
  await expect(clean.getByText("~/Documents · 1 page")).toBeVisible();
  await expect(clean.getByRole("list", { name: "Report categories" }).getByRole("heading")).toHaveText(categoryNames);
  for (const name of categoryNames) await expect(category(clean, name)).toHaveText(`${name}None found`);
  await expect(clean.getByText("5 categories with none found. Everything was checked.")).toBeVisible();
  await expect(clean.getByRole("alert")).toHaveCount(0);

  const leaky = report(page, "lease.pdf");
  await expect(leaky.getByText("~/Documents · 14 pages")).toBeVisible();
  await expect(leaky.getByText("0 high")).toBeVisible();
  await expect(leaky.getByText("3 medium")).toBeVisible();
  await expect(leaky.getByText("2 low")).toBeVisible();
  await expect(leaky.getByText("0 categories with none found. 1 item wasn't checked.")).toBeVisible();
  const expected: Array<[string, string, string, string, Array<[string, string]>]> = [
    ["Document information", "Medium", "4 fields", "Anyone you send the file to can see who made it, with which apps, and when.", [
      ["Author", "Asha Patel"], ["Created with", "Microsoft Word"], ["Saved as PDF by", "Acrobat Distiller 9.0.1 (Windows)"], ["Created", "2026-03-02 09:14"],
    ]],
    ["XMP metadata", "Medium", "2 items", "A second copy of the properties. It can keep old names after the visible ones change.", [
      ["Author differs", 'XMP says "A. Patel (draft)"'], ["Stored in XMP", "dc:creator, xmpMM:DocumentID"],
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
    ["Unsupported items", "Medium", "1 item", "These parts weren't checked, so the report can't say what they hold.", [
      ["Item 1", "The document ID could not be read."],
    ]],
  ];
  for (const [name, severity, count, why, items] of expected) {
    const row = category(leaky, name);
    await expect(row.getByText(severity, { exact: true })).toBeVisible();
    await expect(row.getByText(count, { exact: true })).toBeVisible();
    await expect(row.getByText(`Why it matters: ${why}`)).toBeVisible();
    await expect(row.locator("dt")).toHaveText(items.map(([key]) => key));
    await expect(row.locator("dd")).toHaveText(items.map(([, value]) => value));
    await expect(row.getByText("None found")).toHaveCount(0);
  }

  const broken = report(page, "broken.pdf");
  await expect(broken.getByText("~/Documents · Not read")).toBeVisible();
  await expect(broken.getByRole("alert")).toHaveText("Failed: Could not read the PDF: invalid file header");
  await expect(broken.getByRole("list", { name: "Report categories" })).toHaveCount(0);
  await expect(clean.getByRole("alert")).toHaveCount(0);
  await expect(leaky.getByRole("alert")).toHaveCount(0);
});
