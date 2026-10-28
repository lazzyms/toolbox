import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { parseReleaseNotes, renderReleaseNotes } from '../scripts/release-notes.mjs';

const template = readFileSync(resolve(import.meta.dirname, '../../.github/pull_request_template.md'), 'utf8');

test('an untouched PR template has no release notes', () => {
  assert.equal(parseReleaseNotes(template), null);
});

test('release-note bullets are grouped by heading and wrapped lines join', () => {
  const body = template.replace('### Fixes\n', '### Fixes\n- **PDF editor:** Export PDF now enables\n  after you add a watermark.\n')
    .replace('### Behind the scenes\n', '### Behind the scenes\n- Updated the demo.\n');
  assert.deepEqual(parseReleaseNotes(body), [
    { group: 'Fixes', text: '**PDF editor:** Export PDF now enables after you add a watermark.' },
    { group: 'Behind the scenes', text: 'Updated the demo.' },
  ]);
});

test('"None" marks a PR with nothing to tell users', () => {
  assert.deepEqual(parseReleaseNotes('## Release notes\n- None\n'), []);
});

test('the release body groups PR notes, falls back to titles and skips bookkeeping', () => {
  const notes = renderReleaseNotes({
    version: '1.0.28',
    previousTag: 'tauri-v1.0.27',
    commits: [
      { subject: 'Publish Tauri 1.0.27 update manifest [skip ci]' },
      { subject: 'Speed up OCR (#741)', pr: { number: 741, title: 'Speed up OCR', body: '## Release notes\n### New\n- **OCR:** Faster.\n### Website\n- New page.' } },
      { subject: 'Fix crash (#742)', pr: { number: 742, title: 'Fix crash on launch', body: 'No section here.' } },
      { subject: 'Bump deps (#743)', pr: { number: 743, title: 'Bump deps', body: '## Release notes\n- None' } },
      { subject: 'Tidy README' },
    ],
  });
  assert.equal(notes, [
    "## What's changed in 1.0.28",
    '### New\n- **OCR:** Faster. ([#741](https://github.com/lazzyms/toolbox/pull/741))',
    '### Website\n- New page. ([#741](https://github.com/lazzyms/toolbox/pull/741))',
    '### Other changes\n- Fix crash on launch ([#742](https://github.com/lazzyms/toolbox/pull/742))\n- Tidy README',
    '**Full Changelog**: https://github.com/lazzyms/toolbox/compare/tauri-v1.0.27...tauri-v1.0.28',
  ].join('\n\n') + '\n');
});
