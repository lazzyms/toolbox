import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from "./open-tool";
import { UtilityRegistry } from '../../src/registry';

const svg = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/><text x="60" y="85" font-size="24">A local test document</text></svg>');
test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg }) => {
    const w = window as any;
    w.calls = [];
    w.sceneTextDelays = {};
    w.sceneTextResponses = {};
    w.dialogNext = null;
    w.scriptPacks = [];
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let nextId = 1;
    w.__emit = (event: string, payload: unknown) => {
      for (const [id, listener] of listeners) if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
    };
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: (callback: (event: unknown) => void) => { const id = nextId++; callbacks.set(id, callback); return id; },
      unregisterCallback: (id: number) => { callbacks.delete(id); },
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:event|listen') { const id = nextId++; listeners.set(id, { event: args.event, handler: args.handler }); return id; }
        if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return null; }
        if (command === 'script_packs') return w.scriptPacks;
        // Mirror the native scene contract: kit/pdf/scene.rs rejects any object color that is not #RRGGBB.
        for (const page of args?.request?.scene?.pages ?? []) for (const object of page.objects) {
          if (!/^#[0-9a-fA-F]{6}$/.test(object.color)) throw new Error('Color must be #RRGGBB');
        }
        if (command === 'check_output_folder') return 'ready';
        if (command === 'plugin:dialog|open') return w.dialogNext ?? '/local/scene-fixture.pdf';
        if (command === 'inspect_pdf_scene') return { path: '/local/scene-fixture.pdf', pages: [0, 1, 2].map(index => ({ index, width: 612, height: 792, preview: null })) };
        if (command === 'inspect_pdf_scene_page') {
          const pageIndex = args.request.pageIndex;
          const delay = w.sceneTextDelays[pageIndex] ?? 0;
          if (delay) await new Promise(resolve => setTimeout(resolve, delay));
          const key = `${args.request.path}:${pageIndex}`;
          const response = (w.sceneTextResponses[key] ?? 0) + 1;
          w.sceneTextResponses[key] = response;
          return { index: pageIndex, width: 612, height: 792, preview: svg, textRuns: [{ text: `${args.request.path} page ${pageIndex} response ${response}`, x: 60, y: 55, width: 250, height: 25 }],
            editRuns: [
              { index: 0, text: 'Rent £1,450', x: 60, y: 120, width: 120, height: 14, fontName: 'Helvetica', fontSize: 12, reason: null, substitute: { family: 'Noto Serif', characters: ' ,01457Rent£' } },
              { index: 1, text: 'Stamped by the Landlord', x: 60, y: 400, width: 200, height: 14, fontName: 'Helvetica', fontSize: 12, reason: 'picture' },
            ] };
        }
        if (command === 'render_pdf_source_thumbnails') return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, thumbnail: { dataUrl: svg, width: 247, height: 320 }, error: null }));
        if (command === 'preview_pdf_scene_pages') {
          if (w.previewFailure) throw new Error(w.previewFailureMessage ?? 'Renderer unavailable');
          if (w.previewDelay) await new Promise(resolve => setTimeout(resolve, w.previewDelay));
          return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        }
        if (command === 'install_pack') return { packs: [], installedCount: 1, totalBytes: 0 };
        if (command === 'inspect_pdf_watermark_image') {
          if (args.request.path.includes('huge')) throw 'This picture is larger than 20 megapixels, the limit for a watermark. Use a smaller picture.';
          return { width: 400, height: 200 };
        }
        if (command === 'export_pdf_scene') {
          if (w.exportOutcomes) return w.exportOutcomes;
          // Mirror the native naming: Export As numbers a taken chosen name and says so.
          const chosen: string | undefined = args.request.outputPath;
          const numbered = chosen?.replace(/\.pdf$/, '-1.pdf');
          const name = (path: string) => path.split('/').pop();
          return [{ inputPath: '/local/scene-fixture.pdf', outputPaths: [numbered ?? w.exportOutput ?? '/local/scene-fixture-edited.pdf'], state: 'succeeded', detail: w.exportDetail ?? 'PDF scene exported', failure: null, reason: null,
            warnings: numbered ? [{ code: 'renamedToAvoidReplacing', message: '', details: { chosenName: name(chosen!), finalName: name(numbered) } }] : [] }];
        }
        return null;
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg });
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

test('PDF editor toolbars use roving keyboard focus and render the workspace', async ({ page }, testInfo) => {
  await openEditor(page);
  await draw(page, 'Shape', .4);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
  const tools = page.getByRole('toolbar', { name: 'PDF editor tools' });
  const selectTool = tools.getByRole('button', { name: 'Select', exact: true });
  const textTool = tools.getByRole('button', { name: 'Text', exact: true });
  await selectTool.focus();
  await page.keyboard.press('ArrowRight');
  await expect(tools.getByRole('button', { name: 'Edit text', exact: true })).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(textTool).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(textTool).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('End');
  await expect(tools.getByRole('button', { name: 'Export PDF', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(tools.getByRole('button', { name: 'Organize', exact: true })).toBeFocused();
  await expect(page.getByRole('toolbar', { name: 'PDF page and zoom controls' })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('pdf-editor-workspace.png'), fullPage: true });
});

test('PDF editor keeps the back link, title, and empty-state open action compact', async ({ page }) => {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  page.on('console', message => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto('/');
  await openToolBySearch(page, "Edit PDF");
  await expect(page.locator('.pdf-editor-header-line')).toContainText('PDF Editor');
  await expect(page.locator('.pdf-editor-header-line')).toContainText('Open files');
  await expect(page.getByRole('button', { name: 'Choose files to process' })).toBeVisible();
  await expect(page.locator('.workspace-source-bar[data-empty="true"]')).toBeHidden();
  await expect(page.getByRole('button', { name: '← All tools', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.locator('[aria-label="Object properties"]')).toHaveCount(0);
  expect(consoleErrors).toEqual([]);
  expect(pageErrors).toEqual([]);
});

test('scene preview cache misses use one indexed batch request', async ({ page }) => {
  await openEditor(page);
  const previewCalls = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene_pages'));
  expect(previewCalls).toHaveLength(1);
  expect(previewCalls[0].args).toMatchObject({ request: { pageIndices: [0, 1] } });
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene').length)).toBe(0);
});

test('PDF editor toolbar uses roving keyboard focus and the inspector follows the active tool', async ({ page }) => {
  await openEditor(page);
  const toolbar = page.getByRole('toolbar', { name: 'PDF editor tools' });
  const select = toolbar.getByRole('button', { name: 'Select', exact: true });
  await select.focus();
  await page.keyboard.press('ArrowRight');
  await expect(toolbar.getByRole('button', { name: 'Edit text', exact: true })).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(toolbar.getByRole('button', { name: 'Text', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(toolbar.getByRole('button', { name: 'Organize', exact: true })).toBeFocused();

  const inspector = page.getByRole('complementary', { name: 'PDF editor inspector' });
  await expect(inspector).toBeVisible();
  await expect(inspector.getByRole('region', { name: 'Properties' })).toBeVisible();
  await toolbar.getByRole('button', { name: 'Shape', exact: true }).click();
  await expect(inspector.getByRole('combobox', { name: 'Shape type' })).toBeVisible();
});

test('page actions menu supports keyboard navigation, Escape focus return, and outside dismissal', async ({ page }) => {
  await openEditor(page);
  const trigger = page.getByRole('button', { name: 'Page actions' });
  await expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
  await trigger.focus();
  await page.keyboard.press('Enter');
  const menu = page.getByRole('menu', { name: 'Page actions' });
  await expect(menu).toBeVisible();
  const thumbnailSidebar = page.getByRole('complementary', { name: 'PDF page thumbnails' });
  const sidebarBounds = await thumbnailSidebar.boundingBox();
  const menuBounds = await menu.boundingBox();
  expect(sidebarBounds).not.toBeNull();
  expect(menuBounds).not.toBeNull();
  expect(menuBounds!.x).toBeGreaterThanOrEqual(sidebarBounds!.x + sidebarBounds!.width);
  expect(menuBounds!.y + menuBounds!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  await expect(page.getByRole('menuitem', { name: 'Merge PDF' })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'Split PDF' })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(page.getByRole('menuitem', { name: 'Merge PDF' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(trigger).toBeFocused();

  await trigger.click();
  await expect(menu).toBeVisible();
  await page.getByRole('heading', { name: 'PDF editor' }).click();
  await expect(menu).toBeHidden();
});

for (const [utility, command] of [['Merge PDF', 'merge_pdfs'], ['Split PDF', 'split_pdf']] as const) {
  test(`page actions open ${utility} with the current source and do not run it`, async ({ page }) => {
    await openEditor(page);
    await page.getByRole('button', { name: 'Page actions' }).click();
    await page.getByRole('menuitem', { name: utility }).click();
    await expect(page.getByRole('heading', { name: 'PDF Editor', exact: true })).toBeFocused();
    await expect(page.getByRole('toolbar', { name: 'PDF Editor tools' }).getByRole('button', { name: utility, exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'scene-fixture.pdf /local', exact: true })).toBeVisible();
    expect(await page.evaluate((command) => (window as any).calls.filter((call: any) => call.command === command).length, command)).toBe(0);
    if (utility === 'Merge PDF') {
      await page.evaluate(() => { (window as any).dialogNext = '/local/second-scene-fixture.pdf'; });
      await page.getByRole('button', { name: 'Choose files to process' }).click();
      await expect(page.getByRole('button', { name: 'second-scene-fixture.pdf /local', exact: true })).toBeVisible();
    } else {
      await page.getByRole('textbox', { name: 'Page ranges' }).fill('1');
    }
    await page.locator('.workspace-primary-action').click();
    if (utility === 'Merge PDF') await page.getByRole('dialog').getByRole('button', { name: 'Save' }).click();
    await expect.poll(() => page.evaluate((command) => (window as any).calls.filter((call: any) => call.command === command).at(-1)?.args.request.paths, command))
      .toEqual(utility === 'Merge PDF' ? ['/local/scene-fixture.pdf', '/local/second-scene-fixture.pdf'] : ['/local/scene-fixture.pdf']);
  });
}

test('PostHog flags select tools per OS in search and PDF page actions', async ({ page }, testInfo) => {
  const platform = process.env.TOOLBOX_EXPECTED_PLATFORM;
  test.skip(platform !== 'macos' && platform !== 'windows');
  const targetPlatform = platform as 'macos' | 'windows';
  const otherPlatform = targetPlatform === 'macos' ? 'windows' : 'macos';
  const mergeEnabled = targetPlatform === 'windows';
  const featureFlags: Record<string, boolean> = {};
  for (const { id } of UtilityRegistry) {
    const available = id !== 'pdf-extract';
    featureFlags[`tool-${id}-${targetPlatform}`] = available && (id !== 'pdf-merge' || mergeEnabled);
    featureFlags[`tool-${id}-${otherPlatform}`] = available && (id !== 'pdf-merge' || !mergeEnabled);
  }
  const flagRequests: Array<{ url: string; body: Record<string, unknown> }> = [];
  let failFlagRequests = false;
  let failedFlagRequestCount = 0;
  await page.route('**/capture/**', route => route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }));
  await page.route('**/flags/**', async route => {
    if (failFlagRequests) {
      failedFlagRequestCount += 1;
      await route.abort();
      return;
    }
    flagRequests.push({ url: route.request().url(), body: route.request().postDataJSON() });
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ featureFlags, errorsWhileComputingFlags: false }) });
  });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Tool library', exact: true })).toBeVisible();
  const mergeTool = page.getByRole('button', { name: 'Open Merge PDF', exact: true });
  if (mergeEnabled) await expect(mergeTool).toBeVisible();
  else await expect(mergeTool).toHaveCount(0);

  const search = page.getByRole('textbox', { name: 'Search tools' });
  await search.fill('Merge PDF');
  if (mergeEnabled) await expect(mergeTool).toBeVisible();
  else await expect(mergeTool).toHaveCount(0);
  await search.fill('Extract from PDF');
  const pdfExtractTool = page.getByRole('button', { name: 'Open Extract from PDF', exact: true });
  await expect(pdfExtractTool).toHaveCount(0);
  await search.fill('');

  await openToolBySearch(page, "Edit PDF");
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  const pageActions = page.getByRole('button', { name: 'Page actions' });
  await expect(pageActions).toBeEnabled();
  await pageActions.click();
  const menu = page.getByRole('menu', { name: 'Page actions' });
  await expect(menu).toBeVisible();
  const mergeAction = menu.getByRole('menuitem', { name: 'Merge PDF', exact: true });
  if (mergeEnabled) {
    await expect(mergeAction).toBeVisible();
    await mergeAction.click();
    await expect(page.getByRole('heading', { name: 'PDF Editor', exact: true })).toBeFocused();
    await expect(page.getByRole('toolbar', { name: 'PDF Editor tools' }).getByRole('button', { name: 'Merge PDF', exact: true })).toBeVisible();
  } else {
    await expect(mergeAction).toHaveCount(0);
    await expect(menu.getByRole('menuitem', { name: 'Split PDF', exact: true })).toBeVisible();
  }

  await expect.poll(() => flagRequests.length).toBe(1);
  expect(flagRequests[0].url).toBe('https://us.i.posthog.com/flags/?v=2');
  expect(flagRequests[0].body).toMatchObject({ api_key: 'phc_toolbox_e2e_flags', geoip_disable: true });
  expect(typeof flagRequests[0].body.distinct_id).toBe('string');
  await page.screenshot({ path: testInfo.outputPath(`tool-flags-${targetPlatform}.png`), fullPage: true });

  failFlagRequests = true;
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Tool library', exact: true })).toBeVisible();
  if (mergeEnabled) await expect(mergeTool).toBeVisible();
  else await expect(mergeTool).toHaveCount(0);
  await search.fill('Extract from PDF');
  await expect(pdfExtractTool).toHaveCount(0);
  expect(failedFlagRequestCount).toBe(1);
});

test('non-adjacent page selection rotates only the selected pages', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Page 1', exact: true }).click();
  await page.getByRole('button', { name: 'Page 3', exact: true }).click({ modifiers: ['ControlOrMeta'] });
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('button', { name: 'Page 3', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Rotate selected pages' }).click();
  const result = await exported(page);
  expect(result.pages.map((item: any) => item.rotation)).toEqual([90, 0, 90]);
});

test('inspector property edits stay in scene history and undo and redo exactly', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Shape', .3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'shape object 1', exact: true }).click();
  const inspector = page.getByRole('complementary', { name: 'PDF editor inspector' });
  const shape = inspector.getByRole('combobox', { name: 'Shape type' });
  await shape.selectOption('triangle');
  let result = await exported(page);
  expect(result.pages[0].objects[0].shape).toBe('triangle');
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  result = await exported(page);
  expect(result.pages[0].objects[0].shape).toBe('square');
  await page.getByRole('button', { name: /^Redo \(/ }).click();
  result = await exported(page);
  expect(result.pages[0].objects[0].shape).toBe('triangle');
});

const pageNumberHit = (page: Page) => page.getByRole('button', { name: /^Page number / });
async function addPageNumbers(page: Page) {
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Page numbers', exact: true }).click();
  await page.getByRole('button', { name: 'Add page numbers', exact: true }).click();
  await expect(page.getByText('Page numbers added to all 3 pages.')).toBeVisible();
}

test('page numbers are one range-applied object, and every option is sent exactly to preview and export', async ({ page }) => {
  await openEditor(page);
  await addPageNumbers(page);
  const sent = async () => (await previewScene(page)).pages[0].objects[0];
  const base = {
    kind: 'pageNumbers', text: '', fontSize: 9, opacity: 1, fontFamily: 'Noto Sans', layer: 'front', pageRange: 'all',
    pageNumbers: { format: 'page-of-total', start: 1, position: 'bottom-center', margin: 36 },
  };
  await expect.poll(sent).toMatchObject(base);
  await expect(pageNumberHit(page)).toHaveAccessibleName('Page number Page 1 of 3');
  const option = async (patch: object, act: () => Promise<void>) => {
    await act();
    Object.assign(base, patch);
    await expect.poll(sent).toMatchObject(base);
  };
  for (const [format, label, text] of [['page', 'Page 1', 'Page 1'], ['number', '1', '1'], ['number-of-total', '1 / 3', '1 / 3'], ['dashes', '- 1 -', '- 1 -'], ['page-of-total', 'Page 1 of 3', 'Page 1 of 3']]) {
    await option({ pageNumbers: { ...base.pageNumbers, format } }, () => page.getByRole('combobox', { name: 'Format' }).selectOption({ label }));
    await expect(pageNumberHit(page)).toHaveAccessibleName(`Page number ${text}`);
  }
  for (const [position, label] of [['top-left', 'Top left'], ['top-center', 'Top centre'], ['top-right', 'Top right'], ['middle-left', 'Middle left'], ['middle-center', 'Centre'], ['middle-right', 'Middle right'], ['bottom-left', 'Bottom left'], ['bottom-right', 'Bottom right'], ['bottom-center', 'Bottom centre']]) {
    await option({ pageNumbers: { ...base.pageNumbers, position } }, () => page.getByRole('radio', { name: label, exact: true }).click());
    await expect(page.getByRole('radio', { name: label, exact: true })).toHaveAttribute('aria-checked', 'true');
  }
  await option({ pageNumbers: { ...base.pageNumbers, start: 7 } }, () => page.getByRole('spinbutton', { name: 'Start at' }).fill('7'));
  await expect(pageNumberHit(page)).toHaveAccessibleName('Page number Page 7 of 9');
  await option({ pageNumbers: { ...base.pageNumbers, margin: 54 } }, () => page.getByRole('combobox', { name: 'Distance from edge' }).selectOption('54'));
  await option({ fontSize: 72 }, () => page.getByRole('combobox', { name: 'Size' }).selectOption('72'));
  await option({ fontSize: 8 }, () => page.getByRole('combobox', { name: 'Size' }).selectOption('8'));
  await option({ fontFamily: 'Noto Serif' }, () => page.getByRole('combobox', { name: 'Font' }).selectOption('Noto Serif'));
  await option({ pageRange: '2-' }, () => page.getByRole('radio', { name: 'Skip the first page' }).click());
  await expect(page.getByText('Page numbers added to 2 of 3 pages.')).toBeVisible();
  await expect(pageNumberHit(page)).toHaveCount(0);
  await page.getByRole('radio', { name: 'These pages' }).click();
  const range = page.getByRole('textbox', { name: 'Pages to include' });
  await range.fill('3-9');
  await range.press('Enter');
  await expect.poll(sent).toMatchObject({ ...base, pageRange: '3-9' });
  await expect(page.getByText('Page numbers added to 1 of 3 pages.')).toBeVisible();
  const scene = await exported(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0]).toMatchObject({ ...base, pageRange: '3-9' });
  expect(scene.pages[0].objects[0].pageNumbers).toEqual({ format: 'page-of-total', start: 7, position: 'bottom-center', margin: 54 });
  expect(scene.pages.slice(1).map((item: any) => item.objects)).toEqual([[], []]);
  await page.getByRole('button', { name: 'Page 3', exact: true }).click();
  await expect(pageNumberHit(page)).toHaveAccessibleName('Page number Page 9 of 9');
});

test('page numbers follow the page order after a reorder, in the preview and the export', async ({ page }) => {
  await openEditor(page);
  await addPageNumbers(page);
  await page.getByRole('radio', { name: 'Skip the first page' }).click();
  await expect(pageNumberHit(page)).toHaveCount(0);
  await page.evaluate(() => { (window as any).previewDelay = 60_000; });
  await page.getByRole('button', { name: 'Move page later' }).click();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
  // The draft shows the resolved number before the rendered preview arrives.
  await expect(pageNumberHit(page)).toHaveAccessibleName('Page number Page 2 of 3');
  await expect(page.locator('.scene-canvas text', { hasText: 'Page 2 of 3' })).toHaveCount(1);
  await page.getByRole('button', { name: 'Page 1', exact: true }).click();
  await expect(pageNumberHit(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'Page 3', exact: true }).click();
  await expect(pageNumberHit(page)).toHaveAccessibleName('Page number Page 3 of 3');
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).not.toContainText('marks');
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toContainText('1 marks');
  await page.evaluate(() => { (window as any).previewDelay = 0; });
  const scene = await exported(page);
  expect(scene.pages.map((item: any) => item.sourceIndex)).toEqual([1, 0, 2]);
  expect(scene.pages.map((item: any) => item.objects.map((object: any) => object.kind))).toEqual([[], ['pageNumbers'], []]);
  expect(scene.pages[1].objects[0]).toMatchObject({ pageRange: '2-', pageNumbers: { format: 'page-of-total', start: 1 } });
});

test('PDF scene history retains exactly the latest 100 edits', async ({ page }) => {
  await openEditor(page);
  const rotate = page.getByRole('button', { name: 'Rotate selected pages' });
  for (let index = 0; index < 101; index += 1) await rotate.click();
  const undo = page.getByRole('button', { name: /^Undo \(/ });
  for (let index = 0; index < 100; index += 1) await undo.click();
  await expect(undo).toBeDisabled();
  const result = await exported(page);
  expect(result.pages.map((item: any) => item.rotation)).toEqual([90, 0, 0]);
});

test('pointer-down does not request another preview before the scene changes', async ({ page }) => {
  await openEditor(page);
  const canvas = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  const before = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene_pages').length);
  await page.getByRole('button', { name: 'Text', exact: true }).click();
  await page.mouse.move(canvas.x + canvas.width * .2, canvas.y + canvas.height * .2);
  await page.mouse.down();
  await page.waitForTimeout(250);
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene_pages').length)).toBe(before);
  expect(await page.locator('.scene-canvas image').count()).toBeGreaterThan(0);
  await page.mouse.up();
});

test('scene lazily requests text for the current source page', async ({ page }) => {
  await openEditor(page);
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_scene_page').map((call: any) => call.args.request.pageIndex))).toEqual([0]);
  await page.getByRole('button', { name: 'Highlight', exact: true }).click();
  await expect(page.locator('[data-text-run]')).toHaveCount(1);
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_scene_page').map((call: any) => call.args.request.pageIndex))).toEqual([0, 1]);
});

test('scene ignores stale page text responses after changing pages', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => { (window as any).sceneTextDelays[0] = 300; });
  await openToolBySearch(page, "Edit PDF");
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await page.getByRole('button', { name: 'Highlight', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_scene_page').map((call: any) => call.args.request.pageIndex))).toEqual([0, 1]);
  await expect.poll(() => page.evaluate(() => (window as any).sceneTextResponses['/local/scene-fixture.pdf:0'])).toBe(1);
  await expect.poll(() => page.locator('[data-text-run]').allTextContents()).toEqual(['/local/scene-fixture.pdf page 1 response 1']);
  await page.getByRole('button', { name: 'Page 1', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_scene_page').map((call: any) => call.args.request.pageIndex))).toEqual([0, 1, 0]);
  await expect.poll(() => page.locator('[data-text-run]').allTextContents()).toEqual(['/local/scene-fixture.pdf page 0 response 2']);
});

test('new text and watermark markup use native colors so Export PDF enables', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .3);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
  await expect(page.getByText(/Preview could not be verified/)).toHaveCount(0);
  const result = await exported(page);
  expect(result.pages[0].objects.map((o: any) => [o.kind, o.color, o.watermark?.color])).toEqual([['text', '#1b1b1b', undefined], ['watermark', '#c62828', '#c62828']]);
});

test('scene composes every mark kind, moves and resizes, then exports once', async ({ page }) => {
  await openEditor(page);
  for (const [i, name] of ['Text', 'Highlight', 'Shape', 'Signature'].entries()) await draw(page, name, .15 + i * .14);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(5);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  const text = page.getByRole('button', { name: 'text object 1', exact: true });
  await text.focus();
  await page.keyboard.press('Space');
  await page.keyboard.press('Shift+ArrowRight');
  await page.getByRole('button', { name: 'Resize text se', exact: true }).focus();
  await page.keyboard.press('Shift+ArrowRight');
  const result = await exported(page);
  expect(result.pages[0].objects.map((o: any) => o.kind)).toEqual(['text', 'highlight', 'shape', 'signature', 'watermark']);
  expect(result.pages[0].objects[0].rect.x).toBeCloseTo(612 * .2 + 10, 0);
  expect(result.pages[0].objects[0].rect.width).toBeGreaterThan(612 * .35 + 8);
  expect(result.pages[0].objects[3]).toMatchObject({ signatureMode: 'text', text: 'Your signature', color: '#1f1f1f', opacity: 1 });
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').length)).toBe(1);
});

test('scene crop and thumbnail rotate reorder delete share undo redo reset', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Crop', .1, .1);
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).toHaveAttribute('aria-current', 'page');
  await draw(page, 'Shape', .3);
  await page.getByRole('button', { name: 'Rotate selected pages' }).click();
  await page.getByRole('button', { name: 'Move page earlier' }).click();
  let result = await exported(page);
  expect(result.pages[0]).toMatchObject({ sourceIndex: 1, rotation: 90 });
  expect(result.pages[0].objects).toHaveLength(1);
  expect(result.pages[1].crop).not.toBeNull();
  await page.getByRole('button', { name: 'Delete selected pages' }).click();
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  result = await exported(page);
  expect(result.pages).toHaveLength(3);
  await page.getByRole('button', { name: /^Redo \(/ }).click();
  result = await exported(page);
  expect(result.pages).toHaveLength(2);
  await page.getByRole('button', { name: 'Reset…' }).click();
  await page.getByRole('alertdialog', { name: 'Reset all page changes?' }).getByRole('button', { name: 'Reset', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: /^Page \d/ })).toHaveText(['1', '2', '3'].map((n) => new RegExp(n)));
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  expect(await page.getByRole('button', { name: 'Reset…' }).isEnabled()).toBe(true);
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

test('Export stays disabled until the document has an effective change', async ({ page }) => {
  await openEditor(page);
  const exportButton = page.getByRole('button', { name: 'Export PDF', exact: true });
  await expect(exportButton).toBeDisabled();
  await draw(page, 'Shape', .4);
  await expect(exportButton).toBeEnabled();
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  await expect(exportButton).toBeDisabled();
  await page.getByRole('button', { name: /^Redo \(/ }).click();
  await expect(exportButton).toBeEnabled();
  await exportButton.click();
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene'))).toHaveLength(1);
});

test('Cmd+O opens a PDF in the editor and Cmd+Enter exports only once there is a change', async ({ page }) => {
  const count = (command: string) => page.evaluate((command) => (window as any).calls.filter((c: any) => c.command === command).length, command);
  // The hints and keys below are the macOS ones; pin the platform so the test means the same on Windows.
  await page.addInitScript(() => Object.defineProperty(window.navigator, 'platform', { configurable: true, value: 'MacIntel' }));
  await page.goto('/');
  await openToolBySearch(page, "Edit PDF");
  await expect(page.locator('.pdf-editor-open-files')).toContainText('⌘O');
  await page.locator('body').press('Meta+O');
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  expect(await count('plugin:dialog|open')).toBe(1);
  await expect(page.locator('.scene-canvas image').first()).toBeAttached();

  await page.locator('body').press('Meta+Enter');
  expect(await count('export_pdf_scene')).toBe(0);
  await draw(page, 'Shape', .4);
  const exportButton = page.getByRole('button', { name: 'Export PDF', exact: true });
  await expect(exportButton).toContainText('⌘↵');
  await expect(exportButton).toBeEnabled();
  await page.locator('body').press('Meta+Enter');
  await expect.poll(() => count('export_pdf_scene')).toBe(1);
});

test('scene rejects stale and failed previews without exposing an unverified export', async ({ page }) => {
  await openEditor(page);
  await expect.poll(() => page.locator('.scene-canvas image').evaluateAll((images) => images.map((image) => image.getAttribute('href')))).toContain(svg);
  await page.evaluate(() => { (window as any).previewDelay = 350; });
  await draw(page, 'Text', .2);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await expect.poll(() => page.locator('.scene-canvas image').evaluateAll((images) => images.map((image) => image.getAttribute('href')))).toContain(svg);
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await page.evaluate(() => { (window as any).previewFailure = true; });
  await draw(page, 'Shape', .4);
  await expect.poll(() => page.locator('.scene-canvas image').evaluateAll((images) => images.map((image) => image.getAttribute('href')))).toContain(svg);
  await expect(page.getByRole('alert')).toContainText('Renderer unavailable');
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

test('scene explains unsafe PDF preview failures without exposing native wrappers', async ({ page }) => {
  await openEditor(page);
  await page.evaluate(() => {
    (window as any).previewFailure = true;
    (window as any).previewFailureMessage = 'Error: This tagged PDF contains structure mappings that cannot be remapped by scene edits; scene export was rejected before output';
  });
  await draw(page, 'Shape', .4);
  await expect(page.getByRole('alert')).toHaveText('This tagged PDF uses accessibility structure that cannot be preserved after this edit. Undo the page edit or use a copy without tagged structure. Export is disabled until the preview can be verified.');
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

test('Escape cancels a canvas gesture before pointer release', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Text', exact: true }).click();
  const canvas = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(canvas.x + canvas.width * .2, canvas.y + canvas.height * .2);
  await page.mouse.down();
  await page.mouse.move(canvas.x + canvas.width * .5, canvas.y + canvas.height * .25, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();

  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Undo \(/ })).toBeDisabled();
});

test('Escape releases pointer capture while cancelling object movement', async ({ page }, testInfo) => {
  await openEditor(page);
  await draw(page, 'Text', .2);
  await page.getByRole('button', { name: 'Select', exact: true }).click();

  const object = page.getByRole('button', { name: 'text object 1', exact: true });
  const originalX = await object.getAttribute('x');
  const originalY = await object.getAttribute('y');
  const bounds = (await object.boundingBox())!;
  await page.evaluate(() => window.addEventListener('pointerdown', event => {
    (window as any).__scenePointerId = event.pointerId;
  }, { once: true }));
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 40, bounds.y + bounds.height / 2 + 20);
  await page.keyboard.press('Escape');

  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  const hasPointerCapture = await page.evaluate(() => {
    const pointerId = (window as any).__scenePointerId;
    return (document.querySelector('.scene-canvas-page') as SVGSVGElement).hasPointerCapture(pointerId);
  });
  expect(hasPointerCapture).toBe(false);
  await page.mouse.up();
  await expect(object).toHaveAttribute('x', originalX!);
  await expect(object).toHaveAttribute('y', originalY!);
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('escape-releases-pointer-capture.png') });
});

test('Escape discards inline text edits without changing the committed scene', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .2);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).dblclick();

  const editor = page.getByRole('textbox', { name: 'Text box text' });
  const originalText = await editor.inputValue();
  await editor.fill('Discard this draft');
  await editor.press('Escape');
  await expect(editor).toHaveCount(0);

  const scene = await exported(page);
  expect(scene.pages[0].objects[0].text).toBe(originalText);
});

test('one signature drag stays in one undo step', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Signature', .2);
  await expect(page.locator('.scene-object-hit')).toHaveCount(1);
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Redo \(/ })).toBeEnabled();
});

test('text in a script that needs a pack offers the pack install instead of an export error', async ({ page }) => {
  const message = "Arabic text needs the Unicode scripts pack, which isn't installed. Nothing was saved.";
  await page.addInitScript((message) => {
    (window as any).exportOutcomes = [{ inputPath: '/local/scene-fixture.pdf', outputPaths: [], state: 'failed', detail: '', reason: null, warnings: [],
      failure: { kind: 'unavailable', message, pointer: { kind: 'pack', packId: 'unicode-scripts', name: 'Unicode scripts', reason: 'notInstalled' } } }];
  }, message);
  await openEditor(page);
  await draw(page, 'Text', .3);
  await exported(page);
  const row = page.locator('[data-tone="fail"]').filter({ hasText: 'scene-fixture.pdf' });
  await expect(row).toContainText('Needs a pack');
  await expect(row).toContainText(message);
  await expect(row).not.toContainText('could not be exported safely');
  await expect(row.getByRole('button', { name: /^Retry/ })).toHaveCount(0);
  await row.getByRole('button', { name: 'Install Unicode scripts pack for scene-fixture.pdf', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'install_pack').map((call: any) => call.args.packId))).toEqual(['unicode-scripts']);
});

const MB = 1024 * 1024;
const scriptPacks = (status: string) => ['Arabic', 'Devanagari', 'Han'].map(script => ({ script, packId: 'unicode-scripts', packName: 'Unicode scripts', status, downloadBytes: 38 * MB }));
const emit = (page: Page, event: string, payload: unknown) => page.evaluate(([event, payload]) => (window as any).__emit(event, payload), [event, payload] as const);
async function editText(page: Page, text: string) {
  await draw(page, 'Text', .2);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).dblclick();
  const editor = page.getByRole('textbox', { name: 'Text box text' });
  await editor.fill(text);
  return editor;
}
const scriptPrompt = (page: Page) => page.getByRole('complementary', { name: 'PDF editor inspector' }).getByRole('status').filter({ hasText: "can't be shown yet" });

test('text in a script whose pack is not installed shows the install prompt, which clears once the pack is installed', async ({ page }) => {
  await page.addInitScript((states) => { (window as any).scriptPacks = states; }, scriptPacks('notInstalled'));
  await openEditor(page);
  const editor = await editText(page, 'Received from आशा');
  // The prompt follows the text while it is typed, before export.
  const prompt = scriptPrompt(page);
  await expect(prompt).toContainText("“आशा” can't be shown yet. Devanagari needs the Unicode scripts pack. Until it's installed, this box can't be exported.");
  await expect(prompt).toContainText('38 MB download');
  await editor.press('ControlOrMeta+Enter');
  await expect(editor).toHaveCount(0);
  await expect(prompt).toBeVisible();

  await prompt.getByRole('button', { name: 'Install script pack', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'install_pack').map((call: any) => call.args.packId))).toEqual(['unicode-scripts']);

  // The native install announces the pack; the query then reports it installed.
  await page.evaluate((states) => { (window as any).scriptPacks = states; }, scriptPacks('installed'));
  const previews = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene_pages').length);
  await emit(page, 'pack-changed', { packId: 'unicode-scripts' });
  await expect(prompt).toHaveCount(0);
  // The preview renders again with the pack, so a preview that failed for it doesn't block export.
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'preview_pdf_scene_pages').length)).toBeGreaterThan(previews);
  const scene = await exported(page);
  expect(scene.pages[0].objects[0].text).toBe('Received from आशा');
});

test('text in Latin, Cyrillic and Greek needs no script pack', async ({ page }) => {
  await page.addInitScript((states) => { (window as any).scriptPacks = states; }, scriptPacks('notInstalled'));
  await openEditor(page);
  const editor = await editText(page, 'Hello Привет Γειά σου 2026 — “ok”');
  await expect(editor).toBeVisible();
  await expect(scriptPrompt(page)).toHaveCount(0);
  await editor.press('ControlOrMeta+Enter');
  await expect(editor).toHaveCount(0);
  await expect(scriptPrompt(page)).toHaveCount(0);
  // Script text whose pack is installed needs nothing either.
  await page.evaluate((states) => { (window as any).scriptPacks = states; }, scriptPacks('installed'));
  await emit(page, 'pack-changed', { packId: 'unicode-scripts' });
  await page.getByRole('button', { name: 'text object 1', exact: true }).dblclick();
  await page.getByRole('textbox', { name: 'Text box text' }).fill('مرحبا 你好');
  await expect(scriptPrompt(page)).toHaveCount(0);
});

test('an export that still needs the script pack after the prompt offers the install on its result row', async ({ page }) => {
  await page.addInitScript((states) => {
    (window as any).scriptPacks = states;
    (window as any).exportOutcomes = [{ inputPath: '/local/scene-fixture.pdf', outputPaths: [], state: 'failed', detail: '', reason: null, warnings: [],
      failure: { kind: 'unavailable', message: "Devanagari text needs the Unicode scripts pack, which isn't installed. Nothing was saved.", pointer: { kind: 'pack', packId: 'unicode-scripts', name: 'Unicode scripts', reason: 'notInstalled' } } }];
  }, scriptPacks('installed'));
  await openEditor(page);
  const editor = await editText(page, 'Received from आशा');
  await editor.press('ControlOrMeta+Enter');
  await expect(scriptPrompt(page)).toHaveCount(0);
  await exported(page);
  const row = page.locator('[data-tone="fail"]').filter({ hasText: 'scene-fixture.pdf' });
  await row.getByRole('button', { name: 'Install Unicode scripts pack for scene-fixture.pdf', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'install_pack').map((call: any) => call.args.packId))).toEqual(['unicode-scripts']);
});

for (const theme of ['dark', 'light'] as const) {
  test(`the script-pack prompt renders in the ${theme} theme`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.addInitScript(({ theme, states }) => { localStorage.setItem('toolbox-theme', theme); (window as any).scriptPacks = states; }, { theme, states: scriptPacks('notInstalled') });
    await openEditor(page);
    await editText(page, 'Received from आशा');
    await expect(page.locator('body')).toHaveAttribute('data-theme', theme);
    const prompt = scriptPrompt(page);
    await expect(prompt).toBeVisible();
    const inspector = (await page.getByRole('region', { name: 'Properties' }).boundingBox())!;
    for (const part of [prompt, prompt.getByRole('button', { name: 'Install script pack' }), prompt.getByText('38 MB download')]) {
      const box = (await part.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(inspector.x);
      expect(box.x + box.width).toBeLessThanOrEqual(inspector.x + inspector.width - 16);
    }
    await page.screenshot({ path: testInfo.outputPath(`script-pack-prompt-${theme}.png`), animations: 'disabled' });
  });
}

const hitX = (page: Page, index = 0) => page.locator('.scene-object-hit').nth(index).evaluate((rect) => Number(rect.getAttribute('x')));
const summary = (page: Page) => page.getByRole('list', { name: 'Changes, applied in this fixed order' }).getByRole('listitem');

test('arrow and Shift+arrow nudge by recorded 1 pt and 10 pt steps', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).focus();
  await expect(page.getByText('Arrow keys nudge it, Shift+arrow nudges further.')).toBeVisible();
  const start = await hitX(page);
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => hitX(page)).toBeCloseTo(start + 1, 5);
  await page.keyboard.press('Shift+ArrowRight');
  await expect.poll(() => hitX(page)).toBeCloseTo(start + 11, 5);
  await page.keyboard.press('ControlOrMeta+z');
  await expect.poll(() => hitX(page)).toBeCloseTo(start + 1, 5);
  await page.keyboard.press('ControlOrMeta+z');
  await expect.poll(() => hitX(page)).toBeCloseTo(start, 5);
});

test('duplicate copies the selected object with a shortcut or button and is undoable', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Shape', .3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'shape object 1', exact: true }).focus();
  const start = await hitX(page);
  await page.keyboard.press('ControlOrMeta+d');
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);
  expect(await hitX(page, 1)).toBeCloseTo(start + 10, 5);
  await expect(page.getByRole('button', { name: 'shape object 2', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(3);
  await page.keyboard.press('ControlOrMeta+z');
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);
  await page.keyboard.press('ControlOrMeta+z');
  await expect(page.locator('.scene-object-hit')).toHaveCount(1);
  const scene = await exported(page);
  expect(scene.pages[0].objects).toHaveLength(1);
});

test('the object list names the type, page and text of every placed object', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .2);
  await draw(page, 'Shape', .5);
  await page.getByRole('button', { name: 'Next page' }).click();
  await draw(page, 'Highlight', .6);
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Signature', exact: true }).click();
  await draw(page, 'Signature', .3);
  // The signature tool adds its own "Use again" list to the inspector, so target the object list by its section.
  const list = page.getByRole('complementary', { name: 'PDF editor inspector' }).getByRole('region', { name: /^Objects in this document/ }).getByRole('list');
  await expect(list.getByRole('button')).toHaveText([/Page 1Text/, /Page 1square/, /Page 2highlighted area/, /Page 2Your signature/]);
  for (const name of ['Text box, page 1: Text', 'Shape, page 1: square', 'Highlight, page 2: highlighted area', 'Signature, page 2: Your signature'])
    await expect(list.getByRole('button', { name, exact: true })).toBeVisible();
  await list.getByRole('button', { name: 'Text box, page 1: Text', exact: true }).click();
  await expect(page.getByText('Page 1 of 3')).toBeVisible();
  await expect(page.getByRole('button', { name: 'text object 1', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(list.getByRole('button', { name: 'Text box, page 1: Text', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('undo and redo bindings work across sub-tools and switching keeps the edits', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .2);
  await draw(page, 'Shape', .5);
  const tools = page.getByRole('toolbar', { name: 'PDF editor tools' });
  await tools.getByRole('button', { name: 'Watermark', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);
  await tools.getByRole('button', { name: 'Crop', exact: true }).click();
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);
  const hits = page.locator('.scene-object-hit');
  for (const [key, count] of [['Meta+z', 1], ['Meta+Shift+z', 2], ['Control+z', 1], ['Control+Shift+z', 2], ['Control+z', 1], ['Meta+Shift+z', 2]] as const) {
    await page.keyboard.press(key);
    await expect(hits, key).toHaveCount(count);
  }
  await expect(tools.getByRole('button', { name: 'Crop', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await tools.getByRole('button', { name: 'Text', exact: true }).click();
  await page.keyboard.press('ControlOrMeta+z');
  await page.keyboard.press('ControlOrMeta+z');
  await expect(hits).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Redo' })).toBeEnabled();
  await page.getByRole('button', { name: 'Redo' }).click();
  await expect(hits).toHaveCount(1);
});

test('the export summary lists contributing sub-tools in the fixed order as edits change', async ({ page }) => {
  await openEditor(page);
  await expect(page.getByText('No changes to export yet.')).toBeVisible();
  await draw(page, 'Shape', .5);
  await expect(summary(page)).toHaveText(['Shape']);
  await draw(page, 'Text', .2);
  await expect(summary(page)).toHaveText(['Text', 'Shape']);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate selected pages' }).click();
  await expect(summary(page)).toHaveText(['Text', 'Shape', 'Watermark', 'Organize pages']);
  await page.keyboard.press('ControlOrMeta+z');
  await page.keyboard.press('ControlOrMeta+z');
  await expect(summary(page)).toHaveText(['Text', 'Shape']);
});

test('unexported edits prompt before leaving the view or switching files', async ({ page }) => {
  await openEditor(page);
  const status = page.locator('.scene-original-note');
  await expect(status).toHaveText('Original unchanged');
  await draw(page, 'Shape', .5);
  await expect(status).toHaveText('1 change not exported · Original unchanged');
  await draw(page, 'Text', .2);
  await expect(status).toHaveText('2 changes not exported · Original unchanged');
  const prompt = page.getByRole('alertdialog', { name: 'Export your changes before closing?' });

  await page.getByRole('button', { name: '← All tools' }).click();
  await expect(prompt).toContainText("scene-fixture.pdf has 2 changes that haven't been exported. If you close now, they're lost.");
  await prompt.getByRole('button', { name: 'Cancel' }).click();
  await expect(prompt).toHaveCount(0);
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);

  await page.keyboard.press('ControlOrMeta+k');
  await expect(prompt).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(prompt).toHaveCount(0);
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);

  await page.evaluate(() => { (window as any).dialogNext = '/local/other.pdf'; });
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(prompt).toBeVisible();
  await expect(page.locator('[data-shortcut="run"]')).toBeEnabled();
  await prompt.getByRole('button', { name: 'Export…' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').length)).toBe(1);
  await expect(status).toHaveText('Original unchanged');
  await expect(page.locator('.scene-object-hit')).toHaveCount(2);

  await draw(page, 'Shape', .7);
  await expect(status).toHaveText('1 change not exported · Original unchanged');
  await page.keyboard.press('Escape');
  await expect(prompt).toBeVisible();
  await prompt.getByRole('button', { name: 'Discard changes' }).click();
  await expect(page.getByRole('textbox', { name: 'Search tools' })).toBeVisible();
});

test('replacing an exported file does not prompt and discarding switches files', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Shape', .5);
  await page.evaluate(() => { (window as any).dialogNext = '/local/other.pdf'; });
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  const prompt = page.getByRole('alertdialog', { name: 'Export your changes before closing?' });
  await prompt.getByRole('button', { name: 'Discard changes' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'inspect_pdf_scene').map((c: any) => c.args.request.path).at(-1))).toBe('/local/other.pdf');
  await expect(page.locator('.scene-object-hit')).toHaveCount(0);
  await page.getByRole('button', { name: '← All tools' }).click();
  await expect(prompt).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Search tools' })).toBeVisible();
});

for (const { platform, undo, redo, duplicate } of [
  { platform: 'MacIntel', undo: '⌘Z', redo: '⇧⌘Z', duplicate: '⌘D' },
  { platform: 'Win32', undo: 'Ctrl+Z', redo: 'Ctrl+Y', duplicate: 'Ctrl+D' },
]) test(`PDF editor shortcut labels follow ${platform === 'Win32' ? 'Windows' : 'macOS'}`, async ({ page }) => {
  await page.addInitScript((platform) => Object.defineProperty(window.navigator, 'platform', { configurable: true, value: platform }), platform);
  await openEditor(page);
  await expect(page.getByRole('button', { name: 'Undo' })).toHaveAttribute('title', `Undo (${undo})`);
  await expect(page.getByRole('button', { name: 'Redo' })).toHaveAttribute('title', `Redo (${redo})`);
  await draw(page, 'Shape', .5);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'shape object 1', exact: true }).focus();
  await expect(page.getByRole('button', { name: 'Duplicate', exact: true }).locator('kbd')).toHaveText(duplicate);
});

for (const theme of ['dark', 'light'] as const) test(`object list, export summary and the unexported prompt render in ${theme}`, async ({ page }, testInfo) => {
  await page.addInitScript(theme => localStorage.setItem('toolbox-theme', theme), theme);
  await openEditor(page);
  await expect(page.locator('body')).toHaveAttribute('data-theme', theme);
  await draw(page, 'Text', .2);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'Text box, page 1: Text', exact: true }).click();
  await expect(summary(page)).toHaveText(['Text']);
  const colors = await page.locator('.scene-inspector-heading').first().evaluate((heading) => {
    const panel = heading.closest('.scene-inspector-panel')!;
    return [getComputedStyle(heading).color, getComputedStyle(panel).backgroundColor];
  });
  expect(colors[0]).not.toBe(colors[1]);
  await page.screenshot({ path: testInfo.outputPath(`composition-${theme}.png`) });
  await page.getByRole('button', { name: '← All tools' }).click();
  const prompt = page.getByRole('alertdialog', { name: 'Export your changes before closing?' });
  await expect(prompt).toBeVisible();
  const promptColors = await prompt.evaluate((dialog) => [getComputedStyle(dialog).color, getComputedStyle(dialog).backgroundColor]);
  expect(promptColors[0]).not.toBe(promptColors[1]);
  await page.screenshot({ path: testInfo.outputPath(`unexported-prompt-${theme}.png`) });
});

test('scene export sends only the scene sections and the invoke boundary accepts the staged request shape', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .2);
  await exported(page);
  const request = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request);
  expect(Object.keys(request).sort()).toEqual(['outputLocation', 'paths', 'scene']);

  const staged = { ...request, formValues: [], textEdits: [], flatten: 'both', redactions: [] };
  const results = await page.evaluate((staged) => (window as any).__TAURI_INTERNALS__.invoke('export_pdf_scene', { request: staged }), staged);
  expect(results[0].state).toBe('succeeded');
  const accepted = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request);
  expect(accepted).toEqual(staged);
});

test('redaction marks are added, reviewed, recoloured and removed, and export waits for the disclosure', async ({ page }, testInfo) => {
  await openEditor(page);
  await page.evaluate(() => {
    const w = window as any;
    w.exportOutput = '/local/scene-fixture-redacted.pdf';
    w.exportDetail = 'Pages 1 and 3 were redacted as 300 dpi images. Document metadata, XMP data and attachments were removed.';
  });
  const inspector = page.getByRole('complementary', { name: 'PDF editor inspector' });
  const marks = inspector.getByRole('list', { name: 'Marks' });
  await draw(page, 'Redact', .2);
  await draw(page, 'Redact', .5);
  await page.getByRole('button', { name: 'Page 3' }).click();
  await draw(page, 'Redact', .3);
  // The property card is titled for the tool; the export summary below it also lists Redact as a change.
  await expect(inspector.locator('[data-slot="card-title"]').filter({ hasText: /^Redact$/ })).toBeVisible();
  await expect(marks.getByRole('button')).toHaveText(['Page 1, area 1', 'Page 1, area 2', 'Page 3, area 1']);
  await expect(page.locator('.scene-redaction')).toHaveCount(1);
  await expect(page.locator('.scene-redaction')).not.toHaveAttribute('data-preview', 'true');

  await marks.getByRole('button', { name: 'Page 1, area 2' }).click();
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).toHaveAttribute('aria-current', 'page');
  await inspector.getByRole('button', { name: 'Remove mark' }).click();
  await expect(marks.getByRole('button')).toHaveText(['Page 1, area 1', 'Page 3, area 1']);
  await inspector.getByRole('radio', { name: 'White' }).click();
  await expect(inspector.getByRole('radio', { name: 'White' })).toHaveAttribute('aria-checked', 'true');

  await inspector.getByRole('button', { name: 'Review marks' }).click();
  await expect(inspector.getByRole('status').filter({ hasText: 'Mark 1 of 2' })).toHaveText('Mark 1 of 2: page 1, area 1');
  await expect(page.locator('.scene-redaction')).toHaveAttribute('data-preview', 'true');
  await expect(page.locator('.scene-redaction')).toHaveCSS('fill', 'rgb(255, 255, 255)');
  await inspector.getByRole('button', { name: 'Next' }).click();
  await expect(inspector.getByText('Mark 2 of 2: page 3, area 1')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('pdf-redact-reviewing.png') });
  await expect(page.getByRole('button', { name: 'Page 3', exact: true })).toHaveAttribute('aria-current', 'page');
  await page.getByRole('button', { name: 'Redaction mark 1' }).focus();
  await page.keyboard.press('Tab');
  await expect(inspector.getByText('Mark 1 of 2: page 1, area 1')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Redaction mark 1' })).toBeFocused();
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).toHaveAttribute('aria-current', 'page');
  await inspector.getByRole('checkbox', { name: 'Preview as exported' }).click();
  await expect(page.locator('.scene-redaction')).not.toHaveAttribute('data-preview', 'true');
  await inspector.getByRole('button', { name: 'Done reviewing' }).click();
  await expect(inspector.getByRole('button', { name: 'Review marks' })).toBeVisible();

  const exportButton = page.getByRole('button', { name: 'Export PDF', exact: true });
  await expect(exportButton).toBeEnabled();
  await exportButton.click();
  const dialog = page.getByRole('dialog', { name: 'Before you export with redactions' });
  await expect(dialog).toContainText('2 marks on pages 1 and 3 will be removed for good in the copy. To make sure nothing can be recovered:');
  await expect(dialog.getByRole('listitem')).toHaveText([
    "Pages 1 and 3 become 300 dpi images. Their text can't be selected or searched.",
    'Comments, form fields and hidden layers on those pages are removed.',
    'Document metadata, XMP data and attachments are removed from the whole copy.',
  ]);
  const continueExport = dialog.getByRole('button', { name: 'Continue to export' });
  await expect(continueExport).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath('pdf-redact-disclosure.png') });
  await dialog.getByRole('button', { name: 'Review marks' }).click();
  await expect(dialog).toBeHidden();
  await expect(inspector.getByText('Mark 1 of 2: page 1, area 1')).toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(0);

  await exportButton.click();
  await dialog.getByRole('checkbox', { name: "I've checked the marks on every page" }).click();
  await continueExport.click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(1);
  const request = await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'export_pdf_scene').args.request);
  expect(request.scene.pages.every((item: any) => !('redactions' in item))).toBe(true);
  expect(request.redactions.map(({ pageIndex, color }: any) => [pageIndex, color])).toEqual([[0, '#ffffff'], [2, '#ffffff']]);
  for (const [mark, y] of [[request.redactions[0], .2], [request.redactions[1], .3]] as const) {
    expect(mark.rect.x).toBeCloseTo(612 * .2, 0);
    expect(mark.rect.width).toBeCloseTo(612 * .35, 0);
    expect(mark.rect.y).toBeCloseTo(792 * y, 0);
    expect(mark.rect.height).toBeCloseTo(792 * .07, 0);
  }
  await expect(page.getByText('Pages 1 and 3 were redacted as 300 dpi images. Document metadata, XMP data and attachments were removed.')).toBeVisible();
});

test('exporting without redaction marks skips the disclosure', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Shape', .4);
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(1);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await page.evaluate(() => 'redactions' in (window as any).calls.find((call: any) => call.command === 'export_pdf_scene').args.request)).toBe(false);
});

const textFields = (object: any) => ({ fontFamily: object.fontFamily, fontSize: object.fontSize, bold: object.bold, italic: object.italic, align: object.align,
  color: object.color, opacity: object.opacity, pageRange: object.pageRange ?? null });

test('text controls set the next box and the selected box exactly', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Text', exact: true }).click();
  const inspector = page.getByRole('complementary', { name: 'PDF editor inspector' });
  await expect(inspector.getByText('Click the page to add a text box. Double-click a box or press Enter to edit it.')).toBeVisible();
  await inspector.getByRole('combobox', { name: 'Font' }).selectOption('Noto Sans Mono');
  await inspector.getByRole('button', { name: 'Bold', exact: true }).click();
  await inspector.getByRole('radio', { name: 'Align right' }).click();
  await draw(page, 'Text', .3);
  let result = await exported(page);
  expect(textFields(result.pages[0].objects[0])).toEqual({ fontFamily: 'Noto Sans Mono', fontSize: 18, bold: true, italic: false, align: 'right', color: '#1b1b1b', opacity: 1, pageRange: null });

  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).click();
  await inspector.getByRole('combobox', { name: 'Font' }).selectOption('Noto Serif');
  await inspector.getByRole('spinbutton', { name: 'Size' }).fill('24');
  await inspector.getByRole('button', { name: 'Bold', exact: true }).click();
  await inspector.getByRole('button', { name: 'Italic', exact: true }).click();
  await inspector.getByRole('radio', { name: 'Align centre' }).click();
  await expect(inspector.getByRole('radio', { name: 'Align centre' })).toHaveAttribute('aria-checked', 'true');
  await page.locator('input[type="color"][aria-label="Object color"]').fill('#aa3300');
  await page.getByRole('slider', { name: 'Object opacity' }).press('Home');
  result = await exported(page);
  expect(textFields(result.pages[0].objects[0])).toEqual({ fontFamily: 'Noto Serif', fontSize: 24, bold: false, italic: true, align: 'center', color: '#aa3300', opacity: .05, pageRange: null });
});

test('a text box can be shown on a page range', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).click();
  const pages = page.getByRole('textbox', { name: 'Pages for this text box' });
  await expect(page.getByRole('radio', { name: 'This page only' })).toBeChecked();
  await expect(pages).toBeDisabled();
  await page.getByRole('radio', { name: 'These pages' }).click();
  await expect(pages).toHaveValue('1');
  await pages.fill('1-2, 3-');
  await pages.press('Enter');
  let result = await exported(page);
  expect(result.pages[0].objects[0].pageRange).toBe('1-2, 3-');
  expect(result.pages.slice(1).map((item: any) => item.objects.length)).toEqual([0, 0]);
  await page.getByRole('radio', { name: 'This page only' }).click();
  result = await exported(page);
  expect(result.pages[0].objects[0].pageRange).toBeNull();
});

test('double-click and Enter open the text box editor in place', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  const box = page.getByRole('button', { name: 'text object 1', exact: true });
  await box.dblclick();
  const editor = page.getByRole('textbox', { name: 'Text box text' });
  await expect(editor).toBeFocused();
  await editor.fill('Привет\nΚαλημέρα');
  await editor.press('Control+Enter');
  await expect(editor).toHaveCount(0);
  await box.focus();
  await box.press('Enter');
  await expect(editor).toBeFocused();
  await expect(editor).toHaveValue('Привет\nΚαλημέρα');
  await editor.fill('Hello, world');
  await editor.press('Control+Enter');
  const result = await exported(page);
  expect(result.pages[0].objects[0].text).toBe('Hello, world');
});

test('unrenderable text names the characters and keeps Export disabled', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Text', .3);
  await page.evaluate(() => {
    (window as any).previewFailure = true;
    (window as any).previewFailureMessage = 'Noto Sans can’t show these characters: “😀” (U+1F600), “आ” (U+0906). Remove them to export this text box.';
  });
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'text object 1', exact: true }).dblclick();
  await page.getByRole('textbox', { name: 'Text box text' }).fill('Hi 😀 आ');
  await page.getByRole('textbox', { name: 'Text box text' }).press('Control+Enter');
  await expect(page.getByRole('alert')).toHaveText('Preview could not be verified. Noto Sans can’t show these characters: “😀” (U+1F600), “आ” (U+0906). Remove them to export this text box. Export is disabled until the preview can be verified.');
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

// Mirrors WatermarkOptions in src/contracts.ts, shared with Edit Image.
const headerFooterPart = (page: Page, part: string) => page.locator(`.scene-canvas [data-header-footer-part="${part}"]`);
const tokenField = (page: Page, name: string) => page.getByRole('textbox', { name, exact: true });
async function addHeaderFooter(page: Page) {
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Header and footer', exact: true }).click();
  await page.getByRole('button', { name: 'Add header and footer', exact: true }).click();
  await expect(page.getByText('Header and footer added to all 3 pages.')).toBeVisible();
}
const blankBand = { left: '', center: '', right: '' };

test('header and footer text uses token tags, and every option is sent exactly to preview and export', async ({ page }) => {
  await openEditor(page);
  await page.evaluate(() => { (window as any).previewDelay = 60_000; });
  await addHeaderFooter(page);
  const sent = async () => (await previewScene(page)).pages[0].objects[0];
  await expect.poll(sent).toMatchObject({
    kind: 'headerFooter', text: '', fontSize: 8.5, color: '#555555', opacity: 1, fontFamily: 'Noto Sans', layer: 'front', pageRange: 'all',
    headerFooter: { header: blankBand, footer: { ...blankBand, center: '{page} of {pages}' }, firstPage: null, even: null, fileName: 'scene-fixture.pdf', margin: 36 },
  });
  expect((await sent()).headerFooter.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  await expect(tokenField(page, 'Footer centre').locator('[data-token]')).toHaveText(['Page number', 'Page count']);
  await expect(headerFooterPart(page, 'footer-center')).toHaveText('1 of 3');
  await expect(page.getByRole('button', { name: 'Footer', exact: true })).toBeVisible();

  await tokenField(page, 'Header left').click();
  await page.keyboard.type('Acme ');
  await page.getByRole('group', { name: 'Insert' }).getByRole('button', { name: 'File name' }).click();
  await expect(tokenField(page, 'Header left').locator('[data-token]')).toHaveText(['File name']);
  await expect(headerFooterPart(page, 'header-left')).toHaveText('Acme scene-fixture.pdf');
  await tokenField(page, 'Header right').click();
  await page.getByRole('group', { name: 'Insert' }).getByRole('button', { name: 'Date' }).click();
  await page.locator('#scene-header-footer-date').fill('2026-10-13');
  await expect(headerFooterPart(page, 'header-right')).toHaveText('13 October 2026');
  await tokenField(page, 'Header right').click();
  await page.keyboard.press('End');
  await page.keyboard.press('Backspace');
  await expect(tokenField(page, 'Header right').locator('[data-token]')).toHaveCount(0);
  await expect(headerFooterPart(page, 'header-right')).toHaveCount(0);
  await page.keyboard.type('Draft ');
  await page.getByRole('group', { name: 'Insert' }).getByRole('button', { name: 'Page number' }).click();
  await page.keyboard.press('Enter');
  await expect(headerFooterPart(page, 'header-right')).toHaveText('Draft 1');

  await page.getByRole('combobox', { name: 'Size' }).selectOption('10');
  await page.getByRole('combobox', { name: 'Distance from edge' }).selectOption('54');
  await page.getByRole('combobox', { name: 'Font' }).selectOption('Noto Serif');
  await page.locator('#scene-header-footer-color').fill('#112233');
  await page.getByRole('radio', { name: 'Skip the first page' }).click();
  await expect(page.getByText('Header and footer added to 2 of 3 pages.')).toBeVisible();
  await expect(headerFooterPart(page, 'footer-center')).toHaveCount(0);
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(headerFooterPart(page, 'footer-center')).toHaveText('2 of 3');
  await expect(headerFooterPart(page, 'header-right')).toHaveText('Draft 2');
  await page.evaluate(() => { (window as any).previewDelay = 0; });

  const scene = await exported(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages.slice(1).map((item: any) => item.objects)).toEqual([[], []]);
  const object = scene.pages[0].objects[0];
  expect(object).toMatchObject({ kind: 'headerFooter', fontSize: 10, color: '#112233', fontFamily: 'Noto Serif', pageRange: '2-' });
  expect(object).not.toHaveProperty('headerFooterParts');
  expect(object.headerFooter).toEqual({
    header: { left: 'Acme {file}', center: '', right: 'Draft {page}' }, footer: { ...blankBand, center: '{page} of {pages}' },
    firstPage: null, even: null, date: '2026-10-13', fileName: 'scene-fixture.pdf', margin: 54,
  });
});

test('header and footer first-page and even-page text apply by final page, and overlaps with page text are flagged', async ({ page }) => {
  await openEditor(page);
  await addHeaderFooter(page);
  await page.evaluate(() => { (window as any).previewDelay = 60_000; });
  await page.getByRole('checkbox', { name: 'Different first page' }).click();
  await expect(page.getByRole('radio', { name: 'First page', exact: true })).toHaveAttribute('aria-checked', 'true');
  await tokenField(page, 'Footer left').click();
  await page.keyboard.type('Cover');
  await expect(headerFooterPart(page, 'footer-left')).toHaveText('Cover');
  await page.getByRole('checkbox', { name: 'Different odd and even pages' }).click();
  await expect(page.getByRole('radio', { name: 'Even pages', exact: true })).toHaveAttribute('aria-checked', 'true');
  await expect(tokenField(page, 'Footer left')).toHaveText('');
  await tokenField(page, 'Header right').click();
  await page.keyboard.type('Even');
  await page.getByRole('radio', { name: 'Odd pages', exact: true }).click();
  await tokenField(page, 'Header left').click();
  await page.keyboard.type('Company');
  await expect(headerFooterPart(page, 'header-left')).toHaveCount(0);

  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(headerFooterPart(page, 'header-right')).toHaveText('Even');
  await expect(headerFooterPart(page, 'header-left')).toHaveCount(0);
  await expect(headerFooterPart(page, 'footer-left')).toHaveCount(0);
  await page.getByRole('button', { name: 'Page 3', exact: true }).click();
  await expect(headerFooterPart(page, 'header-left')).toHaveText('Company');
  await expect(headerFooterPart(page, 'header-right')).toHaveCount(0);

  await page.getByRole('button', { name: 'Header', exact: true }).click();
  const overlap = page.getByRole('alert').filter({ hasText: 'overlaps existing text' });
  await expect(overlap).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Distance from edge' }).selectOption('54');
  await expect(overlap).toBeVisible();
  await page.getByRole('combobox', { name: 'Distance from edge' }).selectOption('36');
  await expect(overlap).toHaveCount(0);
  await page.evaluate(() => { (window as any).previewDelay = 0; });

  let scene = await exported(page);
  expect(scene.pages[0].objects[0].headerFooter).toMatchObject({
    header: { ...blankBand, left: 'Company' }, footer: { ...blankBand, center: '{page} of {pages}' },
    firstPage: { header: blankBand, footer: { left: 'Cover', center: '{page} of {pages}', right: '' } },
    even: { header: { ...blankBand, right: 'Even' }, footer: { ...blankBand, center: '{page} of {pages}' } },
  });
  await page.getByRole('checkbox', { name: 'Different first page' }).click();
  await expect(page.getByRole('radio', { name: 'First page', exact: true })).toHaveCount(0);
  scene = await exported(page);
  expect(scene.pages[0].objects[0].headerFooter.firstPage).toBeNull();
  await page.getByRole('button', { name: 'Remove header and footer' }).click();
  await expect(page.getByRole('button', { name: 'Header', exact: true })).toHaveCount(0);
});

const sharedWatermarkOptions = ['text', 'logoPath', 'size', 'opacity', 'color', 'position', 'rotation'];
const watermarkHit = (page: Page) => page.getByRole('button', { name: /^Watermark / });
async function previewScene(page: Page) {
  return page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'preview_pdf_scene_pages').at(-1).args.request.scene);
}

test('a watermark is stored once with its range and every shared option, and sent unchanged to preview and export', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await expect(page.getByRole('slider', { name: 'Object opacity' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await expect(page.getByText('Watermark added to all 3 pages.')).toBeVisible();
  await page.getByRole('textbox', { name: 'Text', exact: true }).fill('PRIVATE');
  await page.getByRole('combobox', { name: 'Font' }).selectOption('Noto Serif|bold');
  await page.locator('#scene-watermark-color').fill('#0000ff');
  await page.getByRole('slider', { name: 'Opacity' }).focus();
  await page.keyboard.press('End');
  await page.getByRole('combobox', { name: 'Angle' }).selectOption('custom');
  await page.getByRole('spinbutton', { name: 'Degrees' }).fill('30');
  await page.getByRole('combobox', { name: 'Position' }).selectOption('tile');
  await page.getByRole('combobox', { name: 'Size' }).selectOption('0.2');
  await page.getByRole('radio', { name: 'Above content' }).click();
  await page.getByRole('radio', { name: 'Skip the first page' }).click();
  await expect(page.getByText('Watermark added to 2 of 3 pages.')).toBeVisible();
  await expect(watermarkHit(page)).toHaveCount(0);
  await expect.poll(async () => (await previewScene(page)).pages[0].objects[0]?.pageRange).toBe('2-');
  await page.getByRole('radio', { name: 'These pages' }).click();
  const pages = page.getByRole('textbox', { name: 'Pages to include' });
  await pages.fill('2-9');
  await pages.press('Enter');
  await expect(page.getByText('Watermark added to 2 of 3 pages.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
  const expected = {
    kind: 'watermark', fontFamily: 'Noto Serif', bold: true, layer: 'front', pageRange: '2-9',
    watermark: { text: 'PRIVATE', opacity: 100, color: '#0000ff', position: 'tile', rotation: 30, size: 0.2 },
  };
  for (const scene of [await previewScene(page), await exported(page)]) {
    expect(scene.pages[0].objects).toHaveLength(1);
    expect(scene.pages[0].objects[0]).toMatchObject(expected);
    expect(scene.pages[0].objects[0].rect).toEqual({ x: 0, y: 0, width: 612, height: 792 });
    expect(scene.pages.slice(1).map((item: any) => item.objects)).toEqual([[], []]);
    expect(Object.keys(scene.pages[0].objects[0].watermark).every((key) => sharedWatermarkOptions.includes(key))).toBe(true);
  }
});

test('a range-applied watermark shows and edits on every affected page only', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await page.getByRole('radio', { name: 'These pages' }).click();
  const pages = page.getByRole('textbox', { name: 'Pages to include' });
  await pages.fill('odd-2');
  await pages.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('“odd-2” is not a page list. Use page numbers and ranges like 1-3, 7, 10-, or all, odd or even.');
  await pages.fill('odd');
  await pages.press('Enter');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).toContainText('1 marks');
  await expect(page.getByRole('button', { name: 'Page 2', exact: true })).not.toContainText('marks');
  await expect(page.getByRole('button', { name: 'Page 3', exact: true })).toContainText('1 marks');
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  await expect(watermarkHit(page)).toHaveCount(0);
  await page.evaluate(() => { (window as any).previewDelay = 60_000; });
  await page.getByRole('button', { name: 'Page 3', exact: true }).click();
  await expect(watermarkHit(page)).toHaveCount(1);
  await watermarkHit(page).focus();
  await page.getByRole('textbox', { name: 'Text', exact: true }).fill('COPY');
  // The draft follows the shared placement geometry until the rendered preview is ready.
  await expect(page.locator('.scene-canvas text', { hasText: 'COPY' })).toHaveCount(1);
  await page.evaluate(() => { (window as any).previewDelay = 0; });
  await page.getByRole('button', { name: 'Page 2', exact: true }).click();
  const scene = await exported(page);
  expect(scene.pages[0].objects).toHaveLength(1);
  expect(scene.pages[0].objects[0]).toMatchObject({ pageRange: 'odd', watermark: { text: 'COPY' } });
  expect(scene.pages[2].objects).toEqual([]);
  await page.getByRole('button', { name: 'Page 3', exact: true }).click();
  await watermarkHit(page).focus();
  await page.keyboard.press('Shift+ArrowRight');
  const moved = (await exported(page)).pages[0].objects[0].watermark.position;
  expect(moved).toEqual({ x: expect.any(Number), y: expect.any(Number) });
  await watermarkHit(page).focus();
  await page.keyboard.press('Delete');
  await expect(watermarkHit(page)).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Page 1', exact: true })).not.toContainText('marks');
});

test('a picture watermark replaces the text options and an oversized picture is refused before placement', async ({ page }) => {
  await openEditor(page);
  await page.getByRole('button', { name: 'Watermark', exact: true }).click();
  await page.getByRole('button', { name: 'Add watermark', exact: true }).click();
  await page.evaluate(() => { (window as any).dialogNext = '/local/huge.png'; });
  await page.getByRole('radio', { name: 'Picture' }).click();
  await expect(page.getByRole('alert')).toHaveText('This picture is larger than 20 megapixels, the limit for a watermark. Use a smaller picture.');
  await expect(page.getByRole('radio', { name: 'Text' })).toHaveAttribute('aria-checked', 'true');
  await expect.poll(async () => { const options = (await previewScene(page)).pages[0].objects[0]?.watermark; return [options?.text, options?.logoPath]; }).toEqual(['DRAFT', undefined]);
  await page.evaluate(() => { (window as any).dialogNext = '/local/logo.png'; });
  await page.getByRole('radio', { name: 'Picture' }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Size' })).toHaveValue('original');
  await expect(page.getByRole('textbox', { name: 'Text', exact: true })).toHaveCount(0);
  const watermark = (await exported(page)).pages[0].objects[0];
  expect(watermark.watermark).toEqual({ logoPath: '/local/logo.png', opacity: 18, position: 'center', rotation: -45 });
  expect(watermark.watermarkImageSize).toEqual({ width: 400, height: 200 });
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'inspect_pdf_watermark_image').map((c: any) => c.args.request.path))).toEqual(['/local/huge.png', '/local/logo.png']);
});

async function openEditText(page: Page) {
  await openEditor(page);
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Edit text', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Properties' })).toContainText('Click text with a dashed outline to change it.');
}
async function replaceRun(page: Page, replacement: string) {
  await page.getByRole('button', { name: 'Edit text: Rent £1,450' }).click();
  const input = page.getByRole('textbox', { name: 'Replacement text' });
  await expect(input).toHaveValue('Rent £1,450');
  await input.fill(replacement);
  await input.press('Enter');
  await expect(input).toHaveCount(0);
}

test('Edit text outlines editable runs and marks picture text as not editable', async ({ page }) => {
  await openEditText(page);
  const editable = page.locator('[data-edit-run="0"]');
  const blocked = page.locator('[data-edit-run="1"]');
  await expect(editable).toHaveClass('scene-edit-run');
  await expect(editable).toHaveAttribute('aria-label', 'Edit text: Rent £1,450');
  await expect(blocked).toHaveClass('scene-edit-run-blocked');
  await expect(blocked.locator('title')).toHaveText("This text is part of a picture, so it can't be edited.");

  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Select', exact: true }).click();
  await expect(page.locator('[data-edit-run]')).toHaveCount(0);
});

test('not-editable text explains why and offers Add text instead', async ({ page }) => {
  await openEditText(page);
  await page.locator('[data-edit-run="1"]').click();
  await expect(page.getByRole('textbox', { name: 'Replacement text' })).toHaveCount(0);
  const alert = page.getByRole('region', { name: 'Properties' }).getByRole('alert');
  await expect(alert).toContainText("This text can't be edited.");
  await expect(alert).toContainText('It is part of a scanned picture, not real text.');
  await alert.getByRole('button', { name: 'Cover it with Add text instead' }).click();
  await expect(page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Text', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

test('a text edit is sent as textEdits to the preview and the export, and can be reverted', async ({ page }) => {
  await openEditText(page);
  await replaceRun(page, 'Rent £1,475');
  await expect(page.locator('[data-edit-run="0"]')).toHaveAttribute('aria-label', 'Edit text: Rent £1,475');
  const inspector = page.getByRole('region', { name: 'Properties' });
  await expect(inspector.getByRole('group', { name: 'Selected text' })).toContainText('Helvetica · 12 pt');
  await expect(inspector.getByRole('group', { name: 'Edits on this document' })).toContainText('Page 1: “Rent £1,450” to “Rent £1,475”');
  await expect(page.locator('.scene-edit-overflow')).toHaveCount(0);

  const edit = { pageIndex: 0, run: 0, original: 'Rent £1,450', replacement: 'Rent £1,475' };
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'preview_pdf_scene_pages').at(-1).args.request.textEdits)).toEqual([edit]);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  const request = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request);
  expect(Object.keys(request).sort()).toEqual(['outputLocation', 'paths', 'scene', 'textEdits']);
  expect(request.textEdits).toEqual([edit]);
  expect(request.scene).toEqual({ pages: expect.any(Array) });

  await inspector.getByRole('button', { name: 'Revert this text' }).click();
  await expect(inspector.getByRole('group', { name: 'Edits on this document' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: /^Undo \(/ }).click();
  await expect(inspector.getByRole('group', { name: 'Edits on this document' })).toBeVisible();
});

test('a replacement the embedded font cannot draw notes the substitute font and the export row shows font substituted', async ({ page }) => {
  await openEditText(page);
  await replaceRun(page, 'Rent £1,475');
  const selected = page.getByRole('region', { name: 'Properties' }).getByRole('group', { name: 'Selected text' });
  await expect(selected).toContainText('Edited text keeps this font, size and colour.');
  await expect(selected.getByRole('note')).toHaveCount(0);

  await page.locator('[data-edit-run="0"]').click();
  await page.getByRole('textbox', { name: 'Replacement text' }).fill('Rent €1,475');
  await page.getByRole('textbox', { name: 'Replacement text' }).press('Enter');
  await expect(selected.getByRole('note')).toHaveText("Helvetica isn't embedded in this PDF. Edited lines use Noto Serif, which looks close. Untouched text keeps its original font.");
  await expect(selected).not.toContainText('Edited text keeps this font');

  await page.evaluate(() => {
    (window as any).exportOutcomes = [{ inputPath: '/local/scene-fixture.pdf', outputPaths: ['/local/scene-fixture-edited-1.pdf'], state: 'succeeded', detail: 'PDF scene exported', failure: null, reason: null,
      warnings: [{ code: 'fontSubstituted', message: 'Font substituted on page 1.', details: { requestedFamily: 'Helvetica', actualFamily: 'Noto Serif' } }] }];
  });
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  const row = page.locator('[data-tone]').filter({ hasText: 'scene-fixture.pdf' });
  await expect(row).toHaveAttribute('data-tone', 'warn');
  await expect(row).toContainText('The font Helvetica wasn’t available, so Noto Serif was used.');
  await expect(row).not.toContainText('Font substituted on page 1.');
});

test('a replacement longer than the original line warns that it will be cut off', async ({ page }) => {
  await openEditText(page);
  await replaceRun(page, 'Rent £1,475 per calendar month');
  await expect(page.locator('.scene-edit-overflow')).toHaveText('Text runs past the original line by 3 words. It will be cut off unless you make it shorter or smaller.');
  await expect(page.getByRole('region', { name: 'Properties' }).getByRole('alert')).toContainText('This text is too long for its space.');
  await page.locator('[data-edit-run="0"]').click();
  await page.getByRole('textbox', { name: 'Replacement text' }).fill('Rent £1,475');
  await page.getByRole('textbox', { name: 'Replacement text' }).press('Enter');
  await expect(page.locator('.scene-edit-overflow')).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Properties' }).getByRole('alert')).toHaveCount(0);
});

test('Edit PDF shows where the export goes and names it by the contributing tool', async ({ page }) => {
  await openEditor(page);
  // The folder sits next to Export; the "-edited" name comes from the contributing tool (shared/subtools.ts, kit/pdf/scene.rs).
  await expect(page.getByRole('button', { name: 'Save to /local. Change for this export' })).toContainText('local');
  await draw(page, 'Shape', .4);
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  const results = page.getByRole('list', { name: 'Results' });
  await expect(results.getByRole('button', { name: 'Open scene-fixture-edited.pdf' })).toBeVisible();
  await expect(results.getByRole('button', { name: /^Show in / })).toBeVisible();
});

test('Edit PDF sends a per-run folder in the export request without writing Settings', async ({ page }) => {
  await openEditor(page);
  await draw(page, 'Shape', .4);
  await page.evaluate(() => { (window as any).dialogNext = '/Users/test/Delivery'; });
  await page.getByRole('button', { name: /^Save to .*Change for this export$/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Save this export to' });
  await dialog.getByRole('button', { name: 'Choose folder…' }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole('button', { name: 'Save to /Users/test/Delivery, this export only. Change for this export' })).toContainText('This export only');
  await page.evaluate(() => { (window as any).dialogNext = null; });
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect(page.getByRole('list', { name: 'Results' })).toBeVisible();
  const request = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request);
  expect(request.outputLocation).toEqual({ customFolder: '/Users/test/Delivery' });
  expect(request.outputPath).toBeUndefined();
  expect(await page.evaluate(() => (window as any).calls.map((c: any) => c.command))).not.toContain('set_output_settings');
});

test('Export As saves a chosen copy and reports the numbered name instead of replacing a file', async ({ page }) => {
  await openEditor(page);
  await expect(page.getByRole('button', { name: 'Export As…', exact: true })).toBeDisabled();
  await draw(page, 'Shape', .4);
  await page.getByRole('button', { name: 'Export As…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Export a copy' });
  await expect(dialog.getByLabel('File name')).toHaveValue('scene-fixture-edited.pdf');
  await expect(dialog.getByTestId('save-folder')).toContainText('/local');
  await dialog.getByLabel('File name').fill('contract final.pdf');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toBeHidden();
  const request = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'export_pdf_scene').at(-1).args.request);
  expect(request.outputPath).toBe('/local/contract final.pdf');
  expect(request.paths).toEqual(['/local/scene-fixture.pdf']);
  const results = page.getByRole('list', { name: 'Results' });
  await expect(results).toContainText('Saved as contract final-1.pdf. Renamed to avoid replacing an existing file.');
  await expect(results.getByRole('button', { name: 'Open contract final-1.pdf' })).toBeVisible();
});

test('Export stays off with a reason until there is a change and the preview shows it', async ({ page }) => {
  await openEditor(page);
  const reason = page.getByRole('status').filter({ hasText: 'Nothing to export yet' });
  await expect(reason).toBeVisible();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toHaveAttribute('aria-describedby', 'scene-export-reason');
  await page.evaluate(() => { (window as any).previewDelay = 1500; });
  await draw(page, 'Shape', .4);
  await expect(page.getByRole('status').filter({ hasText: 'Updating the preview' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Export As…', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled({ timeout: 5000 });
  await expect(page.locator('#scene-export-reason')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).calls.some((c: any) => c.command === 'export_pdf_scene'))).toBe(false);
});
