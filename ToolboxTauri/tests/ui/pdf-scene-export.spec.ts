import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from './open-tool';

type Call = { command: string; args: any };
type ExportWindow = Window & {
  calls: Call[];
  dialogNext: string | null;
  holdExport: boolean;
  exportOutcome: unknown;
  signed: boolean;
  fileNext: string;
  inspectFailure: string | null;
  __emit: (payload: unknown) => void;
  __finish: () => void;
};

const svg = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/></svg>');
const succeeded = (output: string) => [{ inputPath: '/local/docs/scene-fixture.pdf', outputPaths: [output], state: 'succeeded', detail: '3 pages kept, 0 removed, 0 rotated, 0 mirrored.', failure: null, reason: null, warnings: [] }];
const failed = [{ inputPath: '/local/docs/scene-fixture.pdf', outputPaths: [], state: 'failed', detail: '', failure: { kind: 'processing', message: 'The destination folder is read-only.', pointer: null }, reason: null, warnings: [] }];

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg, success }) => {
    const w = window as unknown as ExportWindow;
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let nextId = 1;
    let finish: (() => void) | null = null;
    w.calls = [];
    w.dialogNext = null;
    w.holdExport = false;
    w.exportOutcome = success;
    w.signed = false;
    w.fileNext = '/local/docs/scene-fixture.pdf';
    w.inspectFailure = null;
    Object.defineProperty(window.navigator, 'platform', { configurable: true, value: 'MacIntel' });
    const dispatch = (name: string, payload: unknown) => {
      for (const [id, listener] of listeners) if (listener.event === name) callbacks.get(listener.handler)?.({ event: name, id, payload });
    };
    w.__emit = (payload) => dispatch('batch-progress', payload);
    w.__finish = () => finish?.();
    (window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:event|listen') {
          const id = nextId++;
          listeners.set(id, { event: args.event, handler: args.handler });
          return id;
        }
        if (command === 'plugin:event|unlisten') {
          listeners.delete(args.eventId);
          return null;
        }
        if (command === 'get_output_settings') return { mode: 'alongsideInput', folder: '/Users/test/Documents/Toolbox' };
        if (command === 'check_output_folder') return 'ready';
        if (command === 'plugin:path|resolve_directory') return '/Users/test';
        if (command === 'plugin:dialog|open') {
          if (args?.options?.directory) return w.dialogNext;
          return w.fileNext;
        }
        if (command === 'inspect_pdf_scene' && w.inspectFailure) throw new Error(w.inspectFailure);
        if (command === 'inspect_pdf_scene') return { path: '/local/docs/scene-fixture.pdf', signed: w.signed, pages: [0, 1, 2].map(index => ({ index, width: 612, height: 792, preview: null })) };
        if (command === 'inspect_pdf_scene_page') return { index: args.request.pageIndex, width: 612, height: 792, preview: svg, textRuns: [] };
        if (command === 'preview_pdf_scene_pages') return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        if (command === 'export_pdf_scene') {
          if (w.holdExport) await new Promise<void>((resolve) => { finish = resolve; });
          return w.exportOutcome;
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
  }, { svg, success: succeeded('/local/docs/scene-fixture-edited.pdf') });
});

const calls = (page: Page, command: string) =>
  page.evaluate((command) => (window as unknown as ExportWindow).calls.filter((call) => call.command === command), command);

async function openDirtyEditor(page: Page) {
  await page.goto('/');
  await openToolBySearch(page, 'Edit PDF');
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.locator('.scene-canvas image').first()).toBeAttached();
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Shape', exact: true }).click();
  const box = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(box.x + box.width * .2, box.y + box.height * .4);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .55, box.y + box.height * .47, { steps: 6 });
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeEnabled();
}

const exportButton = (page: Page) => page.getByRole('button', { name: 'Export PDF', exact: true });

test('export goes to the app-wide default, shown next to Export', async ({ page }) => {
  await openDirtyEditor(page);
  const destination = page.getByRole('button', { name: 'Save to /local/docs. Change for this export' });
  await expect(destination).toBeVisible();
  await expect(destination).toHaveText('docs');
  await expect(destination).toHaveAttribute('aria-haspopup', 'dialog');

  await exportButton(page).click();
  await expect.poll(async () => (await calls(page, 'export_pdf_scene')).length).toBe(1);
  const [request] = await calls(page, 'export_pdf_scene');
  expect(request.args.request.outputLocation).toBe('alongsideInput');
  expect(request.args.request.paths).toEqual(['/local/docs/scene-fixture.pdf']);
});

test('a per-run folder is sent with the export and leaves Settings unchanged', async ({ page }) => {
  await openDirtyEditor(page);
  await page.evaluate(() => { (window as unknown as ExportWindow).dialogNext = '/Users/test/Exports'; });

  await page.getByRole('button', { name: /^Save to .*Change for this export$/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Save this export to' });
  await expect(dialog).toContainText('For this export only. Your default in Settings stays “Next to the original”.');
  await expect(dialog.getByRole('button', { name: 'Use default' })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Choose folder…' }).click();
  await expect(dialog).toBeHidden();

  const overridden = page.getByRole('button', { name: 'Save to ~/Exports, this export only. Change for this export' });
  await expect(overridden).toContainText('Exports');
  await expect(overridden).toContainText('This export only');

  await exportButton(page).click();
  await expect.poll(async () => (await calls(page, 'export_pdf_scene')).length).toBe(1);
  const [request] = await calls(page, 'export_pdf_scene');
  expect(request.args.request.outputLocation).toEqual({ customFolder: '/Users/test/Exports' });
  expect(await calls(page, 'check_output_folder')).toEqual([{ command: 'check_output_folder', args: { folder: '/Users/test/Exports' } }]);
  expect(await calls(page, 'set_output_settings')).toEqual([]);

  await overridden.click();
  await expect(page.getByTestId('export-destination-path')).toHaveText('~/Exports');
  await dialog.getByRole('button', { name: 'Use default' }).click();
  await expect(page.getByRole('button', { name: 'Save to /local/docs. Change for this export' })).toBeVisible();
  await expect(page.getByText('This export only')).toHaveCount(0);
  expect(await calls(page, 'set_output_settings')).toEqual([]);
});

test('a slow export shows page progress in place of Export', async ({ page }) => {
  await openDirtyEditor(page);
  await page.evaluate(() => { (window as unknown as ExportWindow).holdExport = true; });
  await exportButton(page).click();
  await expect.poll(async () => (await calls(page, 'export_pdf_scene')).length).toBe(1);
  const [request] = await calls(page, 'export_pdf_scene');
  const runId = request.args.runId as string;
  expect(runId).toMatch(/\S+/);

  const progress = page.getByRole('progressbar', { name: 'Exporting scene-fixture.pdf' });
  await expect(progress).toBeVisible();
  await expect(exportButton(page)).toHaveCount(0);
  await expect(page.locator('.export-progress-text')).toHaveText('Exporting…');

  const emit = (payload: Record<string, unknown>) => page.evaluate((payload) => (window as unknown as ExportWindow).__emit(payload), { runId, ...payload });
  await emit({ index: 0, phase: 'started' });
  await emit({ index: 0, phase: 'progress', completed: 1, total: 3 });
  await expect(page.locator('.export-progress-text')).toHaveText('Page 2 of 3');
  await expect(progress).toHaveAttribute('aria-valuenow', '33');
  await emit({ runId: 'another-run', index: 0, phase: 'progress', completed: 3, total: 3 });
  await expect(page.locator('.export-progress-text')).toHaveText('Page 2 of 3');
  await emit({ index: 0, phase: 'progress', completed: 3, total: 3 });
  await expect(page.locator('.export-progress-text')).toHaveText('Page 3 of 3');
  await expect(progress).toHaveAttribute('aria-valuenow', '100');

  await page.evaluate(() => (window as unknown as ExportWindow).__finish());
  await expect(progress).toHaveCount(0);
  await expect(exportButton(page)).toBeVisible();
});

test('the result row offers Open and Reveal, or shows the failure message', async ({ page }) => {
  await openDirtyEditor(page);
  await exportButton(page).click();
  const rows = page.getByRole('list', { name: 'Results' }).getByRole('listitem');
  await rows.first().getByRole('button', { name: 'Open scene-fixture-edited.pdf' }).click();
  await rows.first().getByRole('button', { name: 'Show in Finder: scene-fixture-edited.pdf' }).click();
  await expect(rows.first()).toContainText('3 pages kept, 0 removed, 0 rotated, 0 mirrored.');
  expect((await calls(page, 'open_output_path')).map((call) => call.args.path)).toEqual(['/local/docs/scene-fixture-edited.pdf']);
  expect((await calls(page, 'reveal_output_path')).map((call) => call.args.path)).toEqual(['/local/docs/scene-fixture-edited.pdf']);

  await page.evaluate((outcome) => { (window as unknown as ExportWindow).exportOutcome = outcome; }, failed);
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Shape', exact: true }).click();
  const box = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(box.x + box.width * .2, box.y + box.height * .6);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .5, box.y + box.height * .66, { steps: 6 });
  await page.mouse.up();
  await exportButton(page).click();
  await expect(rows.first()).toContainText('The destination folder is read-only.');
  await expect(rows.first().getByRole('button', { name: /^Open / })).toHaveCount(0);
  await expect(rows.first().getByRole('button', { name: /^Show in Finder/ })).toHaveCount(0);
});

test('a signed PDF shows the signature notice on open and the removed-signature warning on the result row', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as ExportWindow;
    w.signed = true;
    w.exportOutcome = [{ inputPath: '/local/docs/scene-fixture.pdf', outputPaths: ['/local/docs/scene-fixture-edited.pdf'], state: 'succeeded', detail: '3 pages kept, 0 removed, 0 rotated, 0 mirrored.', failure: null, reason: null,
      warnings: [{ code: 'signatureRemoved', message: 'native wording' }] }];
  });
  await openDirtyEditor(page);
  const notice = page.getByRole('status').filter({ hasText: 'This PDF is digitally signed.' });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Any copy you export won't be signed any more. The original keeps its signature.");

  await exportButton(page).click();
  const row = page.getByRole('list', { name: 'Results' }).getByRole('listitem').first();
  await expect(row).toContainText('Saved. The digital signature was removed, because changing the file breaks it.');
  await expect(row).toHaveAttribute('data-tone', 'warn');

  await notice.getByRole('button', { name: 'Dismiss signature notice' }).click();
  await expect(notice).toHaveCount(0);
});

test('an unsigned PDF shows no signature notice and no removed-signature warning', async ({ page }) => {
  await openDirtyEditor(page);
  await expect(page.getByText('This PDF is digitally signed.')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Dismiss signature notice' })).toHaveCount(0);

  await exportButton(page).click();
  const row = page.getByRole('list', { name: 'Results' }).getByRole('listitem').first();
  await expect(row).toContainText('3 pages kept, 0 removed, 0 rotated, 0 mirrored.');
  await expect(row).not.toContainText('digital signature');
  await expect(row).not.toHaveAttribute('data-tone', 'warn');
});

// The native refusal for a password-protected source (kit/pdf/scene.rs LOCKED_PDF), as the invoke rejects with it.
const locked = 'This PDF is password-protected. Remove its password with File Security first, then open the unlocked copy here.';

async function openLockedPdf(page: Page) {
  await page.addInitScript((locked) => {
    const w = window as unknown as ExportWindow;
    w.fileNext = '/local/docs/tax-return-2025.pdf';
    w.inspectFailure = locked;
  }, locked);
  await page.goto('/');
  await openToolBySearch(page, 'Edit PDF');
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  const alert = page.getByRole('alert').filter({ hasText: 'is password-protected' });
  await expect(alert.getByRole('heading', { level: 2, name: 'tax-return-2025.pdf is password-protected' })).toBeVisible();
  await expect(alert).toContainText('Remove its password with File Security first, then open the unlocked copy here.');
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toHaveCount(0);
  await expect(page.getByText('could not be opened')).toHaveCount(0);
  await expect(exportButton(page)).toBeDisabled();
}

test('a password-protected PDF opens to the locked state, and Open File Security opens Lock & Unlock', async ({ page }, testInfo) => {
  await openLockedPdf(page);
  await page.screenshot({ path: testInfo.outputPath('locked-pdf.png') });
  await page.getByRole('button', { name: 'Open File Security', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Lock & Unlock', exact: true })).toBeVisible();
  expect(await calls(page, 'export_pdf_scene')).toHaveLength(0);
});

test('Open another PDF replaces the password-protected file and opens the new one', async ({ page }) => {
  await openLockedPdf(page);
  await expect(page.getByRole('button', { name: /^Open another PDF/ })).toContainText('⌘O');
  await page.evaluate(() => {
    const w = window as unknown as ExportWindow;
    w.fileNext = '/local/docs/scene-fixture.pdf';
    w.inspectFailure = null;
  });
  await page.getByRole('button', { name: /^Open another PDF/ }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.getByText('is password-protected')).toHaveCount(0);
  const opened = (await calls(page, 'inspect_pdf_scene')).map((call) => call.args.request.path);
  expect([...new Set(opened)]).toEqual(['/local/docs/tax-return-2025.pdf', '/local/docs/scene-fixture.pdf']);
  await expect(page.getByText('tax-return-2025.pdf', { exact: true })).toHaveCount(0);
});

test('a password-protected export result reads plainly and points to Lock & Unlock', async ({ page }) => {
  await openDirtyEditor(page);
  await page.evaluate((locked) => {
    (window as unknown as ExportWindow).exportOutcome = [{ inputPath: '/local/docs/scene-fixture.pdf', outputPaths: [], state: 'failed', detail: '', reason: null, warnings: [],
      failure: { kind: 'invalidInput', message: locked, pointer: { kind: 'tool', toolId: 'lock-unlock' } } }];
  }, locked);
  await exportButton(page).click();
  const row = page.getByRole('list', { name: 'Results' }).getByRole('listitem').first();
  await expect(row).toContainText(locked);
  await expect(row).not.toContainText('could not be exported safely');
  await expect(row).toContainText("Failed, the input can't be used");
  await expect(row.getByRole('link', { name: /scene-fixture-edited/ })).toHaveCount(0);
  await row.getByRole('button', { name: 'Open Lock & Unlock', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Lock & Unlock', exact: true })).toBeVisible();
});

test('native structure refusals still read as plain language on the result row', async ({ page }) => {
  const cases = [
    ['This PDF contains bookmarks or destinations whose coordinates cannot be transformed safely after crop or rotation; scene export was rejected before output',
      'This PDF contains bookmarks or destinations that cannot be preserved after this page edit. Undo the crop or rotation, or use a copy without them.'],
    ['This PDF page has annotations or form widgets and the requested crop or rotation would change their coordinates; scene export was rejected before output',
      'This PDF has annotations or form fields that cannot be preserved after this page edit. Undo the crop or rotation, or use a copy without them.'],
    ['This PDF contains navigation or form structures that could target a removed page; scene export was rejected before output',
      'This page edit could break bookmarks or form fields. Undo it, or use a copy without those structures.'],
    ['This tagged PDF contains structure mappings that cannot be remapped by scene edits; scene export was rejected before output',
      'This tagged PDF uses accessibility structure that cannot be preserved after this edit. Undo the page edit or use a copy without tagged structure.'],
    ['PDF uses a nested or unsupported page tree; scene export was rejected before output',
      'This PDF uses a page structure that Toolbox cannot edit safely. Use a copy with a standard page tree.'],
  ];
  await openDirtyEditor(page);
  const row = page.getByRole('list', { name: 'Results' }).getByRole('listitem').first();
  for (const [native, plain] of cases) {
    await page.evaluate((message) => {
      (window as unknown as ExportWindow).exportOutcome = [{ inputPath: '/local/docs/scene-fixture.pdf', outputPaths: [], state: 'failed', detail: '', reason: null, warnings: [],
        failure: { kind: 'processing', message, pointer: null } }];
    }, native);
    await exportButton(page).click();
    await expect(row).toContainText(plain);
    await expect(row).not.toContainText('rejected before output');
  }
});
