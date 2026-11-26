import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { site } from '../../docs/landing-pages.mjs';
import { prepareSite } from '../../scripts/prepare-netlify-docs.mjs';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const docsRoot = join(repositoryRoot, 'docs');
const builder = join(repositoryRoot, 'scripts/prepare-netlify-docs.mjs');
const artifactPath = resolve(repositoryRoot, 'ToolboxTauri/test-results/netlify-publish-manifest.sha256');
const host = 'https://example.test';

const listFiles = (root, directory = root) => readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
  const path = join(directory, entry.name);
  return entry.isDirectory() ? listFiles(root, path) : [relative(root, path).split('\\').join('/')];
}).sort();

const manifest = (root) => listFiles(root)
  .map((file) => `${createHash('sha256').update(readFileSync(join(root, file))).digest('hex')}  ${file}`)
  .join('\n') + '\n';

const temporaryDirectory = (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'toolbox-netlify-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
};

const page = (file, body = '') => `<!doctype html><html><head>
<link rel="canonical" href="${host}${file === 'index.html' ? '/' : `/${file}`}">
<meta property="og:url" content="${host}${file === 'index.html' ? '/' : `/${file}`}">
<meta property="og:image" content="${host}/og.jpg">
<meta name="twitter:image" content="${host}/og.jpg">
<link rel="stylesheet" href="assets/styles.css">
</head><body><a href="index.html">Home</a>${body}</body></html>`;

const fixtureSite = (t, overrides = {}) => {
  const root = temporaryDirectory(t);
  const sourceRoot = join(root, 'docs');
  const files = {
    'index.html': page('index.html', '<a href="pdf.html">PDF</a>'),
    'pdf.html': page('pdf.html'),
    'og.jpg': 'jpg',
    'robots.txt': `User-agent: *\nAllow: /\n\nSitemap: ${host}/sitemap.xml\n`,
    'sitemap.xml': `<urlset><url><loc>${host}/</loc></url><url><loc>${host}/pdf.html</loc></url></urlset>\n`,
    'llms.txt': `# Site\n- [PDF](${host}/pdf.html)\n`,
    'assets/styles.css': 'body{}',
    'README.md': '# internal',
    'check-docs.mjs': '// internal',
    'CNAME': 'example.test',
    'adr/0001-internal.md': '# internal',
    ...overrides,
  };
  for (const [file, contents] of Object.entries(files)) {
    if (contents === undefined) continue;
    mkdirSync(dirname(join(sourceRoot, file)), { recursive: true });
    writeFileSync(join(sourceRoot, file), contents);
  }
  return { sourceRoot, outputRoot: join(root, 'publish') };
};

const build = (paths) => prepareSite({ ...paths, site: host });

test('the real site builds the same allowlisted tree twice and writes a manifest artifact', (t) => {
  const outputs = [temporaryDirectory(t), temporaryDirectory(t)].map((parent) => join(parent, 'publish'));
  for (const output of outputs) {
    const run = spawnSync(process.execPath, [builder, output], { cwd: repositoryRoot, encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
  }
  const [first, second] = outputs.map(manifest);
  assert.equal(first, second, 'Two builds of the same docs must be byte-identical');

  const published = listFiles(outputs[0]);
  const pages = readdirSync(docsRoot).filter((name) => name.endsWith('.html')).sort();
  const assets = listFiles(join(docsRoot, 'assets')).map((file) => `assets/${file}`);
  assert.deepEqual(published, [...pages, ...assets, '_redirects', 'llms.txt', 'og.jpg', 'robots.txt', 'sitemap.xml'].sort());
  assert.ok(pages.length > 5, 'Every page in docs/ is published, not a fixed handful');
  assert.ok(published.every((file) => !/\.(md|mjs)$/.test(file) && file !== 'CNAME'), 'Internal docs stay out of the site');

  const locations = [...readFileSync(join(outputs[0], 'sitemap.xml'), 'utf8').matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => match[1]);
  for (const location of locations) {
    const path = new URL(location).pathname.slice(1) || 'index.html';
    assert.ok(published.includes(path), `${location} in sitemap.xml must be published`);
  }
  assert.equal(readFileSync(join(outputs[0], '_redirects'), 'utf8'), '/docs / 301\n/docs/ / 301\n/docs/* /:splat 301\n');

  mkdirSync(dirname(artifactPath), { recursive: true });
  writeFileSync(artifactPath, first);
});

test('Netlify builds the site with the default output and GitHub Pages no longer claims the domain', () => {
  const config = readFileSync(join(repositoryRoot, 'netlify.toml'), 'utf8');
  assert.match(config, /^\s*command = "node scripts\/prepare-netlify-docs\.mjs"$/m);
  assert.match(config, /^\s*publish = "\.netlify\/publish"$/m);
  assert.equal(existsSync(join(docsRoot, 'CNAME')), false, `docs/CNAME would point GitHub Pages at ${new URL(site).host} too`);
});

test('a fixture site publishes its pages, root files and assets but no internal files', (t) => {
  const paths = fixtureSite(t);
  build(paths);
  assert.deepEqual(listFiles(paths.outputRoot), ['_redirects', 'assets/styles.css', 'index.html', 'llms.txt', 'og.jpg', 'pdf.html', 'robots.txt', 'sitemap.xml']);
  assert.equal(readFileSync(join(paths.outputRoot, 'index.html'), 'utf8'), readFileSync(join(paths.sourceRoot, 'index.html'), 'utf8'));
});

test('fails when a page links to a file that is not published', (t) => {
  const paths = fixtureSite(t, { 'index.html': page('index.html', '<a href="missing.html">Gone</a>') });
  assert.throws(() => build(paths), /index\.html references a missing local file: missing\.html/);
});

test('fails when a page links to an internal file that is kept out of the site', (t) => {
  const paths = fixtureSite(t, { 'index.html': page('index.html', '<a href="README.md">Readme</a>') });
  assert.throws(() => build(paths), /index\.html references a missing local file: README\.md/);
});

test('fails when a page reaches outside the site', (t) => {
  const paths = fixtureSite(t, { 'index.html': page('index.html', '<img src="../secret.png">') });
  assert.throws(() => build(paths), /outside the site/);
});

test('fails when a canonical or social URL uses another host', (t) => {
  const paths = fixtureSite(t, { 'pdf.html': page('pdf.html').replace(`${host}/og.jpg`, 'https://lazzyms.github.io/toolbox/og.jpg') });
  assert.throws(() => build(paths), /pdf\.html.*https:\/\/lazzyms\.github\.io\/toolbox\/og\.jpg/);
});

test('fails when a page has no canonical URL', (t) => {
  const paths = fixtureSite(t, { 'pdf.html': page('pdf.html').replace(/<link rel="canonical"[^>]+>/, '') });
  assert.throws(() => build(paths), /pdf\.html needs a canonical URL/);
});

test('fails when sitemap.xml or llms.txt lists a page that is not published', (t) => {
  const sitemap = fixtureSite(t, { 'sitemap.xml': `<urlset><url><loc>${host}/gone.html</loc></url></urlset>\n` });
  assert.throws(() => build(sitemap), /sitemap\.xml lists .*gone\.html/);
  const llms = fixtureSite(t, { 'llms.txt': `- [Gone](${host}/gone.html)\n` });
  assert.throws(() => build(llms), /llms\.txt lists .*gone\.html/);
});

test('fails when a required root file is missing', (t) => {
  const paths = fixtureSite(t, { 'og.jpg': undefined });
  assert.throws(() => build(paths), /docs\/og\.jpg/);
});

test('fails when the assets tree contains a symbolic link', (t) => {
  const paths = fixtureSite(t);
  symlinkSync(join(paths.sourceRoot, 'README.md'), join(paths.sourceRoot, 'assets/linked.md'));
  assert.throws(() => build(paths), /symbolic link: assets\/linked\.md/);
});

test('a rebuild removes files a previous build published', (t) => {
  const paths = fixtureSite(t);
  build(paths);
  writeFileSync(join(paths.outputRoot, 'stale.html'), 'old');
  build(paths);
  assert.equal(existsSync(join(paths.outputRoot, 'stale.html')), false);
});

test('a failed build leaves no partial site behind', (t) => {
  const paths = fixtureSite(t, { 'index.html': page('index.html', '<a href="missing.html">Gone</a>') });
  assert.throws(() => build(paths));
  assert.equal(existsSync(paths.outputRoot), false);
});

test('refuses an output directory that would delete the docs or the repository', (t) => {
  const paths = fixtureSite(t);
  for (const outputRoot of [paths.sourceRoot, join(paths.sourceRoot, 'publish'), dirname(paths.sourceRoot)]) {
    assert.throws(() => build({ ...paths, outputRoot }), /must be outside/);
  }
  assert.ok(existsSync(join(paths.sourceRoot, 'index.html')), 'The docs must survive a refused build');
});
