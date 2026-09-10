import { test, expect, type Page } from '@playwright/test';

const svg = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/><text x="60" y="85" font-size="24">A local test document</text></svg>');
test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg }) => {
    const w = window as any;
    w.calls = [];
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:dialog|open') return '/local/scene-fixture.pdf';
        if (command === 'inspect_pdf_scene') return { path: '/local/scene-fixture.pdf', pages: [0, 1, 2].map(index => ({ index, width: 612, height: 792, preview: svg })) };
        if (command === 'preview_pdf_scene') {
          if (w.previewFailure) throw new Error('Renderer unavailable');
          if (w.previewDelay) await new Promise(resolve => setTimeout(resolve, w.previewDelay));
          return { dataUrl: svg, width: 612, height: 792 };
        }
        if (command === 'export_pdf_scene') return [{ inputPath: '/local/scene-fixture.pdf', outputPaths: ['/local/scene-fixture-edited-1.pdf'], detail: 'PDF scene exported', failure: null }];
        return null;
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg });
});

async function openEditor(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Open Edit PDF', exact: true }).click();
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
}
async function draw(page: Page, tool: string, y: number, x = 0.2) {
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: tool, exact: true }).click();
  const box = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(box.x + box.width * x, box.y + box.height * y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * (x + .35), box.y + box.height * (y + .07), { steps: 6 });
  await page.mouse.up();
}
async function exported(page: Page) {
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  return page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request.scene);
}

test('scene composes every mark kind, moves and resizes, then exports once', async ({ page }) => {
  await openEditor(page);
  for (const [i, name] of ['Text', 'Highlight', 'Shape', 'Signature'].entries()) await draw(page, name, .15 + i * .14);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add fixed watermark', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(5);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  const text = page.getByRole('button', { name: 'text object 1', exact: true });
  await text.focus();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Shift+ArrowRight');
  await page.getByRole('button', { name: 'Resize text se', exact: true }).focus();
  await page.keyboard.press('Shift+ArrowRight');
  const result = await exported(page);
  expect(result.pages[0].objects.map((o: any) => o.kind)).toEqual(['text', 'highlight', 'shape', 'signature', 'watermark']);
  expect(result.pages[0].objects[0].rect.x).toBeCloseTo(612 * .2 + 10, 0);
  expect(result.pages[0].objects[0].rect.width).toBeGreaterThan(612 * .35 + 8);
  expect(result.pages[0].objects[3]).toMatchObject({ signatureMode: 'text', fontFamily: 'Helvetica-Oblique' });
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').length)).toBe(1);
});

test('scene crop and thumbnail insert rotate reorder delete share undo redo reset', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Crop', .1, .1);
  await page.getByRole('button', { name: 'Add blank page' }).click();
  await expect(page.getByRole('button', { name: 'Page 2, blank', exact: true })).toHaveAttribute('aria-current', 'page');
  await draw(page, 'Shape', .3);
  await page.getByRole('button', { name: 'Rotate selected pages' }).click();
  await page.getByRole('button', { name: 'Move page earlier' }).click();
  let result = await exported(page);
  expect(result.pages[0]).toMatchObject({ sourceIndex: null, rotation: 90 });
  expect(result.pages[0].objects).toHaveLength(1);
  expect(result.pages[1].crop).not.toBeNull();
  await page.getByRole('button', { name: 'Delete selected pages' }).click();
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  result = await exported(page);
  expect(result.pages).toHaveLength(4);
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  result = await exported(page);
  expect(result.pages).toHaveLength(3);
  await page.getByRole('button', { name: 'Reset edits' }).click();
  result = await exported(page);
  expect(result.pages.map((p: any) => p.sourceIndex)).toEqual([0, 1, 2]);
  expect(result.pages.every((p: any) => p.crop === null && !p.objects.length && !p.rotation)).toBe(true);
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  expect(await page.getByRole('button', { name: 'Reset edits' }).isEnabled()).toBe(true);
});

test('scene drag reorders selected thumbnails together and protects the last page', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Page 2', exact: true }).click({ modifiers: ['Shift'] });
  await page.getByRole('button', { name: 'Page 3', exact: true }).dragTo(page.getByRole('button', { name: 'Page 1', exact: true }));
  const result = await exported(page);
  expect(result.pages.map((p: any) => p.sourceIndex)).toEqual([2, 0, 1]);
  await page.getByRole('button', { name: 'Page 1', exact: true }).click();
  await page.getByRole('button', { name: 'Page 3', exact: true }).click({ modifiers: ['Shift'] });
  await expect(page.getByRole('button', { name: 'Delete selected pages' })).toBeDisabled();
});

test('scene rejects stale and failed previews without exposing an unverified export', async ({ page }) => {
  await openEditor(page);
  await page.evaluate(() => { (window as any).previewDelay = 350; });
  await draw(page, 'Text', .2);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
  await page.evaluate(() => { (window as any).previewFailure = true; });
  await draw(page, 'Shape', .4);
  await expect(page.getByRole('alert')).toContainText('Renderer unavailable');
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

for (const theme of ['light', 'dark']) test(`scene preview and inline tools remain above the fold in ${theme}`, async ({ page }, testInfo) => {
  await page.addInitScript(theme => localStorage.setItem('toolbox-theme', theme), theme);
  await page.setViewportSize({ width: 1280, height: 800 });
  const remote: string[] = [];
  page.on('request', request => { if (!/^(http:\/\/(127\.0\.0\.1|localhost)|data:)/.test(request.url())) remote.push(request.url()); });
  await openEditor(page);
  const canvas = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.screenshot({ path: testInfo.outputPath(`scene-${theme}.png`) });
  expect(canvas.y).toBeLessThan(230);
  expect(canvas.height).toBeGreaterThan(440);
  expect(canvas.y + canvas.height).toBeLessThan(800);
  const toolbar = (await page.getByRole('toolbar', { name: 'PDF editor tools' }).boundingBox())!;
  expect(toolbar.y + toolbar.height).toBeLessThan(canvas.y);
  expect(remote).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath(`scene-${theme}.png`) });
});
