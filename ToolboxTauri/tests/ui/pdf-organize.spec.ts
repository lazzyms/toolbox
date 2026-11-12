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
    ['1-3, 250', 'The PDF has 5 pages, so page 250 can\'t be used.'],
    ['4-2', '"4-2" runs backwards. Put the lower page first, for example 2-4.'],
    ['0', '"0" uses page 0. Page numbers start at 1.'],
    ['two', '"two" isn\'t a page or range. Use pages and ranges like 1-3, 5, 8-.'],
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
