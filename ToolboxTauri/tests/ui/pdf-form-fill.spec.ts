import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from './open-tool';

const svg = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/></svg>');
const field = (name: string, label: string | null, type: 'text' | 'checkbox', page: number, y: number, extra: Record<string, unknown> = {}) => ({
  name, label, type, value: type === 'checkbox' ? false : '', options: type === 'checkbox' ? ['Yes'] : [], flags: 0,
  required: false, readOnly: false, multiline: false, maxLen: null, widgets: [{ page, x: 72, y, width: type === 'checkbox' ? 14 : 220, height: type === 'checkbox' ? 14 : 22 }], ...extra,
});
// Native Tab order differs from the visual order: Email sits above Full name on the page.
const fillable = { unsupported: null, fields: [
  field('applicant.name', 'Full name', 'text', 0, 160, { required: true, flags: 2 }),
  field('email', 'Email', 'text', 0, 100, { value: 'old@example.com' }),
  field('agree', 'Credit check consent', 'checkbox', 0, 220),
  field('city', null, 'text', 1, 100, { required: true, flags: 2 }),
] };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ svg }) => {
    const w = window as any;
    w.calls = [];
    w.formResponse = null;
    w.exportResponse = null;
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (command: string, args: any) => {
        w.calls.push({ command, args });
        if (command === 'plugin:dialog|open') return '/local/form.pdf';
        if (command === 'inspect_pdf_scene') return { path: '/local/form.pdf', pages: [0, 1].map(index => ({ index, width: 612, height: 792, preview: svg })) };
        if (command === 'inspect_pdf_scene_page') return { index: args.request.pageIndex, width: 612, height: 792, preview: svg, textRuns: [] };
        if (command === 'preview_pdf_scene_pages') return args.request.pageIndices.map((pageIndex: number) => ({ pageIndex, preview: { dataUrl: svg, width: 612, height: 792 } }));
        if (command === 'inspect_pdf_form') {
          if (w.formResponse === 'throw') throw new Error('Could not read the form');
          return w.formResponse;
        }
        if (command === 'export_pdf_scene') return w.exportResponse ?? [{ inputPath: '/local/form.pdf', outputPaths: ['/local/form-filled.pdf'], state: 'succeeded', detail: 'PDF scene exported', failure: null, reason: null, warnings: [] }];
        return null;
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, { svg });
});

async function openForm(page: Page, response: unknown) {
  await page.addInitScript((response) => { (window as any).formResponse = response; }, response);
  await page.goto('/');
  await openToolBySearch(page, 'Edit PDF');
  await page.getByRole('button', { name: 'Choose files to process' }).click();
  await expect(page.getByRole('group', { name: 'PDF page canvas' })).toBeVisible();
  await expect(page.locator('.scene-canvas image').first()).toBeAttached();
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_form').length)).toBe(0);
  await page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Fill form', exact: true }).click();
}
const inspector = (page: Page) => page.getByRole('complementary', { name: 'PDF editor inspector' });
const canvas = (page: Page) => page.getByRole('group', { name: 'PDF page canvas' });

test('Fill form inspects the open PDF once, fills fields in Tab order, and exports the exact values', async ({ page }, testInfo) => {
  await openForm(page, fillable);
  const panel = inspector(page);
  await expect(panel.getByText('Type into the fields on the page. Tab moves to the next field in the order below.')).toBeVisible();
  await expect(panel.getByRole('note')).toHaveText('4 fields on pages 1 and 2. 2 required fields are still empty.');
  await expect(panel.getByRole('list', { name: 'Fields' }).getByRole('listitem')).toHaveText(['Full nameRequired, empty', 'Email', 'Credit check consent', 'cityPage 2, required']);
  await expect(page.getByText('Fields are filled in the copy only')).toBeVisible();
  const inspections = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'inspect_pdf_form'));
  expect(inspections).toEqual([{ command: 'inspect_pdf_form', args: { request: { path: '/local/form.pdf' } } }]);

  const name = canvas(page).getByRole('textbox', { name: 'Full name' });
  const email = canvas(page).getByRole('textbox', { name: 'Email' });
  const agree = canvas(page).getByRole('checkbox', { name: 'Credit check consent' });
  await expect(email).toHaveValue('old@example.com');
  await name.click();
  await page.keyboard.type('Ada Lovelace');
  await page.keyboard.press('Tab');
  await expect(email).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(agree).toBeFocused();
  await page.keyboard.press('Space');
  await expect(agree).toBeChecked();
  await page.keyboard.press('Shift+Tab');
  await expect(email).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(name).toBeFocused();
  // Retyping a field's original value is not a change.
  await email.fill('changed');
  await email.fill('old@example.com');

  await expect(panel.getByRole('note')).toHaveText('4 fields on pages 1 and 2. 1 required field is still empty.');
  await expect(panel.getByRole('list', { name: 'Fields' }).getByRole('listitem')).toHaveText(['Full name', 'Email', 'Credit check consentChecked', 'cityPage 2, required']);

  await panel.getByRole('button', { name: /^city/ }).click();
  await expect(page.getByText('Page 2 of 2')).toBeVisible();
  await expect(canvas(page).getByRole('textbox', { name: 'city' })).toBeFocused();
  await page.keyboard.type('Zürich');

  await page.screenshot({ path: testInfo.outputPath('pdf-form-fill.png') });
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(1);
  const request = await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'export_pdf_scene').args.request);
  expect(request.formValues).toEqual([
    { name: 'applicant.name', value: 'Ada Lovelace' },
    { name: 'agree', value: true },
    { name: 'city', value: 'Zürich' },
  ]);
  expect(request.scene.pages.every((item: any) => item.objects.length === 0)).toBe(true);
});

test('form entries stay out of undo history and Reset clears them', async ({ page }) => {
  await openForm(page, fillable);
  await canvas(page).getByRole('textbox', { name: 'Full name' }).fill('Ada');
  await expect(page.getByRole('button', { name: 'Undo' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Reset edits' })).toBeEnabled();
  await page.getByRole('button', { name: 'Reset edits' }).click();
  await expect(canvas(page).getByRole('textbox', { name: 'Full name' })).toHaveValue('');
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

test('a PDF without fields says Fill form is unavailable and points to Add text', async ({ page }, testInfo) => {
  await openForm(page, { fields: [], unsupported: null });
  const panel = inspector(page);
  await expect(panel.getByRole('note')).toContainText('This PDF has no form fields to fill.');
  await expect(page.getByRole('status').filter({ hasText: 'Fill form is unavailable for this PDF.' })).toBeVisible();
  await expect(canvas(page).getByRole('textbox')).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('pdf-form-unavailable.png') });
  await panel.getByRole('button', { name: 'Use Add text instead' }).click();
  await expect(page.getByRole('toolbar', { name: 'PDF editor tools' }).getByRole('button', { name: 'Text', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('an XFA form explains in plain language why it cannot be filled', async ({ page }) => {
  await openForm(page, { fields: [], unsupported: "This form can't be filled here. It was built with XFA, an older Adobe form format that Toolbox doesn't support." });
  const panel = inspector(page);
  await expect(panel.getByText('This PDF has a form, but not one Toolbox can fill.')).toBeVisible();
  await expect(panel.getByRole('note').locator('strong')).toHaveText("This form can't be filled here.");
  await expect(panel.getByRole('note')).toContainText('It was built with XFA, an older Adobe form format');
  await expect(panel.getByText('To fill it anyway, open it in Adobe Acrobat or Reader. Or type your answers onto the page with Add text.')).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Use Add text instead' })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'Fill form is unavailable for this PDF.' })).toBeVisible();
});

test('a failed form inspection is shown as unavailable, not as a crash', async ({ page }) => {
  await openForm(page, 'throw');
  await expect(inspector(page).getByRole('note')).toContainText("Toolbox couldn't read its fields.");
  await expect(page.getByRole('status').filter({ hasText: 'Fill form is unavailable for this PDF.' })).toBeVisible();
});

test('a refused fill shows the native reason in plain language', async ({ page }) => {
  await openForm(page, fillable);
  await page.evaluate(() => {
    (window as any).exportResponse = [{ inputPath: '/local/form.pdf', outputPaths: [], state: 'failed', detail: 'Export failed',
      failure: { kind: 'invalidInput', message: '“applicant.name” is read-only, so it can\'t be filled.', pointer: null }, reason: null, warnings: [] }];
  });
  await canvas(page).getByRole('textbox', { name: 'Full name' }).fill('Ada');
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect(page.getByText('This PDF could not be exported safely. “applicant.name” is read-only, so it can\'t be filled.')).toBeVisible();
});
