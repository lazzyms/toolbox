import { test, expect, type Page } from '@playwright/test';

const svg = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/><text x="60" y="85" font-size="24">Selectable local PDF text</text></svg>');

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg }) => {
    const w = window as any;
    w.calls = [];
    w.dialogNext = null;
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: () => 1, unregisterCallback: () => {},
      convertFileSrc: (path: string) => path === '/local/signature.png' ? svg : path,
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:dialog|open') {
          const next = w.dialogNext;
          w.dialogNext = null;
          return next ?? '/local/scene-fixture.pdf';
        }
        if (command === 'inspect_pdf_scene') return { path: '/local/scene-fixture.pdf', pages: [{ index: 0, width: 612, height: 792, preview: svg, textRuns: [{ text: 'Selectable local PDF text', x: 60, y: 55, width: 250, height: 25 }] }] };
        if (command === 'preview_pdf_scene') return { dataUrl: svg, width: 612, height: 792 };
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

async function dragOnCanvas(page: Page, x: number, y: number, width = 0.28, height = 0.08) {
  const box = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(box.x + box.width * x, box.y + box.height * y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * (x + width), box.y + box.height * (y + height), { steps: 6 });
  await page.mouse.up();
}

async function sceneFromExport(page: Page) {
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  return page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').at(-1).args.request.scene);
}

test('text edits happen in the page box and do not expose a duplicate property input', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Text', exact: true }).click();
  await dragOnCanvas(page, .18, .18);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).dblclick();
  await expect(page.getByRole('textbox', { name: 'Edit text object' })).toBeVisible();
  expect(await page.locator('input[aria-label="Object text"]').count()).toBe(0);
  await page.getByRole('textbox', { name: 'Edit text object' }).fill('Edited directly on page');
  await page.getByRole('textbox', { name: 'Edit text object' }).press('Control+Enter');
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects[0].text).toBe('Edited directly on page');
});

test('highlight can convert a selected source-text range into highlights', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Highlight', exact: true }).click();
  const text = page.locator('[data-text-run]').first();
  const box = (await text.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2, { steps: 8 });
  await page.mouse.up();
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0]).toMatchObject({ kind: 'highlight', highlightMode: 'text-selection' });
});

test('text selection cancels on outside release and leaves later drawing usable', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Highlight', exact: true }).click();
  const text = page.locator('[data-text-run]').first();
  const box = (await text.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(5, 5, { steps: 4 });
  await page.mouse.up();

  await page.getByRole('button', { name: 'Text', exact: true }).click();
  await dragOnCanvas(page, .2, .2);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0].kind).toBe('text');
});

test('text selection cancels on window blur and leaves later drawing usable', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Highlight', exact: true }).click();
  const text = page.locator('[data-text-run]').first();
  const box = (await text.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await page.mouse.up();

  await page.getByRole('button', { name: 'Shape', exact: true }).click();
  await dragOnCanvas(page, .25, .25);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0].kind).toBe('shape');
});

test('space-drag and middle-button pan leave rotated document coordinates unchanged', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Text', exact: true }).click();
  await dragOnCanvas(page, .2, .2);
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate selected pages', exact: true }).click();
  const beforePan = await sceneFromExport(page);
  const host = page.locator('.scene-canvas');
  const canvas = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;

  await page.evaluate(() => document.querySelector<HTMLElement>('.scene-canvas')?.scrollTo(0, 0));
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2 - 120, { steps: 6 });
  await page.mouse.up({ button: 'middle' });
  const afterMiddle = await host.evaluate((node) => ({ left: node.scrollLeft, top: node.scrollTop }));
  expect(afterMiddle.top + afterMiddle.left).toBeGreaterThan(0);

  await page.evaluate(() => document.querySelector<HTMLElement>('.scene-canvas')?.scrollTo(0, 0));
  await page.keyboard.down(' ');
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
  await page.mouse.down();
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2 - 120, { steps: 6 });
  await page.mouse.up();
  await page.keyboard.up(' ');
  const afterSpace = await host.evaluate((node) => ({ left: node.scrollLeft, top: node.scrollTop }));
  expect(afterSpace.top + afterSpace.left).toBeGreaterThan(0);

  const afterPan = await sceneFromExport(page);
  expect(afterPan.pages[0].objects[0].rect).toEqual(beforePan.pages[0].objects[0].rect);
});

test('shape tool exposes every requested shape and arrow direction', async ({ page }) => {
  await openEditor(page);
  const variants = ['square', 'round', 'triangle', 'line', 'dotted-line', 'arrow-left', 'arrow-right', 'arrow-up', 'arrow-down'];
  for (const [index, variant] of variants.entries()) {
    await page.getByRole('button', { name: 'Shape', exact: true }).click();
    await page.getByRole('combobox', { name: 'Shape type' }).selectOption(variant);
    await dragOnCanvas(page, .08 + (index % 3) * .28, .08 + Math.floor(index / 3) * .2, .16, .1);
  }
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects.map((object: any) => object.shape)).toEqual(variants);
});

test('signature supports a draggable local image and a typed font-backed box', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await page.getByRole('combobox', { name: 'Signature mode' }).selectOption('image');
  await page.evaluate(() => { (window as any).dialogNext = '/local/signature.png'; });
  await page.getByRole('button', { name: 'Choose signature image', exact: true }).click();
  await expect(page.getByText('signature.png', { exact: true })).toBeVisible();
  await dragOnCanvas(page, .18, .22, .25, .12);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await page.getByRole('combobox', { name: 'Signature mode' }).selectOption('text');
  await page.getByRole('textbox', { name: 'Signature text' }).fill('A. Local');
  await page.getByRole('combobox', { name: 'Signature font' }).selectOption('Times-Italic');
  await dragOnCanvas(page, .52, .48, .24, .12);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(2);
  expect(scene.pages[0].objects[0]).toMatchObject({ signatureMode: 'image', signaturePath: '/local/signature.png' });
  expect(scene.pages[0].objects[1]).toMatchObject({ signatureMode: 'text', text: 'A. Local', fontFamily: 'Times-Italic' });
});

test('watermarks are added as fixed patterns and cannot be moved by dragging', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await expect(page.locator('input[type="color"][aria-label="Object color"]')).toBeVisible();
  await expect(page.getByRole('slider', { name: 'Object opacity' })).toBeVisible();
  await page.getByRole('textbox', { name: 'Watermark text' }).fill('PRIVATE');
  await page.getByRole('combobox', { name: 'Watermark pattern' }).selectOption('bottom-right-to-top-left');
  await page.getByRole('button', { name: 'Add fixed watermark', exact: true }).click();
  const watermark = page.getByRole('button', { name: 'watermark object 1', exact: true });
  const before = await watermark.boundingBox();
  const box = before!;
  await page.mouse.move(box.x + 5, box.y + 5); await page.mouse.down(); await page.mouse.move(box.x + 100, box.y + 50, { steps: 5 }); await page.mouse.up();
  const after = await watermark.boundingBox();
  expect(after?.x).toBeCloseTo(box.x, 0);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects[0]).toMatchObject({ kind: 'watermark', text: 'PRIVATE', watermarkPattern: 'bottom-right-to-top-left' });
});
