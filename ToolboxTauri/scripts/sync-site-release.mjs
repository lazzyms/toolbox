// Points the website's pinned download links and release note at a published release.
// Usage: node scripts/sync-site-release.mjs <version> [YYYY-MM-DD]
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const siteRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../docs');
const version = String.raw`\d+\.\d+\.\d+`;

export const siteReleaseFiles = () => [
  ...readdirSync(siteRoot).filter((name) => name.endsWith('.html')),
  'assets/app.js',
].map((name) => resolve(siteRoot, name));

export const releaseDate = (isoDate) => new Date(`${isoDate}T00:00:00Z`).toLocaleDateString('en-US', {
  month: 'long', day: 'numeric', year: 'numeric', timeZone: 'UTC',
});

export const syncSiteRelease = (source, nextVersion, isoDate) => source
  .replace(new RegExp(`tauri-v${version}`, 'g'), `tauri-v${nextVersion}`)
  .replace(new RegExp(`Toolbox-${version}-macos\\.dmg`, 'g'), `Toolbox-${nextVersion}-macos.dmg`)
  .replace(new RegExp(`>Tauri ${version}</a> · Released [A-Z][a-z]+ \\d{1,2}, \\d{4}`, 'g'),
    `>Tauri ${nextVersion}</a> · Released ${releaseDate(isoDate)}`);

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [nextVersion, isoDate = new Date().toISOString().slice(0, 10)] = process.argv.slice(2);
  if (!new RegExp(`^${version}$`).test(nextVersion ?? '')) throw new Error('Usage: sync-site-release.mjs <version> [YYYY-MM-DD]');
  for (const file of siteReleaseFiles()) {
    const source = readFileSync(file, 'utf8');
    const next = syncSiteRelease(source, nextVersion, isoDate);
    if (next !== source) writeFileSync(file, next);
  }
}
