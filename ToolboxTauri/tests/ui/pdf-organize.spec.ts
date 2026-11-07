import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from './open-tool';

// Organize PDF opens exactly one local PDF. This harness fakes the native scene commands, the file dialog
// and the shell's drop events, so every path below stays on the page and no request leaves the machine.
const svg = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/><text x="60" y="85" font-size="24">Local page</text></svg>');
const lockedMessage = 'This PDF is password-protected. Remove its password with File Security first, then open the unlocked copy.';
const damagedMessage = 'Toolbox can’t read this file. It may be damaged, or it isn’t a PDF that Toolbox supports. Nothing was changed.';

let remote: string[] = [];

test.beforeEach(async ({ page }) => {
  remote = [];
  page.on('request', (request) => {
    if (!/^(http:\/\/(127\.0\.0\.1|localhost)|data:|blob:)/.test(request.url())) remote.push(request.url());
  });
  await page.addInitScript(({ svg, lockedMessage, damagedMessage }) => {
    const w = window as any;
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let nextId = 1;
    w.calls = [];
    w.dialogNext = '/local/report.pdf';
    w.__drop = (paths: string[]) => {
      for (const [id, listener] of listeners) {
        if (listener.event === 'toolbox://shell-event') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: { kind: 'dropped-files', paths } });
      }
    };
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: (callback: (event: unknown) => void) => { const id = nextId++; callbacks.set(id, callback); return id; },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:event|listen') {
          const id = nextId++;
          listeners.set(id, { event: args.event, handler: args.handler });
          return id;
        }
        if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return null; }
        if (command === 'inspect_intake_paths') return args.paths.map((path: string) => ({ path, kind: 'file' }));
        if (command === 'plugin:dialog|open') return w.dialogNext;
        if (command === 'inspect_pdf_scene') {
          const path: string = args.request?.path ?? args.path;
          if (path.endsWith('locked.pdf')) throw { kind: 'invalidInput', message: lockedMessage, pointer: { kind: 'tool', toolId: 'lock-unlock' } };
          if (path.endsWith('damaged.pdf')) throw { kind: 'invalidInput', message: damagedMessage, pointer: null };
          return { path, pages: [0, 1, 2, 3, 4].map((index) => ({ index, width: 612, height: 792, preview: null })) };
        }
        if (command === 'inspect_pdf_scene_page') {
          const pageIndex = args.request.pageIndex;
          return { index: pageIndex, width: 612, height: 792, preview: svg, textRuns: [] };
        }
        if (command === 'preview_pdf_scene_pages') return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        if (command === 'export_pdf_scene') return [{ inputPath: args.request.path, outputPaths: ['/local/report-organized.pdf'], state: 'succeeded', detail: 'PDF scene exported', failure: null, reason: null, warnings: [] }];
        return null;
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg, lockedMessage, damagedMessage });
});

test.afterEach(() => {
  expect(remote).toEqual([]);
});

async function openOrganize(page: Page) {
  await page.goto('/');
  await openToolBySearch(page, 'Organize PDF');
  await expect(page.getByRole('heading', { name: 'Open a PDF to organize its pages' })).toBeVisible();
}
async function drop(page: Page, paths: string[]) {
  await expect.poll(() => page.evaluate(() => (window as any).calls.some(({ command, args }: any) =>
    command === 'plugin:event|listen' && args.event === 'toolbox://shell-event'))).toBe(true);
  await page.evaluate((paths) => (window as any).__drop(paths), paths);
}
const grid = (page: Page, name = 'report.pdf') => page.getByRole('listbox', { name: `Pages of ${name}, in order` });
const focusPreview = (page: Page) => page.getByRole('complementary', { name: 'Preview of the focused page' });

test('the empty state offers one-PDF opening, drop guidance and Merge', async ({ page }) => {
  await openOrganize(page);
  await expect(page.getByText('Or drop one PDF anywhere in this window.')).toBeVisible();
  await expect(page.getByText(/Organize works on one PDF at a time\. To combine several PDFs, use Merge first/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add blank page' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Open Merge', exact: true }).click();
  await expect(page.getByRole('toolbar', { name: 'PDF Editor tools' }).getByRole('button', { name: 'Merge PDF', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('the dialog opens one PDF into the page grid with position and original-page labels', async ({ page }) => {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(grid(page)).toBeVisible();
  await expect(page.getByRole('heading', { name: 'report.pdf', level: 2 })).toBeVisible();
  await expect(page.getByText('5 pages', { exact: true })).toBeVisible();
  const options = grid(page).getByRole('option');
  await expect(options).toHaveCount(5);
  await expect(options.first()).toHaveAccessibleName('Position 1, original page 1');
  await expect(options.first()).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByText('was page')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: 'Showing 5 pages. Previews load as you scroll.' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add blank page' })).toHaveCount(0);
  const open = page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'plugin:dialog|open').at(-1).args.options);
  expect(await open).toMatchObject({ multiple: false });
});

test('the focused page preview follows clicks and arrow keys', async ({ page }) => {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await grid(page).getByRole('option', { name: 'Position 3, original page 3' }).click();
  const preview = focusPreview(page);
  await expect(preview.getByText('Page 3')).toBeVisible();
  await expect(preview.getByText('3 of 5')).toBeVisible();
  await expect(preview.getByText('As in the original')).toBeVisible();
  await expect(preview.getByRole('img', { name: 'Preview of page 3' })).toBeVisible();
  await page.keyboard.press('ArrowRight');
  await expect(grid(page).getByRole('option', { name: 'Position 4, original page 4' })).toBeFocused();
  await expect(preview.getByText('4 of 5')).toBeVisible();
  await page.keyboard.press('End');
  await expect(grid(page).getByRole('option', { name: 'Position 5, original page 5' })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(grid(page).getByRole('option', { name: 'Position 1, original page 1' })).toHaveAttribute('aria-selected', 'true');
});

test('the thumbnail size control changes the grid density', async ({ page }) => {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  const sizes = page.getByRole('radiogroup', { name: 'Thumbnails' });
  await expect(grid(page)).toHaveAttribute('data-size', 'medium');
  const medium = (await grid(page).getByRole('option').first().boundingBox())!.width;
  await sizes.getByRole('radio', { name: 'Small' }).click();
  await expect(grid(page)).toHaveAttribute('data-size', 'small');
  const small = (await grid(page).getByRole('option').first().boundingBox())!.width;
  await sizes.getByRole('radio', { name: 'Large' }).click();
  await expect(grid(page)).toHaveAttribute('data-size', 'large');
  const large = (await grid(page).getByRole('option').first().boundingBox())!.width;
  expect(small).toBeLessThan(medium);
  expect(large).toBeGreaterThan(medium);
});

test('switching between Organize and page view keeps the edited scene', async ({ page }) => {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(grid(page)).toBeVisible();
  const inspectionCount = () => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_scene').length);
  const openedWith = await inspectionCount();
  await page.getByRole('button', { name: 'Open in page view', exact: true }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await page.getByRole('button', { name: 'Rotate selected pages', exact: true }).click();
  await page.getByRole('button', { name: 'Move page later', exact: true }).click();
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Organize', exact: true }).click();
  await expect(grid(page).getByRole('option').nth(0)).toHaveAccessibleName('Position 1, original page 2');
  await expect(grid(page).getByRole('option').nth(1)).toHaveAccessibleName('Position 2, original page 1');
  await expect(grid(page).getByRole('option').nth(1)).toContainText('was page 1');
  await expect(focusPreview(page).getByText('Rotated 90° right')).toBeVisible();
  await expect(page.getByText('5 pages. Changes not exported')).toBeVisible();
  expect(await inspectionCount()).toBe(openedWith);
});

test('a dropped PDF opens, and a dropped non-PDF is refused with a message', async ({ page }) => {
  await openOrganize(page);
  await drop(page, ['/local/notes.txt']);
  await expect(page.getByText('PDF Editor only takes .pdf files.')).toBeVisible();
  await expect(grid(page)).toHaveCount(0);
  await drop(page, ['/local/report.pdf']);
  await expect(grid(page)).toBeVisible();
  await page.evaluate(() => (window as any).dialogNext = '/local/notes.txt');
  await page.getByRole('button', { name: /Choose files|Replace/ }).first().click();
  await expect(page.getByText('PDF Editor only takes .pdf files.')).toBeVisible();
  await expect(grid(page)).toBeVisible();
});

test('several dropped PDFs ask which one to open and point to Merge', async ({ page }) => {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(grid(page)).toBeVisible();

  await drop(page, ['/local/a.pdf', '/local/b.pdf', '/local/c.pdf']);
  const dialog = page.getByRole('dialog', { name: 'Organize works on one PDF at a time' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('You dropped 3 PDFs. Choose the one to open.');
  await expect(dialog.getByRole('radio')).toHaveCount(3);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(grid(page)).toBeVisible();

  await drop(page, ['/local/a.pdf', '/local/b.pdf', '/local/c.pdf']);
  await dialog.getByRole('radio', { name: 'b.pdf' }).click();
  await dialog.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(grid(page, 'b.pdf')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'b.pdf', level: 2 })).toBeVisible();

  await drop(page, ['/local/a.pdf', '/local/c.pdf']);
  await expect(dialog).toContainText('You dropped 2 PDFs.');
  await dialog.getByRole('button', { name: 'Open Merge', exact: true }).click();
  await expect(page.getByRole('toolbar', { name: 'PDF Editor tools' }).getByRole('button', { name: 'Merge PDF', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText('a.pdf').first()).toBeVisible();
  await expect(page.getByText('c.pdf').first()).toBeVisible();
});

test('a password-protected PDF points to File Security and writes nothing', async ({ page }) => {
  await openOrganize(page);
  await page.evaluate(() => (window as any).dialogNext = '/local/locked.pdf');
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'locked.pdf is password-protected' })).toBeVisible();
  await expect(page.getByText('Remove its password with File Security first, then open the unlocked copy here.')).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'Couldn’t open locked.pdf. It’s password-protected.' })).toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'export_pdf_scene'))).toBe(false);
  await page.getByRole('button', { name: 'Open File Security', exact: true }).click();
  await expect(page.getByRole('heading', { name: /File Security/ }).first()).toBeVisible();
});

test('a damaged PDF explains the problem and offers another file', async ({ page }) => {
  await openOrganize(page);
  await page.evaluate(() => (window as any).dialogNext = '/local/damaged.pdf');
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Toolbox can’t read damaged.pdf' })).toBeVisible();
  await expect(page.getByText(/The file may be damaged, or it isn’t a PDF that Toolbox supports\. Nothing was changed\./)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open File Security' })).toHaveCount(0);
  await page.evaluate(() => (window as any).dialogNext = '/local/report.pdf');
  await page.getByRole('button', { name: 'Open another PDF', exact: true }).click();
  await expect(grid(page)).toBeVisible();
});

for (const theme of ['light', 'dark']) test(`the Organize page grid renders in ${theme}`, async ({ page }, testInfo) => {
  await page.addInitScript((theme) => localStorage.setItem('toolbox-theme', theme), theme);
  await openOrganize(page);
  await page.screenshot({ path: testInfo.outputPath(`organize-empty-${theme}.png`) });
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(grid(page).getByRole('option')).toHaveCount(5);
  await expect(focusPreview(page).getByRole('img', { name: 'Preview of page 1' })).toBeVisible();
  await expect(page.locator('body')).toHaveAttribute('data-theme', theme);
  await page.screenshot({ path: testInfo.outputPath(`organize-grid-${theme}.png`) });
});
