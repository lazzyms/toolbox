import { expect, test, type Page } from "@playwright/test";
import { openToolBySearch } from "./open-tool";

// PDF Details in the real shell: an over-limit or damaged PDF fails on its own row beside the ones that work, and
// both tools run from the keyboard with the platform's keys.

const cleanPath = "/Users/test/Documents/clean.pdf";
const bigPath = "/Users/test/Documents/big.pdf";
const damagedPath = "/Users/test/Documents/damaged.pdf";
const deckPath = "/Users/test/Documents/deck.pdf";
const paths = [cleanPath, bigPath, damagedPath, deckPath];
const SIZE_LIMIT = "Larger than the 1 GB limit for one PDF. Reduce its size and try again.";
const DAMAGED = "Could not read the PDF: invalid file header";

const report = {
  pageCount: 1,
  documentInformation: { title: null, author: null, subject: null, keywords: null, creator: null, producer: null, creationDate: null, modificationDate: null },
  xmp: { summary: null, conflicts: [] },
  encryption: null,
  identifiers: { documentId: null, producer: null },
  incrementalUpdates: 0,
  embeddedFiles: [],
  actions: [],
  annotations: [],
  filledFormFields: [],
  optionalContent: { layers: [], hidden: [] },
  links: [],
  signatures: [],
  leftovers: { thumbnailPages: [], pagePieces: [] },
  unsupported: [],
};

type DetailsWindow = Window & { calls: Array<{ command: string; args: any }>; picked: unknown[]; clipboard: string[] };

const platforms = [
  { name: "macOS", platform: "MacIntel", primary: "Meta", alt: "Alt", labels: { open: "⌘O", run: "⌘↵", previous: "⌥↑", next: "⌥↓", copy: "⇧⌘C" } },
  { name: "Windows", platform: "Win32", primary: "Control", alt: "Alt", labels: { open: "Ctrl+O", run: "Ctrl+Enter", previous: "Alt+↑", next: "Alt+↓", copy: "Ctrl+Shift+C" } },
] as const;

const install = (page: Page, platform: string) => page.addInitScript(({ platform, report, SIZE_LIMIT, DAMAGED, bigPath, damagedPath }) => {
  const w = window as unknown as DetailsWindow;
  const callbacks = new Map<number, (event: unknown) => void>();
  let nextId = 1;
  w.calls = [];
  w.picked = [];
  w.clipboard = [];
  Object.defineProperty(window.navigator, "platform", { configurable: true, value: platform });
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text: string) => { w.clipboard.push(text); } } });
  const outcome = (inputPath: string, rest: object) => ({ inputPath, outputPaths: [], detail: "", state: "succeeded", failure: null, reason: null, warnings: [], ...rest });
  const failedAlone = (inputPath: string) => inputPath === bigPath
    ? outcome(inputPath, { state: "failed", failure: { kind: "limitExceeded", details: { limit: "fileSize" }, message: SIZE_LIMIT, pointer: null } })
    : inputPath === damagedPath
    ? outcome(inputPath, { state: "failed", failure: { kind: "processing", message: DAMAGED, pointer: null } })
    : null;
  (window as any).__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
    invoke: async (command: string, args: any) => {
      w.calls.push({ command, args });
      if (command === "plugin:event|listen") return nextId++;
      if (command === "plugin:dialog|open") return w.picked.shift() ?? null;
      if (command === "inspect_intake_paths") return args.paths.map((path: string) => ({ path, kind: "file" }));
      if (command === "get_output_settings") return { mode: "customFolder", folder: "/Users/test/Documents/Toolbox" };
      if (command === "check_output_folder") return "ready";
      if (command === "plugin:path|resolve_directory") return "/Users/test";
      if (command === "inspect_pdf_privacy") {
        return args.request.paths.map((path: string) => failedAlone(path) ?? outcome(path, { detail: JSON.stringify(report) }));
      }
      if (command === "read_pdf_properties") {
        return args.request.paths.map((path: string) => failedAlone(path) ?? outcome(path, { detail: JSON.stringify({ ...report.documentInformation, title: "Draft" }) }));
      }
      if (command === "edit_pdf_properties") {
        return args.request.paths.map((path: string) => failedAlone(path) ?? outcome(path, { outputPaths: [path.replace(/\.pdf$/, "-edited.pdf")], detail: "Title set." }));
      }
      return null;
    },
    transformCallback: (callback: (event: unknown) => void) => {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
  };
  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
}, { platform, report, SIZE_LIMIT, DAMAGED, bigPath, damagedPath });

const calls = (page: Page, command: string) =>
  page.evaluate((command) => (window as unknown as DetailsWindow).calls.filter((call) => call.command === command).map((call) => call.args), command);

const queuePick = (page: Page, picked: string[]) => page.evaluate((picked) => { (window as unknown as DetailsWindow).picked.push(picked); }, picked);

const open = async (page: Page, title: string) => {
  await page.goto("/");
  await openToolBySearch(page, title);
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
};

const region = (page: Page, name: string) => page.getByRole("region", { name, exact: true });

for (const { name, platform, primary, alt, labels } of platforms) {
  test.describe(`on ${name}`, () => {
    test.beforeEach(async ({ page }) => install(page, platform));

    test("Privacy Check adds files and runs by keyboard, shows over-limit and damaged files beside reports, and moves between reports", async ({ page }) => {
      await open(page, "Privacy Check");
      await expect(page.locator('[data-shortcut="open"] [data-shortcut-hint="open"]').first()).toHaveText(labels.open);

      await queuePick(page, paths);
      await page.keyboard.press(`${primary}+O`);
      await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(4);
      const run = page.locator('[data-shortcut="run"]');
      await expect(run.locator('[data-shortcut-hint="run"]')).toHaveText(labels.run);
      await page.keyboard.press(`${primary}+Enter`);

      await expect.poll(async () => (await calls(page, "inspect_pdf_privacy")).length).toBe(1);
      expect((await calls(page, "inspect_pdf_privacy"))[0].request.paths).toEqual(paths);
      await expect(page.getByRole("status").filter({ hasText: "Done." })).toHaveText("Done. 2 reports, 2 failedNothing was changed in any PDF.");
      await expect(region(page, "big.pdf").getByRole("alert")).toHaveText(`Failed: ${SIZE_LIMIT}`);
      await expect(region(page, "damaged.pdf").getByRole("alert")).toHaveText(`Failed: ${DAMAGED}`);
      for (const ok of ["clean.pdf", "deck.pdf"]) {
        await expect(region(page, ok).getByRole("list", { name: "Report categories" })).toBeVisible();
        await expect(region(page, ok).getByRole("alert")).toHaveCount(0);
      }

      await expect(page.locator(".privacy-report-keys")).toHaveText(`${labels.previous}${labels.next}from the report moves to the previous or next report`);
      const copy = region(page, "clean.pdf").getByRole("button", { name: /^Copy/ });
      await expect(copy.locator("kbd")).toHaveText(labels.copy);

      await copy.focus();
      await page.keyboard.press(`${alt}+ArrowDown`);
      await expect(region(page, "big.pdf")).toBeFocused();
      await page.keyboard.press(`${alt}+ArrowDown`);
      await expect(region(page, "damaged.pdf")).toBeFocused();
      await page.keyboard.press(`${alt}+ArrowDown`);
      await expect(region(page, "deck.pdf")).toBeFocused();
      await page.keyboard.press(`${alt}+ArrowDown`);
      await expect(region(page, "deck.pdf")).toBeFocused();
      await page.keyboard.press(`${primary}+Shift+C`);
      await expect.poll(() => page.evaluate(() => (window as unknown as DetailsWindow).clipboard)).toEqual([expect.stringContaining("deck.pdf")]);
      await page.keyboard.press(`${alt}+ArrowUp`);
      await expect(region(page, "damaged.pdf")).toBeFocused();
      await page.keyboard.press(`${primary}+Shift+C`);
      expect(await page.evaluate(() => (window as unknown as DetailsWindow).clipboard)).toHaveLength(1);

      await page.getByRole("button", { name: "Copy all" }).focus();
      await page.keyboard.press(`${alt}+ArrowDown`);
      await expect(region(page, "clean.pdf")).toBeFocused();
      await page.getByRole("button", { name: "Copy all" }).focus();
      await page.keyboard.press(`${alt}+ArrowUp`);
      await expect(region(page, "deck.pdf")).toBeFocused();
    });

    test("Edit Properties adds files by keyboard, runs from a text field, and shows over-limit and damaged rows beside saved ones", async ({ page }) => {
      await open(page, "Edit Properties");
      await queuePick(page, paths);
      await page.keyboard.press(`${primary}+O`);
      await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(4);
      expect(await calls(page, "plugin:dialog|open")).toHaveLength(1);

      const title = page.getByRole("group", { name: "Title", exact: true });
      await title.getByRole("radio", { name: "Set", exact: true }).click();
      const value = title.getByRole("textbox", { name: "New Title" });
      await value.fill("Final");
      await page.keyboard.press(`${primary}+O`);
      expect(await calls(page, "plugin:dialog|open")).toHaveLength(1);
      await expect(page.locator('[data-shortcut="run"] [data-shortcut-hint="run"]')).toHaveText(labels.run);
      await value.press(`${primary}+Enter`);

      await expect.poll(async () => (await calls(page, "edit_pdf_properties")).length).toBe(1);
      expect((await calls(page, "edit_pdf_properties"))[0].request.paths).toEqual(paths);
      const results = page.getByRole("list", { name: "Results" }).getByRole("listitem");
      await expect(results).toHaveCount(4);
      await expect(results.filter({ hasText: "big.pdf" })).toHaveAttribute("data-state", "failed");
      await expect(results.filter({ hasText: "big.pdf" })).toContainText(SIZE_LIMIT);
      await expect(results.filter({ hasText: "damaged.pdf" })).toHaveAttribute("data-state", "failed");
      await expect(results.filter({ hasText: "damaged.pdf" })).toContainText(DAMAGED);
      await expect(results.filter({ hasText: "clean.pdf" })).toHaveAttribute("data-state", "succeeded");
      await expect(results.filter({ hasText: "deck.pdf" })).toHaveAttribute("data-state", "succeeded");
    });

    test("an IME composition in a text field does not start the run", async ({ page }) => {
      await open(page, "Edit Properties");
      await queuePick(page, [deckPath]);
      await page.keyboard.press(`${primary}+O`);
      await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(1);
      const title = page.getByRole("group", { name: "Title", exact: true });
      await title.getByRole("radio", { name: "Set", exact: true }).click();
      const value = title.getByRole("textbox", { name: "New Title" });
      await value.fill("Final");
      await value.evaluate((input, primary) => input.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter", bubbles: true, cancelable: true, isComposing: true, metaKey: primary === "Meta", ctrlKey: primary === "Control",
      })), primary);
      await value.press(`${primary}+Enter`);
      await expect.poll(async () => (await calls(page, "edit_pdf_properties")).length).toBe(1);
      await expect(page.getByRole("list", { name: "Results" }).getByRole("listitem")).toHaveCount(1);
      expect(await calls(page, "edit_pdf_properties")).toHaveLength(1);
    });
  });
}
