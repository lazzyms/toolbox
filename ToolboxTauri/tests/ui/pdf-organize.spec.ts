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
  await expect(options.first()).toHaveAttribute('aria-selected', 'false');
  await expect(options.first()).toHaveAttribute('tabindex', '0');
  await expect(page.getByRole('toolbar', { name: 'Page actions, No pages selected' }).getByRole('status')).toHaveText('No pages selected');
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
  await expect(grid(page).getByRole('option', { name: 'Position 1, original page 1' })).toBeFocused();
  await expect(preview.getByText('1 of 5')).toBeVisible();
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
  await expect(grid(page).getByRole('option').nth(1)).toHaveAccessibleName('Position 2, original page 1, rotated 90° right');
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

// Selection. The grid is one tab stop: arrow keys move focus, and selection changes only on purpose.
const option = (page: Page, position: number) => grid(page).getByRole('option').nth(position - 1);
const selectedPositions = (page: Page) => grid(page).getByRole('option').evaluateAll((options) =>
  options.flatMap((option, index) => option.getAttribute('aria-selected') === 'true' ? [index + 1] : []));
const count = (page: Page) => page.getByRole('toolbar', { name: /^Page actions, / }).getByRole('status');
const undoButton = (page: Page) => page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Undo', exact: true });
const selectGroup = (page: Page) => page.getByRole('group', { name: 'Select', exact: true });
async function openReport(page: Page) {
  await openOrganize(page);
  await page.getByRole('button', { name: 'Open PDF', exact: true }).click();
  await expect(grid(page).getByRole('option')).toHaveCount(5);
}
const setPlatform = (page: Page, value: 'MacIntel' | 'Win32') => page.addInitScript((value) => {
  Object.defineProperty(window.navigator, 'platform', { configurable: true, value });
}, value);

for (const [platform, modifier] of [['MacIntel', 'Meta'], ['Win32', 'Control']] as const) {
  test(`pointer selection on ${platform}: click selects, ${modifier}-click toggles, Shift-click selects a run`, async ({ page }) => {
    await setPlatform(page, platform);
    // Chromium on a macOS host turns a real Ctrl-click into a context-menu click, so the Windows case dispatches the click it would get on Windows.
    const toggleClick = (position: number) => platform === 'Win32'
      ? option(page, position).dispatchEvent('click', { ctrlKey: true, bubbles: true })
      : option(page, position).click({ modifiers: [modifier] });
    await openReport(page);
    await expect(grid(page)).toHaveAttribute('aria-multiselectable', 'true');
    await option(page, 2).click();
    expect(await selectedPositions(page)).toEqual([2]);
    await expect(option(page, 2)).toHaveAccessibleName('Position 2, original page 2, selected');
    await expect(count(page)).toHaveText('1 page selected');
    await expect(page.getByRole('toolbar', { name: 'Page actions, 1 page selected' })).toBeVisible();

    await toggleClick(4);
    expect(await selectedPositions(page)).toEqual([2, 4]);

    // Shift-click runs from the last page clicked.
    await option(page, 1).click({ modifiers: ['Shift'] });
    expect(await selectedPositions(page)).toEqual([1, 2, 3, 4]);
    await expect(count(page)).toHaveText('4 pages selected');

    await toggleClick(2);
    expect(await selectedPositions(page)).toEqual([1, 3, 4]);

    await option(page, 5).click();
    expect(await selectedPositions(page)).toEqual([5]);
    await expect(undoButton(page)).toBeDisabled();
  });

  test(`keyboard selection on ${platform}: one tab stop, arrows move focus, Space toggles, Shift extends, ${modifier}+A and Escape`, async ({ page }) => {
    await setPlatform(page, platform);
    await openReport(page);
    await expect(grid(page).locator('[tabindex="0"]')).toHaveCount(1);
    await option(page, 1).focus();
    await page.keyboard.press('ArrowRight');
    await expect(option(page, 2)).toBeFocused();
    expect(await selectedPositions(page)).toEqual([]);

    await page.keyboard.press('Space');
    expect(await selectedPositions(page)).toEqual([2]);
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Space');
    expect(await selectedPositions(page)).toEqual([2, 4]);
    await page.keyboard.press('Space');
    expect(await selectedPositions(page)).toEqual([2]);
    await expect(grid(page).locator('[tabindex="0"]')).toHaveCount(1);
    await expect(option(page, 4)).toHaveAttribute('tabindex', '0');

    await page.keyboard.press('Shift+ArrowRight');
    await expect(option(page, 5)).toBeFocused();
    expect(await selectedPositions(page)).toEqual([4, 5]);
    await page.keyboard.press('Shift+Home');
    expect(await selectedPositions(page)).toEqual([1, 2, 3, 4]);
    await expect(option(page, 1)).toBeFocused();

    await page.keyboard.press('Escape');
    expect(await selectedPositions(page)).toEqual([]);
    await expect(count(page)).toHaveText('No pages selected');
    await expect(grid(page)).toBeVisible();

    await page.keyboard.press(`${modifier}+A`);
    expect(await selectedPositions(page)).toEqual([1, 2, 3, 4, 5]);
    await expect(count(page)).toHaveText('5 pages selected');
    await expect(undoButton(page)).toBeDisabled();
  });
}

test('Up and Down move focus by a row, and Enter opens the focused page in page view', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openReport(page);
  await page.getByRole('radiogroup', { name: 'Thumbnails' }).getByRole('radio', { name: 'Large' }).click();
  const columns = await grid(page).getByRole('option').evaluateAll((options) =>
    options.filter((option) => (option as HTMLElement).offsetTop === (options[0] as HTMLElement).offsetTop).length);
  expect(columns).toBeGreaterThan(1);
  expect(columns).toBeLessThan(5);
  await option(page, 1).focus();
  await page.keyboard.press('ArrowDown');
  await expect(option(page, 1 + columns)).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(option(page, 1)).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(option(page, 1)).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('All, Odd, Even and None select by current position and are disabled while exporting', async ({ page }) => {
  await setPlatform(page, 'MacIntel');
  await openReport(page);
  await expect(selectGroup(page).getByRole('button', { name: 'All' })).toContainText('⌘A');
  await expect(selectGroup(page).getByRole('button', { name: 'None' })).toContainText('Esc');
  await selectGroup(page).getByRole('button', { name: 'Odd', exact: true }).click();
  expect(await selectedPositions(page)).toEqual([1, 3, 5]);
  await expect(count(page)).toHaveText('3 pages selected');
  await selectGroup(page).getByRole('button', { name: 'Even', exact: true }).click();
  expect(await selectedPositions(page)).toEqual([2, 4]);
  await selectGroup(page).getByRole('button', { name: /^All/ }).click();
  expect(await selectedPositions(page)).toEqual([1, 2, 3, 4, 5]);
  await selectGroup(page).getByRole('button', { name: /^None/ }).click();
  expect(await selectedPositions(page)).toEqual([]);
  await expect(undoButton(page)).toBeDisabled();

  // Odd and even follow the current order, not the original page numbers.
  await page.getByRole('button', { name: 'Open in page view', exact: true }).click();
  await page.getByRole('button', { name: 'Move page later', exact: true }).click();
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Organize', exact: true }).click();
  await selectGroup(page).getByRole('button', { name: 'Odd', exact: true }).click();
  await expect(option(page, 1)).toHaveAccessibleName('Position 1, original page 2, selected');
  await expect(option(page, 2)).toHaveAccessibleName('Position 2, original page 1');

  await page.evaluate(() => {
    const w = window as any;
    const invoke = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = (command: string, args: any) => command === 'export_pdf_scene' ? new Promise(() => {}) : invoke(command, args);
  });
  await page.getByRole('button', { name: 'Export PDF' }).click();
  for (const name of [/^All/, 'Odd', 'Even', /^None/]) await expect(selectGroup(page).getByRole('button', { name })).toBeDisabled();
  await expect(page.getByLabel('Pages', { exact: true })).toBeDisabled();
});

test('Windows shows Ctrl+A on the All button', async ({ page }) => {
  await setPlatform(page, 'Win32');
  await openReport(page);
  await expect(selectGroup(page).getByRole('button', { name: /^All/ })).toContainText('Ctrl+A');
});

test('a typed page range selects those positions, and a bad range explains itself and keeps the selection', async ({ page }) => {
  await openReport(page);
  const range = page.getByLabel('Pages', { exact: true });
  await expect(range).toHaveAttribute('placeholder', 'For example 1-3, 7');
  await range.fill('1-2, 4');
  await range.press('Enter');
  expect(await selectedPositions(page)).toEqual([1, 2, 4]);
  await expect(count(page)).toHaveText('3 pages selected');

  for (const [value, message] of [
    ['1-3, 250', 'Page 250 isn\'t in this PDF. It has 5 pages, so use numbers from 1 to 5.'],
    ['4-2', 'Write 4-2 from low to high, like 2-4.'],
    ['0', 'Page numbers start at 1.'],
    ['two', 'Invalid page selection: two.'],
  ]) {
    await range.fill(value);
    await range.press('Enter');
    await expect(page.getByRole('alert')).toHaveText(message);
    await expect(range).toHaveAttribute('aria-invalid', 'true');
    await expect(range).toHaveAccessibleDescription(message);
    expect(await selectedPositions(page)).toEqual([1, 2, 4]);
  }
  await range.fill('3');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(range).not.toHaveAttribute('aria-invalid', 'true');
  await range.press('Enter');
  expect(await selectedPositions(page)).toEqual([3]);
  await expect(undoButton(page)).toBeDisabled();
});

test('the grid describes its keys, and each page names its orientation and selected state', async ({ page }) => {
  await openReport(page);
  await expect(grid(page)).toHaveAccessibleDescription(/Arrow keys move focus\. Space selects\. Shift with an arrow key extends the selection\./);
  await option(page, 3).click();
  await page.getByRole('button', { name: 'Open in page view', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate selected pages', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate selected pages', exact: true }).click();
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Organize', exact: true }).click();
  await expect(option(page, 3)).toHaveAccessibleName('Position 3, original page 3, rotated 180°, selected');
  await expect(option(page, 1)).toHaveAccessibleName('Position 1, original page 1');
});

test('a later action sends exactly the selected pages to export', async ({ page }) => {
  await setPlatform(page, 'MacIntel');
  await openReport(page);
  await option(page, 2).click();
  await option(page, 4).click({ modifiers: ['Meta'] });
  await expect(count(page)).toHaveText('2 pages selected');
  await expect(undoButton(page)).toBeDisabled();
  await page.getByRole('button', { name: 'Open in page view', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate selected pages', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Export PDF' })).toBeEnabled();
  await page.getByRole('button', { name: 'Export PDF' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'export_pdf_scene'))).toBe(true);
  const scene = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').at(-1).args.request.scene);
  expect(scene.pages.map((item: any) => item.rotation)).toEqual([0, 90, 0, 90, 0]);
});

// Reordering. Selected pages move as one block, in their order, by drag, keys, Move to and the Page order menu.
const order = (page: Page) => grid(page).getByRole('option').evaluateAll((options) =>
  options.map((option) => Number(/original page (\d+)/.exec(option.getAttribute('aria-label') ?? '')?.[1])));
const liveStatus = (page: Page) => page.locator('.scene-organize-status').getByRole('status');
const pageActions = (page: Page) => page.getByRole('toolbar', { name: /^Page actions, / });
async function exportedOrder(page: Page) {
  await page.getByRole('button', { name: 'Export PDF' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'export_pdf_scene'))).toBe(true);
  const scene = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').at(-1).args.request.scene);
  return scene.pages.map((item: any) => item.sourceIndex + 1);
}
async function selectRun(page: Page, first: number, last: number) {
  await option(page, first).click();
  if (last !== first) await option(page, last).click({ modifiers: ['Shift'] });
}
async function center(page: Page, position: number, side: 'left' | 'right' | 'middle' = 'middle') {
  const box = (await option(page, position).boundingBox())!;
  return { x: box.x + (side === 'left' ? box.width * 0.2 : side === 'right' ? box.width * 0.8 : box.width / 2), y: box.y + box.height / 2 };
}

test('dragging selected pages moves them together, shows the count and the drop position, and exports that order', async ({ page }) => {
  await openReport(page);
  await option(page, 1).click();
  await option(page, 3).click({ modifiers: ['Meta'] });
  const start = await center(page, 1);
  const end = await center(page, 5, 'right');
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 8 });
  await expect(liveStatus(page)).toHaveText('Moving 2 pages. Drop to place them at the end. Press Esc to cancel.');
  await expect(page.locator('.scene-organize-ghost')).toContainText('2');
  await expect(page.locator('.scene-organize-insertion')).toHaveAttribute('data-side', 'after');
  await expect(grid(page).locator('[data-moving]')).toHaveCount(2);
  await page.mouse.up();
  expect(await order(page)).toEqual([2, 4, 5, 1, 3]);
  await expect(liveStatus(page)).toHaveText('Pages 1 and 3 moved to position 4');
  // The drop does not change the selection, and it is one undo step.
  expect(await selectedPositions(page)).toEqual([4, 5]);
  await expect(page.locator('.scene-organize-ghost')).toHaveCount(0);
  await undoButton(page).click();
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
  await expect(undoButton(page)).toBeDisabled();
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  expect(await exportedOrder(page)).toEqual([2, 4, 5, 1, 3]);
});

test('dragging an unselected page moves only that page, and a drop between pages names the position', async ({ page }) => {
  await openReport(page);
  await option(page, 1).click();
  const start = await center(page, 4);
  const end = await center(page, 2, 'left');
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 8 });
  await expect(liveStatus(page)).toHaveText('Moving 1 page. Drop to place it before position 2. Press Esc to cancel.');
  await page.mouse.up();
  expect(await order(page)).toEqual([1, 4, 2, 3, 5]);
  await expect(liveStatus(page)).toHaveText('Page 4 moved to position 2');
  expect(await selectedPositions(page)).toEqual([2]);
});

test('Escape cancels a drag before release: nothing moves and there is nothing to undo', async ({ page }) => {
  await openReport(page);
  await selectRun(page, 1, 2);
  const start = await center(page, 2);
  const end = await center(page, 5, 'right');
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 8 });
  await page.keyboard.press('Escape');
  await expect(liveStatus(page)).toHaveText('Move cancelled. Nothing moved');
  await page.mouse.up();
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
  expect(await selectedPositions(page)).toEqual([1, 2]);
  await expect(undoButton(page)).toBeDisabled();
  await expect(grid(page)).toBeVisible();
});

for (const [platform, modifier, startKey, endKey] of [['MacIntel', 'Meta', 'Meta+ArrowUp', 'Meta+ArrowDown'], ['Win32', 'Control', 'Control+Home', 'Control+End']] as const) {
  test(`keyboard moves on ${platform}: ${modifier} with arrows steps the block, ${startKey} and ${endKey} send it to an end`, async ({ page }) => {
    await setPlatform(page, platform);
    await openReport(page);
    await selectRun(page, 2, 3);
    await page.keyboard.press(`${modifier}+ArrowRight`);
    expect(await order(page)).toEqual([1, 4, 2, 3, 5]);
    await expect(liveStatus(page)).toHaveText('Pages 2 and 3 moved to position 3');
    // Focus and selection follow the moved pages.
    await expect(grid(page).getByRole('option', { name: 'Position 4, original page 3, selected' })).toBeFocused();
    await page.keyboard.press(endKey);
    expect(await order(page)).toEqual([1, 4, 5, 2, 3]);
    await page.keyboard.press(`${modifier}+ArrowRight`);
    await expect(liveStatus(page)).toHaveText('Pages 4 and 5 are already last');
    await page.keyboard.press(startKey);
    expect(await order(page)).toEqual([2, 3, 1, 4, 5]);
    await expect(liveStatus(page)).toHaveText('Pages 4 and 5 moved to position 1');
    await page.keyboard.press(`${modifier}+ArrowLeft`);
    await expect(liveStatus(page)).toHaveText('Pages 1 and 2 are already first');
    await expect(pageActions(page).getByRole('button', { name: `Move earlier (${platform === 'Win32' ? 'Ctrl' : '⌘'}+←)` })).toBeEnabled();
    // Three moves, three undo steps.
    for (let step = 0; step < 3; step += 1) await undoButton(page).click();
    expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
    await expect(undoButton(page)).toBeDisabled();
  });
}

test('scattered pages gather into one block when stepped, and toolbar Move buttons need a selection', async ({ page }) => {
  await setPlatform(page, 'MacIntel');
  await openReport(page);
  await expect(pageActions(page).getByRole('button', { name: 'Move later (⌘+→)' })).toBeDisabled();
  await expect(pageActions(page).getByRole('button', { name: 'Move to…' })).toBeDisabled();
  await option(page, 1).click();
  await option(page, 4).click({ modifiers: ['Meta'] });
  await pageActions(page).getByRole('button', { name: 'Move later (⌘+→)' }).click();
  // Later gathers the block after the last selected page; earlier then steps the whole block back one.
  expect(await order(page)).toEqual([2, 3, 5, 1, 4]);
  await expect(liveStatus(page)).toHaveText('Pages 1 and 4 moved to position 4');
  await pageActions(page).getByRole('button', { name: 'Move earlier (⌘+←)' }).click();
  expect(await order(page)).toEqual([2, 3, 1, 4, 5]);
  await expect(pageActions(page).getByRole('button', { name: 'Move earlier (⌘+←)' })).toHaveAttribute('aria-keyshortcuts', 'Meta+ArrowLeft');
});

test('M opens Move to for the selection, a bad position explains itself, and Enter moves the block', async ({ page }) => {
  await openReport(page);
  await selectRun(page, 4, 5);
  await page.keyboard.press('m');
  const popup = page.getByRole('dialog', { name: 'Move 2 pages to a position' });
  await expect(popup).toBeVisible();
  const field = popup.getByRole('textbox', { name: 'Move pages 4 and 5 to position' });
  await expect(field).toBeFocused();
  await expect(popup).toContainText('of 5');
  await expect(popup).toContainText('The pages stay together in the same order. Position 1 puts them first.');
  await field.fill('9');
  await field.press('Enter');
  await expect(popup.getByRole('alert')).toHaveText('Type a position from 1 to 5.');
  await expect(field).toHaveAttribute('aria-invalid', 'true');
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
  await field.fill('2');
  await field.press('Enter');
  await expect(popup).toHaveCount(0);
  expect(await order(page)).toEqual([1, 4, 5, 2, 3]);
  await expect(liveStatus(page)).toHaveText('Pages 4 and 5 moved to position 2');
  // Focus returns to the grid, on the same page.
  await expect(grid(page).getByRole('option', { name: 'Position 3, original page 5, selected' })).toBeFocused();
  // Start and End in the popup send the block to either end.
  await pageActions(page).getByRole('button', { name: 'Move to…' }).click();
  await page.getByRole('dialog', { name: 'Move 2 pages to a position' }).getByRole('button', { name: /^End/ }).click();
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
  await expect(liveStatus(page)).toHaveText('Pages 2 and 3 moved to position 4');
  // Escape closes the popup without moving anything, and focus goes back to the grid.
  await grid(page).getByRole('option', { name: 'Position 5, original page 5, selected' }).focus();
  await page.keyboard.press('m');
  await expect(page.getByRole('dialog', { name: 'Move 2 pages to a position' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(grid(page).getByRole('option', { name: 'Position 5, original page 5, selected' })).toBeFocused();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
});

test('Page order reverses the selection or the whole document, each as one undo step', async ({ page }) => {
  await openReport(page);
  await option(page, 1).click();
  await pageActions(page).getByRole('button', { name: 'Page order' }).click();
  await expect(page.getByRole('menuitem', { name: 'Reverse the selected pages' })).toBeDisabled();
  await page.keyboard.press('Escape');
  await selectRun(page, 2, 4);
  await pageActions(page).getByRole('button', { name: 'Page order' }).click();
  await page.getByRole('menuitem', { name: 'Reverse the 3 selected pages' }).click();
  expect(await order(page)).toEqual([1, 4, 3, 2, 5]);
  await expect(liveStatus(page)).toHaveText('Pages 2 to 4 reversed');
  await pageActions(page).getByRole('button', { name: 'Page order' }).click();
  await page.getByRole('menuitem', { name: 'Reverse the whole document' }).click();
  expect(await order(page)).toEqual([5, 2, 3, 4, 1]);
  await expect(liveStatus(page)).toHaveText('All 5 pages reversed');
  await undoButton(page).click();
  expect(await order(page)).toEqual([1, 4, 3, 2, 5]);
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  expect(await exportedOrder(page)).toEqual([5, 2, 3, 4, 1]);
});

test('Page order interleaves a two-sided scan into reading order', async ({ page }) => {
  await openReport(page);
  await pageActions(page).getByRole('button', { name: 'Page order' }).click();
  await expect(page.getByRole('menuitem', { name: /Interleave a two-sided scan/ })).toContainText('For fronts scanned first, then backs scanned in reverse. Puts the pages in reading order: 1, 5, 2, 4…');
  await page.getByRole('menuitem', { name: /Interleave a two-sided scan/ }).click();
  expect(await order(page)).toEqual([1, 5, 2, 4, 3]);
  await expect(liveStatus(page)).toHaveText('Pages put in reading order: 1, 5, 2, 4…');
  expect(await exportedOrder(page)).toEqual([1, 5, 2, 4, 3]);
});

for (const reducedMotion of ['no-preference', 'reduce'] as const) {
  test(`a move ${reducedMotion === 'reduce' ? 'jumps without animation under reduced motion' : 'slides pages from where they were'}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion });
    await page.addInitScript(() => {
      const w = window as any;
      w.animations = 0;
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (...args: Parameters<typeof animate>) { w.animations += 1; return animate.apply(this, args); };
    });
    await setPlatform(page, 'MacIntel');
    await openReport(page);
    await option(page, 1).click();
    await page.keyboard.press('Meta+ArrowRight');
    expect(await order(page)).toEqual([2, 1, 3, 4, 5]);
    const animations = await page.evaluate(() => (window as any).animations);
    if (reducedMotion === 'reduce') expect(animations).toBe(0);
    else expect(animations).toBeGreaterThan(0);
  });
}

test('Move buttons and Page order are disabled while exporting', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    const wait = setInterval(() => {
      const internals = w.__TAURI_INTERNALS__;
      if (!internals) return;
      clearInterval(wait);
      const invoke = internals.invoke;
      internals.invoke = (command: string, args: any) => command === 'export_pdf_scene' ? (w.calls.push({ command, args }), new Promise(() => {})) : invoke(command, args);
    }, 1);
  });
  await setPlatform(page, 'MacIntel');
  await openReport(page);
  await option(page, 1).click();
  await page.keyboard.press('Meta+ArrowRight');
  await page.getByRole('button', { name: 'Export PDF' }).click();
  await expect(pageActions(page).getByRole('button', { name: 'Move later (⌘+→)' })).toBeDisabled();
  await expect(pageActions(page).getByRole('button', { name: 'Move to…' })).toBeDisabled();
  await expect(pageActions(page).getByRole('button', { name: 'Page order' })).toBeDisabled();
});

test('dragging into the band at the grid edge scrolls the grid and says which pages are coming into view', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 640 });
  await openReport(page);
  await page.getByRole('radiogroup', { name: 'Thumbnails' }).getByRole('radio', { name: 'Large' }).click();
  await option(page, 1).click();
  const start = await center(page, 1);
  const box = (await grid(page).boundingBox())!;
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x, start.y + 20, { steps: 4 });
  const band = page.locator('.scene-organize-scroll-band[data-edge="bottom"]');
  await expect(band).toHaveText('Drag here to scroll down');
  // The grid reaches its end quickly, so record every text the band shows while it scrolls.
  await band.evaluate((element) => {
    const w = window as any;
    w.bandTexts = [];
    new MutationObserver(() => w.bandTexts.push(`${element.hasAttribute('data-active')}:${element.textContent}`)).observe(element, { attributes: true, childList: true, subtree: true, characterData: true });
  });
  await page.mouse.move(start.x, box.y + box.height - 8, { steps: 8 });
  await expect.poll(() => grid(page).evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  const texts: string[] = await page.evaluate(() => (window as any).bandTexts);
  expect(texts.some((text) => /^true:Scrolling down\. Pages? .* coming into view$/.test(text))).toBe(true);
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(band).toHaveCount(0);
  expect(await order(page)).toEqual([1, 2, 3, 4, 5]);
});
