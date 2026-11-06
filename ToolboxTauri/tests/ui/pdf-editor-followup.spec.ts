import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from "./open-tool";

const svg = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/><text x="60" y="85" font-size="24">Selectable local PDF text</text></svg>');
const signatureAssetUrl = 'asset://localhost/' + encodeURIComponent('/local/signature.png');

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg, signatureAssetUrl }) => {
    const w = window as any;
    w.calls = [];
    w.dialogNext = null;
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: () => 1, unregisterCallback: () => {},
      convertFileSrc: (path: string) => path === '/local/signature.png' ? signatureAssetUrl : path,
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:dialog|open') {
          const next = w.dialogNext;
          w.dialogNext = null;
          return next ?? '/local/scene-fixture.pdf';
        }
        if (command === 'inspect_signature_image') {
          if (!/\.(png|jpe?g)$/i.test(args.request.path)) throw `${args.request.path.split('/').pop()} can't be used. Choose a PNG or JPG. A transparent PNG looks best.`;
          return { width: 400, height: 100 };
        }
        if (command === 'inspect_pdf_scene') return { path: '/local/scene-fixture.pdf', pages: [{ index: 0, width: 612, height: 792, preview: svg, textRuns: [{ text: 'Selectable local PDF text', x: 60, y: 55, width: 250, height: 25 }] }] };
        if (command === 'preview_pdf_scene_pages') return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        if (command === 'export_pdf_scene') return [{ inputPath: '/local/scene-fixture.pdf', outputPaths: ['/local/scene-fixture-edited.pdf'], state: 'succeeded', detail: 'PDF scene exported', failure: null, reason: null, warnings: [] }];
        return null;
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg, signatureAssetUrl });
});

async function openEditor(page: Page) {
  await page.goto('/');
  await openToolBySearch(page, "Edit PDF");
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'preview_pdf_scene_pages'))).toBe(true);
  await expect(page.locator('.scene-canvas image').first()).toBeAttached();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
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
  await expect(page.getByRole('textbox', { name: 'Text box text' })).toBeVisible();
  expect(await page.locator('input[aria-label="Object text"]').count()).toBe(0);
  await page.getByRole('textbox', { name: 'Text box text' }).fill('Edited directly on page');
  await page.getByRole('textbox', { name: 'Text box text' }).press('Control+Enter');
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

async function clickCanvas(page: Page, x: number, y: number) {
  const box = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.click(box.x + box.width * x, box.y + box.height * y);
}

test('signature panel offers a typed name or a picture, with plain wording and no font or drawing controls', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await expect(page.getByText("This adds a picture of a signature. It isn't a certified digital signature.")).toBeVisible();
  await expect(page.getByRole('radiogroup', { name: 'Signature source' })).toBeVisible();
  await expect(page.getByRole('combobox')).toHaveCount(0);
  await expect(page.getByText(/font/i)).toHaveCount(0);
  await expect(page.getByRole('slider', { name: 'Object opacity' })).toHaveCount(0);
  await expect(page.locator('input[aria-label="Object color"]')).toHaveCount(0);
  await expect(page.getByText(/certified signature|digitally signed|legally binding/i)).toHaveCount(0);
});

test('a signature picture is checked before it can be placed and only PNG or JPG are offered', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await page.getByRole('radio', { name: 'Picture', exact: true }).click();
  await page.evaluate(() => { (window as any).dialogNext = '/local/signature.webp'; });
  await page.getByRole('button', { name: 'Choose a PNG or JPG', exact: true }).click();
  const dialog = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'plugin:dialog|open').at(-1).args.options);
  expect(dialog.filters).toEqual([{ name: 'Signature picture', extensions: ['png', 'jpg', 'jpeg'] }]);
  await expect(page.getByRole('alert')).toHaveText("signature.webp can't be used. Choose a PNG or JPG. A transparent PNG looks best.");
  await expect(page.getByText('Choose a PNG or JPG before placing it.')).toBeVisible();
  await dragOnCanvas(page, .18, .22, .25, .12);
  await clickCanvas(page, .5, .5);
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);

  await page.evaluate(() => { (window as any).dialogNext = '/local/signature.png'; });
  await page.getByRole('button', { name: 'Choose a PNG or JPG', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByText('signature.png', { exact: true })).toBeVisible();
  await expect(page.locator('.scene-signature-mini-preview')).toHaveAttribute('src', signatureAssetUrl);
  await clickCanvas(page, .4, .4);
  await expect.poll(() => page.locator('.scene-canvas image').evaluateAll((images) => images.map((image) => image.getAttribute('href'))))
    .toContain(signatureAssetUrl);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0]).toMatchObject({ kind: 'signature', signatureMode: 'image', signaturePath: '/local/signature.png', opacity: 1 });
  expect(scene.pages[0].objects[0].pageRange).toBeUndefined();
  expect(scene.pages[0].objects[0].rect).toMatchObject({ width: 160, height: 56 });
});

test('a typed signature previews in the chosen ink, is placed with a click and can be used again', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  const name = page.getByRole('textbox', { name: 'Name', exact: true });
  await name.fill('');
  await expect(page.getByText('Type a name before placing it.')).toBeVisible();
  await clickCanvas(page, .3, .3);
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await name.fill('A. Local');
  await page.getByRole('radio', { name: 'Dark blue', exact: true }).click();
  const preview = page.getByRole('img', { name: 'Preview', exact: true });
  await expect(preview).toContainText('A. Local');
  await expect(preview.locator('text')).toHaveAttribute('fill', '#1a3a8f');
  await clickCanvas(page, .5, .5);
  await expect(page.locator('.scene-object-hit')).toHaveCount(1);
  const reuse = page.getByRole('list', { name: 'Use again' });
  await expect(reuse.getByRole('button')).toHaveText(['A. LocalTyped']);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await name.fill('Someone else');
  await page.getByRole('radio', { name: 'Black', exact: true }).click();
  await reuse.getByRole('button', { name: /A\. Local/ }).click();
  await expect(name).toHaveValue('A. Local');
  await expect(page.getByRole('radio', { name: 'Dark blue', exact: true })).toBeChecked();
  await clickCanvas(page, .5, .8);
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects).toHaveLength(2);
  for (const object of scene.pages[0].objects) expect(object).toMatchObject({ kind: 'signature', signatureMode: 'text', text: 'A. Local', color: '#1a3a8f' });
  expect(scene.pages[0].objects[0].fontFamily).toBeUndefined();
  await expect(reuse.getByRole('button')).toHaveCount(1);
  await page.getByRole('region', { name: 'Properties' }).getByRole('button', { name: 'Clear list', exact: true }).click();
  await expect(page.getByRole('list', { name: 'Use again' })).toHaveCount(0);
});

test('a signature can be placed on several pages and keeps its proportions when resized', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await dragOnCanvas(page, .3, .3, .3, .1);
  await page.getByRole('radio', { name: 'These pages, for initials', exact: true }).click();
  const pages = page.getByRole('textbox', { name: 'Pages for this signature', exact: true });
  await expect(pages).toHaveAttribute('placeholder', 'For example 1-13');
  await pages.fill('1-');
  await pages.press('Enter');
  const before = (await sceneFromExport(page)).pages[0].objects[0];
  expect(before.pageRange).toBe('1-');
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'signature object 1', exact: true }).focus();
  await page.keyboard.press('Space');
  await page.getByRole('button', { name: 'Resize signature se', exact: true }).focus();
  await page.keyboard.press('Shift+ArrowRight');
  const handle = (await page.getByRole('button', { name: 'Resize signature se', exact: true }).boundingBox())!;
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x + 80, handle.y + handle.height / 2, { steps: 5 });
  await page.mouse.up();
  const after = (await sceneFromExport(page)).pages[0].objects[0];
  expect(after.rect.width).toBeGreaterThan(before.rect.width + 20);
  expect(after.rect.width / after.rect.height).toBeCloseTo(before.rect.width / before.rect.height, 2);
  expect([after.rect.x, after.rect.y]).toEqual([before.rect.x, before.rect.y]);
});

test('typed signature name keeps focus for every character', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Signature', exact: true }).click();
  await dragOnCanvas(page, .3, .3, .22, .1);
  const input = page.getByRole('textbox', { name: 'Name', exact: true });
  await input.fill('');
  for (const character of 'A. Local') {
    await input.press(character === ' ' ? 'Space' : character);
    await expect(input).toBeFocused();
  }
  await expect(input).toHaveValue('A. Local');
});

test('watermarks stay fixed and expose font size as a dropdown', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await expect(page.locator('input[type="color"][aria-label="Object color"]')).toBeVisible();
  await expect(page.getByRole('slider', { name: 'Object opacity' })).toBeVisible();
  await page.getByRole('textbox', { name: 'Watermark text' }).fill('PRIVATE');
  await page.getByRole('combobox', { name: 'Watermark pattern' }).selectOption('bottom-right-to-top-left');
  await page.getByRole('button', { name: 'Add fixed watermark', exact: true }).click();
  const watermark = page.getByRole('button', { name: 'watermark object 1', exact: true });
  await watermark.focus();
  await expect(page.getByRole('button', { name: 'Resize watermark se', exact: true })).toHaveCount(0);
  const before = await sceneFromExport(page);
  const box = (await watermark.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + 100, box.y + 50, { steps: 5 }); await page.mouse.up();
  const afterDrag = await sceneFromExport(page);
  expect(afterDrag.pages[0].objects[0].rect).toEqual(before.pages[0].objects[0].rect);
  await page.getByRole('combobox', { name: 'Watermark font size' }).selectOption('72');
  const scene = await sceneFromExport(page);
  expect(scene.pages[0].objects[0]).toMatchObject({ kind: 'watermark', text: 'PRIVATE', watermarkPattern: 'bottom-right-to-top-left', fontSize: 72 });
  expect(scene.pages[0].objects[0].rect).toEqual(before.pages[0].objects[0].rect);
});
