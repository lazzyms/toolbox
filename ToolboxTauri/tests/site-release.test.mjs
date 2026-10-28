import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { siteReleaseFiles, siteRoot, syncSiteRelease } from '../scripts/sync-site-release.mjs';

const { version } = JSON.parse(readFileSync(resolve(siteRoot, '../tauri-latest.json'), 'utf8'));
const releaseUrls = (source) => source.match(/https:\/\/github\.com\/lazzyms\/toolbox\/releases\/[^"'\s<]+/g) ?? [];

test('every site download link points at the release in the update manifest', () => {
  const urls = siteReleaseFiles().flatMap((file) => releaseUrls(readFileSync(file, 'utf8')));
  assert.ok(urls.length > 0, 'The site must link to the GitHub release');
  for (const url of urls) {
    assert.ok([
      `https://github.com/lazzyms/toolbox/releases/download/tauri-v${version}/Toolbox-${version}-macos.dmg`,
      `https://github.com/lazzyms/toolbox/releases/tag/tauri-v${version}`,
    ].includes(url), `${url} must point at Toolbox ${version}`);
  }
  assert.match(readFileSync(resolve(siteRoot, 'index.html'), 'utf8'), new RegExp(`>Tauri ${version.replaceAll('.', '\\.')}</a> · Released `));
});

test('syncing the site rewrites pinned links and the release note only', () => {
  const source = [
    '<a href="https://github.com/lazzyms/toolbox/releases/download/tauri-v1.0.19/Toolbox-1.0.19-macos.dmg">',
    '<a href="https://github.com/lazzyms/toolbox/releases/tag/tauri-v1.0.19">Tauri 1.0.19</a> · Released October 22, 2026',
    '<p class="lede">Last updated: October 22, 2026</p>',
  ].join('\n');
  assert.equal(syncSiteRelease(source, '1.0.25', '2026-10-27'), [
    '<a href="https://github.com/lazzyms/toolbox/releases/download/tauri-v1.0.25/Toolbox-1.0.25-macos.dmg">',
    '<a href="https://github.com/lazzyms/toolbox/releases/tag/tauri-v1.0.25">Tauri 1.0.25</a> · Released October 27, 2026',
    '<p class="lede">Last updated: October 22, 2026</p>',
  ].join('\n'));
});

test('every canonical and social URL uses the custom domain GitHub Pages serves', () => {
  const domain = readFileSync(resolve(siteRoot, 'CNAME'), 'utf8').trim();
  const pages = siteReleaseFiles().filter((file) => file.endsWith('.html')).map((file) => readFileSync(file, 'utf8'));
  const urls = pages.flatMap((source) => [...source.matchAll(/<(?:link rel="canonical" href|meta (?:property|name)="(?:og:url|og:image|twitter:image)" content)="([^"]+)"/g)].map((match) => match[1]));
  assert.equal(urls.length, pages.length * 4, 'Each page needs canonical, og:url, og:image and twitter:image');
  for (const url of urls) assert.equal(new URL(url).host, domain, `${url} must use ${domain}`);
});
