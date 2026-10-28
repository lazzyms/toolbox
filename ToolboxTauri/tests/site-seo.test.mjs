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

test('the home and category pages link every task guide', () => {
  const home = read('index.html');
  for (const page of landingPages) {
    assert.ok(home.includes(`href="${page.file}"`), `index.html links ${page.file}`);
    assert.ok(read(`${page.category}.html`).includes(`href="${page.file}"`), `${page.category}.html links ${page.file}`);
    for (const related of page.related) assert.ok(landingPages.some((other) => other.file === related), `${page.file} relates to ${related}`);
  }
});
