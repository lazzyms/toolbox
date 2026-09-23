import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
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
const release = {
  tag: 'tauri-v1.0.11',
  version: '1.0.11',
  publishedDate: 'September 9, 2026',
  macosDmgUrl: 'https://github.com/lazzyms/toolbox/releases/download/tauri-v1.0.11/Toolbox-1.0.11-macos.dmg',
  windowsStoreUrl: 'https://apps.microsoft.com/detail/9N5R8W4GJVH4',
};
const pageMetadata = [
  ['index.html', 'Toolbox | Local file utilities for macOS and Windows', 'Local PDF, Office, and image utilities for macOS and Windows. Remove passwords, edit PDFs, and convert, compress, or resize images without uploading files.', 'https://lazzyms.github.io/toolbox/'],
  ['pdf.html', 'PDF and document tools | Toolbox', '18 local PDF and document utilities for macOS and Windows. Remove passwords, edit, merge, split, sign, protect, and organize files.', 'https://lazzyms.github.io/toolbox/pdf.html'],
  ['images.html', 'Image tools | Toolbox', '14 local image utilities for macOS and Windows. Convert, compress, resize, crop, watermark, and inspect images in batches.', 'https://lazzyms.github.io/toolbox/images.html'],
  ['privacy.html', 'Privacy Policy | Toolbox', 'Toolbox privacy policy: files stay on your device, with limited anonymous install and app-open analytics in release builds.', 'https://lazzyms.github.io/toolbox/privacy.html'],
  ['terms.html', 'Terms of Use | Toolbox', 'Terms of use for the Toolbox website and desktop application. Toolbox is free software released under the MIT License.', 'https://lazzyms.github.io/toolbox/terms.html'],
].map(([file, title, description, canonicalUrl]) => ({
  file, title, description, canonicalUrl,
  ogType: 'website',
  ogTitle: title, ogDescription: description, ogUrl: canonicalUrl,
  socialImageUrl: 'https://lazzyms.github.io/toolbox/og.jpg',
  socialImageAlt: 'Toolbox social preview showing macOS and Windows availability and local PDF and image tools',
}));
const tagAttributes = tag => Object.fromEntries(
  [...tag.matchAll(/([a-z_:][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')/gi)]
    .map(([, name, doubleQuoted, singleQuoted]) => [name.toLowerCase(), doubleQuoted ?? singleQuoted])
);
const metaContent = (source, name, attribute = 'name') => [...source.matchAll(/<meta\b[^>]*>/gi)]
  .map(([tag]) => tagAttributes(tag))
  .find(attributes => attributes[attribute] === name)?.content;
const linkContent = (source, rel) => [...source.matchAll(/<link\b[^>]*>/gi)]
  .map(([tag]) => tagAttributes(tag))
  .find(attributes => attributes.rel === rel)?.href;
const contrastRatio = (foreground, background) => {
  const channel = value => {
    const normalized = value / 255;
    return normalized <= 0.03928 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
  };
  const luminance = hex => {
    const rgb = hex.match(/[a-f\d]{2}/gi).map(value => parseInt(value, 16));
    return 0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2]);
  };
  const [first, second] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (first + 0.05) / (second + 0.05);
};
for (const metadata of pageMetadata) {
  const source = pages.get(metadata.file);
  assert(source, `${metadata.file} must exist`);
  comparisons.push([source.match(/<title>([^<]+)<\/title>/)?.[1], metadata.title, `${metadata.file} title`]);
  comparisons.push([metaContent(source, 'description'), metadata.description, `${metadata.file} description`]);
  comparisons.push([linkContent(source, 'canonical'), metadata.canonicalUrl, `${metadata.file} canonical`]);
  comparisons.push([metaContent(source, 'og:type', 'property'), metadata.ogType, `${metadata.file} og:type`]);
  comparisons.push([metaContent(source, 'og:title', 'property'), metadata.ogTitle, `${metadata.file} og:title`]);
  comparisons.push([metaContent(source, 'og:description', 'property'), metadata.ogDescription, `${metadata.file} og:description`]);
  comparisons.push([metaContent(source, 'og:url', 'property'), metadata.ogUrl, `${metadata.file} og:url`]);
  comparisons.push([metaContent(source, 'og:image', 'property'), metadata.socialImageUrl, `${metadata.file} og:image`]);
  comparisons.push([metaContent(source, 'og:image:type', 'property'), 'image/jpeg', `${metadata.file} og:image type`]);
  comparisons.push([metaContent(source, 'og:image:width', 'property'), '1200', `${metadata.file} og:image width`]);
  comparisons.push([metaContent(source, 'og:image:height', 'property'), '630', `${metadata.file} og:image height`]);
  comparisons.push([metaContent(source, 'og:image:alt', 'property'), metadata.socialImageAlt, `${metadata.file} og:image alt`]);
  comparisons.push([metaContent(source, 'twitter:card'), 'summary_large_image', `${metadata.file} Twitter card`]);
  comparisons.push([metaContent(source, 'twitter:title'), metadata.ogTitle, `${metadata.file} Twitter title`]);
  comparisons.push([metaContent(source, 'twitter:description'), metadata.ogDescription, `${metadata.file} Twitter description`]);
  comparisons.push([metaContent(source, 'twitter:image'), metadata.socialImageUrl, `${metadata.file} Twitter image`]);
  comparisons.push([metaContent(source, 'twitter:image:alt'), metadata.socialImageAlt, `${metadata.file} Twitter image alt`]);
  comparisons.push([metadata.canonicalUrl.startsWith('https://'), true, `${metadata.file} canonical HTTPS`]);
  comparisons.push([metadata.ogUrl.startsWith('https://') && metadata.socialImageUrl.startsWith('https://'), true, `${metadata.file} social URLs HTTPS`]);
  comparisons.push([source.includes('terms.html'), true, `${metadata.file} links Terms`]);
  comparisons.push([source.includes('privacy.html') || metadata.file === 'privacy.html', true, `${metadata.file} links Privacy`]);
}
const jpegDimensions = image => {
  if (image[0] !== 0xff || image[1] !== 0xd8) return null;
  const frameMarkers = new Set([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf]);
  let offset = 2;
  while (offset + 3 < image.length) {
    if (image[offset] !== 0xff) { offset += 1; continue; }
    while (image[offset] === 0xff) offset += 1;
    const marker = image[offset++];
    if (marker === 0xd9 || marker === 0xda) break;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    const segmentLength = image.readUInt16BE(offset);
    if (segmentLength < 2 || offset + segmentLength > image.length) return null;
    if (frameMarkers.has(marker)) return {
      height: image.readUInt16BE(offset + 3),
      width: image.readUInt16BE(offset + 5),
    };
    offset += segmentLength;
  }
  return null;
};
const socialImagePath = fileURLToPath(new URL('og.jpg', docs));
const socialImage = existsSync(socialImagePath) ? readFileSync(socialImagePath) : null;
comparisons.push([existsSync(socialImagePath), true, 'Social preview image exists']);
comparisons.push([existsSync(socialImagePath) && statSync(socialImagePath).size < 200_000, true, 'Social preview image is under 200 KB']);
comparisons.push([jpegDimensions(socialImage ?? Buffer.alloc(0)), { width: 1200, height: 630 }, 'Social preview JPEG dimensions']);
comparisons.push([pages.get('privacy.html').includes("local storage") && pages.get('privacy.html').includes('not sent to Toolbox'), true, 'Privacy explains theme preference storage']);
comparisons.push([appSource.includes(release.macosDmgUrl), true, 'app.js macOS release parity']);
comparisons.push([appSource.includes(release.windowsStoreUrl), true, 'app.js Windows Store parity']);
const releaseBase = `https://github.com/lazzyms/toolbox/releases/download/${release.tag}/`;
comparisons.push([release.macosDmgUrl, `${releaseBase}Toolbox-${release.version}-macos.dmg`, 'Release tuple is internally consistent']);
const homepage = pages.get('index.html');
comparisons.push([homepage.includes(`Tauri ${release.version}`), true, 'Homepage release version']);
comparisons.push([homepage.includes(`Released ${release.publishedDate}`), true, 'Homepage release date']);
comparisons.push([homepage.includes(`releases/tag/${release.tag}`), true, 'Homepage release tag']);
const darkCss = readFileSync(new URL('assets/styles.css', docs), 'utf8');
const darkDefaultCta = darkCss.match(/\[data-theme="dark"\] \.btn-default \{ background: (#[0-9a-f]+); color: (#[0-9a-f]+); \}/i);
const darkHoverCta = darkCss.match(/\[data-theme="dark"\] \.btn-default:hover, \[data-theme="dark"\] \.btn-default:focus-visible \{ background: (#[0-9a-f]+); color: (white|#[0-9a-f]+); \}/i);
assert(darkDefaultCta && darkHoverCta, 'Dark CTA colors must be explicit in CSS');
const darkHoverForeground = darkHoverCta[2] === 'white' ? '#ffffff' : darkHoverCta[2];
comparisons.push([contrastRatio(darkDefaultCta[2], darkDefaultCta[1]) >= 4.5, true, 'Dark CTA default contrast']);
comparisons.push([contrastRatio(darkHoverForeground, darkHoverCta[1]) >= 4.5, true, 'Dark CTA hover/focus contrast']);
const iconTable = appSource.match(/const toolIcons = \{([\s\S]*?)\n  \};/);
assert(iconTable, 'Docs must define the shared tool icon table');
const toolIcons = new Map([...iconTable[1].matchAll(/(?:'([^']+)'|([a-z][\w-]*)):\s*'([^']+)'/g)]
  .map(match => [match[1] || match[2], match[3]]));
const downloadAssets = {
  macos: release.macosDmgUrl,
  'windows-x64': release.windowsStoreUrl,
  'windows-arm64': release.windowsStoreUrl,
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
