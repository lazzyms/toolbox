import { test, expect, type Page } from '@playwright/test';
import { openToolBySearch } from './open-tool';

const svg = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792"><rect width="612" height="792" fill="white"/></svg>');
const field = (name: string, label: string | null, type: 'text' | 'checkbox' | 'radio' | 'dropdown' | 'listbox', page: number, y: number, extra: Record<string, unknown> = {}) => ({
  name, label, type, value: type === 'checkbox' ? false : type === 'listbox' ? [] : '', options: type === 'checkbox' ? ['Yes'] : [], optionLabels: [], flags: 0,
  required: false, readOnly: false, multiline: false, multiSelect: false, editable: false, maxLen: null, format: null,
  widgets: [{ page, x: 72, y, width: type === 'checkbox' ? 14 : 220, height: type === 'checkbox' ? 14 : 22 }], ...extra,
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
  await expect(page.getByRole('button', { name: 'Reset…' })).toBeEnabled();
  await page.getByRole('button', { name: 'Reset…' }).click();
  await page.getByRole('alertdialog', { name: 'Reset all page changes?' }).getByRole('button', { name: 'Reset', exact: true }).click();
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

const radio = (option: string, x: number) => ({ page: 0, x, y: 60, width: 14, height: 14, option });
const choices = { unsupported: null, fields: [
  field('pets', 'Pets', 'radio', 0, 60, { options: ['Yes', 'No'], optionLabels: ['Yes', 'No'], required: true, flags: 2 | 1 << 15, widgets: [radio('Yes', 72), radio('No', 120)] }),
  field('status', 'Status', 'dropdown', 0, 100, { options: ['Employed', 'Retired', 'self'], optionLabels: ['Employed', 'Retired', 'Self-employed'] }),
  field('languages', 'Languages', 'listbox', 0, 140, { options: ['English', 'French', 'German'], optionLabels: ['English', 'French', 'German'], multiSelect: true,
    widgets: [{ page: 0, x: 72, y: 140, width: 220, height: 48 }] }),
  field('shift', 'Shift', 'listbox', 0, 200, { options: ['Day', 'Night'], optionLabels: ['Day', 'Night'], widgets: [{ page: 0, x: 72, y: 200, width: 220, height: 32 }] }),
  field('dob', 'Date of birth', 'text', 0, 250, { format: { kind: 'date', pattern: 'dd/mm/yyyy' } }),
  field('zip', 'ZIP', 'text', 0, 290, { maxLen: 5, format: { kind: 'special', special: 0 } }),
  field('phone', 'Phone', 'text', 0, 330, { required: true, flags: 2 }),
] };
const exported = async (page: Page) => {
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(1);
  return page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'export_pdf_scene').args.request.formValues);
};

test('radio groups, drop-downs and list boxes fill from their options and export the chosen values', async ({ page }, testInfo) => {
  await openForm(page, choices);
  const panel = inspector(page);
  const pets = canvas(page).getByRole('radiogroup', { name: 'Pets' });
  await pets.getByRole('radio', { name: 'No' }).click();
  await expect(pets.getByRole('radio', { name: 'No' })).toBeChecked();
  const status = canvas(page).getByRole('combobox', { name: 'Status' });
  await expect(status.getByRole('option')).toHaveText(['', 'Employed', 'Retired', 'Self-employed']);
  await status.selectOption('self');
  const languages = canvas(page).getByRole('toolbar', { name: 'Languages' });
  await languages.getByRole('button', { name: 'German' }).click();
  await languages.getByRole('button', { name: 'English' }).click();
  await expect(languages.getByRole('button', { name: 'English' })).toHaveAttribute('aria-pressed', 'true');
  const shift = canvas(page).getByRole('radiogroup', { name: 'Shift' });
  await shift.getByRole('radio', { name: 'Day' }).click();
  await shift.getByRole('radio', { name: 'Night' }).click();
  await expect(shift.getByRole('radio', { name: 'Day' })).not.toBeChecked();
  await expect(panel.getByRole('list', { name: 'Fields' }).getByRole('listitem')).toHaveText([
    'PetsNo', 'StatusSelf-employed', 'LanguagesEnglish, German', 'ShiftNight', 'Date of birth', 'ZIP', 'PhoneRequired, empty',
  ]);
  await page.screenshot({ path: testInfo.outputPath('pdf-form-choices.png') });
  expect(await exported(page)).toEqual([
    { name: 'pets', value: 'No' },
    { name: 'status', value: 'self' },
    { name: 'languages', value: ['English', 'German'] },
    { name: 'shift', value: ['Night'] },
  ]);
});

test('max length is enforced while typing and invalid or empty required fields are flagged', async ({ page }, testInfo) => {
  await openForm(page, choices);
  const panel = inspector(page);
  const zip = canvas(page).getByRole('textbox', { name: 'ZIP' });
  await zip.click();
  await page.keyboard.type('123456789');
  await expect(zip).toHaveValue('12345');

  const dob = canvas(page).getByRole('textbox', { name: 'Date of birth' });
  await dob.click();
  await page.keyboard.type('31/02/2024');
  await expect(dob).not.toHaveAttribute('aria-invalid');
  await page.keyboard.press('Tab');
  await expect(dob).toHaveAttribute('aria-invalid', 'true');
  await expect(dob).toHaveAccessibleDescription('Not a real date');
  await expect(page.getByRole('status').filter({ hasText: 'Date of birth: not a real date.' })).toBeVisible();
  await expect(panel.getByRole('button', { name: /^Date of birth/ }).locator('.text-destructive')).toHaveText('Not a real date');
  await expect(panel.getByRole('note')).toHaveText("7 fields on page 1. 1 field has an entry that isn't valid, and 2 required fields are still empty.");
  // The ZIP field was left with a valid entry, so it carries no flag.
  await expect(zip).not.toHaveAttribute('aria-invalid');

  const phone = canvas(page).getByRole('textbox', { name: 'Phone' });
  await phone.click();
  await expect(panel.getByRole('button', { name: /^Phone/ }).locator('.text-destructive')).toHaveCount(0);
  await dob.click();
  await expect(phone).toHaveAttribute('aria-invalid', 'true');
  await expect(phone).toHaveAccessibleDescription('Required');
  await expect(page.getByRole('status').filter({ hasText: 'Phone is required.' })).toBeVisible();
  await expect(panel.getByRole('button', { name: /^Phone/ }).locator('.text-destructive')).toHaveText('Required, empty');
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('29/02/2024');
  await phone.click();
  await expect(dob).not.toHaveAttribute('aria-invalid');
  await expect(panel.getByRole('note')).toHaveText('7 fields on page 1. 2 required fields are still empty.');
  await page.screenshot({ path: testInfo.outputPath('pdf-form-flags.png') });

  await panel.getByRole('button', { name: 'Go to next empty required field' }).click();
  await expect(canvas(page).getByRole('radiogroup', { name: 'Pets' }).getByRole('radio', { name: 'Yes' })).toBeFocused();
  await expect(page.getByRole('status').filter({ hasText: 'Pets is required.' })).toBeVisible();
  await page.keyboard.press('Space');
  await phone.fill('555 123 4567');
  await expect(panel.getByRole('note')).toHaveText('7 fields on page 1.');
  await expect(panel.getByRole('button', { name: 'Go to next empty required field' })).toBeDisabled();
  expect(await exported(page)).toEqual([
    { name: 'pets', value: 'Yes' },
    { name: 'dob', value: '29/02/2024' },
    { name: 'zip', value: '12345' },
    { name: 'phone', value: '555 123 4567' },
  ]);
});

const exportRequest = async (page: Page) => {
  await page.getByRole('button', { name: 'Export PDF', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'export_pdf_scene').length)).toBe(1);
  return page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'export_pdf_scene').args.request);
};

test('Flatten asks before turning on and exports exactly the chosen kind', async ({ page }) => {
  await openForm(page, { ...fillable, comments: 2 });
  const panel = inspector(page);
  await expect(panel.getByText("Turns the fields into plain page content in the exported copy, so they can't be changed later.")).toBeVisible();
  const choice = panel.getByRole('radiogroup', { name: 'What to flatten' });
  await expect(choice.getByRole('radio', { name: 'Fields and comments' })).toBeChecked();
  const flatten = panel.getByRole('checkbox', { name: 'Flatten fields when exporting' });

  await flatten.click();
  const dialog = page.getByRole('alertdialog', { name: 'Flatten fields and comments in the copy?' });
  await expect(dialog).toHaveAccessibleDescription('All 4 fields and 2 comments become part of the page in the exported copy. Nobody will be able to change them there. 2 required fields are still empty. You can turn this off before exporting.');
  await dialog.getByRole('button', { name: 'Keep fields editable' }).click();
  await expect(dialog).toBeHidden();
  await expect(flatten).not.toBeChecked();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();

  await choice.getByRole('radio', { name: 'Comments only' }).click();
  await flatten.click();
  const comments = page.getByRole('alertdialog', { name: 'Flatten comments in the copy?' });
  await expect(comments).toHaveAccessibleDescription('All 2 comments become part of the page in the exported copy. Nobody will be able to change them there. You can turn this off before exporting.');
  await comments.getByRole('button', { name: 'Flatten when exporting' }).click();
  await expect(flatten).toBeChecked();
  await expect(choice.getByRole('radio', { name: 'Fields only' })).toBeDisabled();

  // Turning it off needs no confirmation, and Reset turns it off too.
  await flatten.click();
  await expect(flatten).not.toBeChecked();
  await expect(page.getByRole('alertdialog')).toHaveCount(0);
  await flatten.click();
  await page.getByRole('alertdialog').getByRole('button', { name: 'Flatten when exporting' }).click();
  await page.getByRole('button', { name: 'Reset…' }).click();
  await page.getByRole('alertdialog', { name: 'Reset all page changes?' }).getByRole('button', { name: 'Reset', exact: true }).click();
  await expect(flatten).not.toBeChecked();
  await flatten.click();
  await page.getByRole('alertdialog').getByRole('button', { name: 'Flatten when exporting' }).click();

  const request = await exportRequest(page);
  expect(request.flatten).toBe('annotations');
  expect('formValues' in request).toBe(false);
});

test('filling and flattening export the values and the flatten choice together', async ({ page }) => {
  await openForm(page, { ...fillable, comments: 0 });
  const panel = inspector(page);
  await expect(panel.getByRole('radiogroup', { name: 'What to flatten' })).toHaveCount(0);
  await canvas(page).getByRole('textbox', { name: 'Full name' }).fill('Ada Lovelace');
  await panel.getByRole('checkbox', { name: 'Flatten fields when exporting' }).click();
  const dialog = page.getByRole('alertdialog', { name: 'Flatten fields in the copy?' });
  await expect(dialog).toHaveAccessibleDescription('All 4 fields become part of the page in the exported copy. Nobody will be able to change them there. 1 required field is still empty. You can turn this off before exporting.');
  await dialog.getByRole('button', { name: 'Flatten when exporting' }).click();

  const request = await exportRequest(page);
  expect(request.formValues).toEqual([{ name: 'applicant.name', value: 'Ada Lovelace' }]);
  expect(request.flatten).toBe('forms');
});

test('Flatten is unavailable with its reason when there is nothing to flatten', async ({ page }) => {
  await openForm(page, { fields: [], unsupported: null, comments: 0 });
  const panel = inspector(page);
  await expect(panel.getByText('Flatten is unavailable. This PDF has no form fields or comments to flatten.')).toBeVisible();
  await expect(panel.getByRole('checkbox', { name: 'Flatten fields when exporting' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Export PDF', exact: true })).toBeDisabled();
});

test('a PDF with comments but no fields can flatten its comments', async ({ page }) => {
  await openForm(page, { fields: [], unsupported: null, comments: 1 });
  const panel = inspector(page);
  await expect(panel.getByText("Turns the comments into plain page content in the exported copy, so they can't be changed later.")).toBeVisible();
  await panel.getByRole('checkbox', { name: 'Flatten comments when exporting' }).click();
  const dialog = page.getByRole('alertdialog', { name: 'Flatten comments in the copy?' });
  await expect(dialog).toHaveAccessibleDescription('The 1 comment becomes part of the page in the exported copy. Nobody will be able to change it there. You can turn this off before exporting.');
  await dialog.getByRole('button', { name: 'Flatten when exporting' }).click();
  const request = await exportRequest(page);
  expect(request.flatten).toBe('annotations');
});
