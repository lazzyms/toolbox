import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';

const docs = new URL('./', import.meta.url);
const registrySource = readFileSync(new URL('../ToolboxTauri/src/registry/index.ts', docs), 'utf8');
const registryLiteral = registrySource.match(/export const UtilityRegistry[^=]*=\s*(\[[\s\S]*?\n\]);/);
assert(registryLiteral, 'Registry must expose a literal UtilityRegistry array');
const registry = JSON.parse(JSON.stringify(runInNewContext(registryLiteral[1], {}, { timeout: 1000 })));
const fields = ['id', 'category', 'status', 'command', 'verification'];
const matrixRows = readFileSync(new URL('tauri-tool-matrix.md', docs), 'utf8')
  .split('\n').filter(line => line.startsWith('|')).map(line => line.split('|').slice(1, -1).map(cell => cell.trim()));
const matrixStart = matrixRows.findIndex(row => row.join('|') === 'ID|Category|Status|Command|Verification');
assert(matrixStart >= 0, 'Matrix must include the registry table');
const matrix = matrixRows.slice(matrixStart);
const comparisons = [
  [registry.length, 32, 'Registry tool count'],
  [new Set(registry.map(tool => tool.id)).size, registry.length, 'Unique registry IDs'],
  [matrix[0], ['ID', 'Category', 'Status', 'Command', 'Verification'], 'Matrix columns'],
  [matrix[1], ['---', '---', '---', '---', '---'], 'Matrix separator'],
  [matrix.slice(2), registry.map(tool => fields.map(field => tool[field])), 'Matrix registry parity'],
];
const pages = new Map(readdirSync(docs).filter(name => name.endsWith('.html'))
  .map(name => [name, readFileSync(new URL(name, docs), 'utf8')]));
const appSource = readFileSync(new URL('assets/app.js', docs), 'utf8');
const iconTable = appSource.match(/const toolIcons = \{([\s\S]*?)\n  \};/);
assert(iconTable, 'Docs must define the shared tool icon table');
const toolIcons = new Map([...iconTable[1].matchAll(/(?:'([^']+)'|([a-z][\w-]*)):\s*'([^']+)'/g)]
  .map(match => [match[1] || match[2], match[3]]));
const releaseBase = 'https://github.com/lazzyms/toolbox/releases/download/tauri-v1.0.0/';
const downloadAssets = {
  macos: `${releaseBase}Toolbox-1.0.0-macos.dmg`,
  'windows-x64': 'https://apps.microsoft.com/detail/9N5R8W4GJVH4',
  'windows-arm64': 'https://apps.microsoft.com/detail/9N5R8W4GJVH4',
};
const publicSources = pages;
for (const [name, source] of publicSources) {
  const stale = source.match(/SwiftUI|Sparkle|PDFKit|\bAppKit\b|macOS[- ]only|releases\/latest|Download the DMG|pick the DMG|interactive recreation|simulated batch/gi);
  comparisons.push([stale, null, `${name} has no stale product claims or interactive demo copy`]);
}
for (const [name, source] of pages) {
  for (const tag of source.matchAll(/<a\b[^>]*\bdata-dl\b[^>]*>/g)) {
    const href = tag[0].match(/\bhref=["']([^"']*)["']/)?.[1];
    comparisons.push([Object.values(downloadAssets).includes(href), true, `${name} download destination`]);
  }
  for (const tag of source.matchAll(/\bdata-theme-src-(?:dark|light)=["']([^"']+)["']/g)) {
    const path = fileURLToPath(new URL(tag[1], new URL(name, docs)));
    comparisons.push([existsSync(path), true, `${name} themed screenshot ${tag[1]}`]);
  }
  for (const link of source.matchAll(/\b(?:href|src)=["']([^"']+)["']/g)) {
    const href = link[1].replaceAll('&amp;', '&');
    if (/^(?:[a-z][\w+.-]*:|\/\/)/i.test(href)) continue;
    const target = new URL(href, new URL(name, docs));
    const path = fileURLToPath(target);
    comparisons.push([existsSync(path), true, `${name} local link ${href}`]);
    if (target.hash && existsSync(path)) {
      const ids = [...readFileSync(path, 'utf8').matchAll(/\bid=["']([^"']+)["']/g)].map(match => match[1]);
      comparisons.push([ids.includes(decodeURIComponent(target.hash.slice(1))), true, `${name} anchor ${href}`]);
    }
  }
}
const homepage = pages.get('index.html');
for (const [platform, href] of Object.entries(downloadAssets)) {
  const card = homepage.match(new RegExp(`data-download-platform=["']${platform}["'][^>]*href=["']([^"']+)["']`));
  comparisons.push([card?.[1], href, `index.html ${platform} direct download`]);
}
for (const [name, categories] of [['pdf.html', ['Documents', 'PDF']], ['images.html', ['Images']]]) {
  const ids = [...pages.get(name).matchAll(/\bdata-tool-id=["']([^"']+)["']/g)].map(match => match[1]);
  comparisons.push([ids, registry.filter(tool => categories.includes(tool.category)).map(tool => tool.id), `${name} tool coverage and order`]);
  comparisons.push([ids.every(id => toolIcons.has(id)), true, `${name} tools use the shared icon table`]);
  for (const id of ids) {
    const icon = toolIcons.get(id);
    comparisons.push([icon && existsSync(new URL(`assets/design-icons/${icon}.svg`, docs)), true, `${name} ${id} icon asset`]);
  }
  comparisons.push([pages.get(name).includes('class="ico only-dark"') && pages.get(name).includes('class="ico only-light"'), true, `${name} theme icons`]);
}
let failures = 0;
for (const [actual, expected, label] of comparisons) {
  try {
    assert.deepEqual(actual, expected, label);
  } catch (error) {
    failures += 1;
    console.error(error.message);
  }
}
if (failures) process.exitCode = 1;
else console.log('Docs verified: registry, pages, matrix, downloads, screenshots, and local links agree.');
