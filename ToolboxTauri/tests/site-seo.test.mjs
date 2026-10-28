import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { buildSite, landingPages, releaseVersion, site, sitePages } from '../../docs/landing-pages.mjs';
import { siteRoot } from '../scripts/sync-site-release.mjs';

const read = (file) => readFileSync(resolve(siteRoot, file), 'utf8');

test('generated landing pages, sitemap, robots.txt and llms.txt match docs/landing-pages.mjs', () => {
  for (const [file, source] of buildSite(releaseVersion())) {
    assert.equal(read(file), source, `${file} is stale: run node docs/landing-pages.mjs`);
  }
});

test('the sitemap lists every page and robots.txt points crawlers at it', () => {
  const locations = [...read('sitemap.xml').matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => match[1]);
  assert.deepEqual(locations, sitePages().map((file) => (file === 'index.html' ? `${site}/` : `${site}/${file}`)));
  assert.match(read('robots.txt'), new RegExp(`^Sitemap: ${site}/sitemap\\.xml$`, 'm'));
  assert.doesNotMatch(read('robots.txt'), /Disallow: \//);
});

test('every page has a unique title and description', () => {
  const pages = sitePages().map((file) => read(file));
  for (const pattern of [/<title>([^<]+)<\/title>/, /<meta name="description" content="([^"]+)">/]) {
    const values = pages.map((source) => source.match(pattern)?.[1]);
    assert.ok(values.every(Boolean), `every page needs ${pattern}`);
    assert.equal(new Set(values).size, values.length, `${pattern} must be unique`);
  }
});

test('structured data parses and the FAQ schema matches the visible questions', () => {
  for (const file of ['index.html', ...landingPages.map((page) => page.file)]) {
    const blocks = [...read(file).matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)];
    assert.equal(blocks.length, 1, `${file} needs one JSON-LD block`);
    const graph = JSON.parse(blocks[0][1])['@graph'];
    assert.ok(graph.some((node) => node['@type'] === 'SoftwareApplication'), `${file} describes the app`);
  }
  for (const page of landingPages) {
    const source = read(page.file);
    const faq = JSON.parse(source.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)[1])['@graph']
      .find((node) => node['@type'] === 'FAQPage');
    const visible = [...source.matchAll(/<summary>([^<]+)<svg/g)].map((match) => match[1].replaceAll('&#39;', "'"));
    assert.deepEqual(faq.mainEntity.map((question) => question.name), visible, `${page.file} FAQ schema`);
  }
});

test('every task guide is linked from its tool card, or from the home page', () => {
  for (const page of landingPages) {
    if (page.toolId) {
      const card = `<a class="feature-card" id="tool-${page.toolId}" href="${page.file}" data-tool-id="${page.toolId}">`;
      assert.ok(read(`${page.category}.html`).includes(card), `${page.category}.html card ${page.toolId} links ${page.file}`);
    } else {
      assert.ok(read('index.html').includes(`href="${page.file}"`), `index.html links ${page.file}`);
    }
    for (const related of page.related) assert.ok(landingPages.some((other) => other.file === related), `${page.file} relates to ${related}`);
  }
});

test('the tool search index lists every tool card, and every result link resolves', () => {
  const tools = JSON.parse(read('assets/tools.json'));
  const cards = ['pdf.html', 'images.html'].flatMap((file) => [...read(file).matchAll(/data-tool-id="([^"]+)"/g)].map((match) => match[1]));
  assert.equal(tools.length, cards.length);
  for (const tool of tools) {
    assert.ok(tool.name && tool.description, `${tool.href} has a name and description`);
    const [file, anchor] = tool.href.split('#');
    assert.ok(sitePages().includes(file), `${tool.name} links an existing page`);
    if (anchor) assert.ok(read(file).includes(`id="${anchor}"`), `${tool.name} links an existing card`);
  }
  for (const file of sitePages()) assert.match(read(file), /<header class="nav">[\s\S]*?data-tool-search[\s\S]*?<\/header>/, `${file} has the tool search in its header`);
});
