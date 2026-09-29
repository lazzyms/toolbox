import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const readJson = (path) => JSON.parse(readFileSync(resolve(projectRoot, path), 'utf8'));
const config = readJson('src-tauri/tauri.conf.json');
const defaultCapability = readJson('src-tauri/capabilities/default.json');
const cargoManifest = readFileSync(resolve(projectRoot, 'src-tauri/Cargo.toml'), 'utf8');

function sourcesFor(policy, directive) {
  const entry = policy.split(';').map((part) => part.trim()).find((part) => part.startsWith(`${directive} `));
  assert.ok(entry, `CSP must define ${directive}`);
  return new Set(entry.split(/\s+/).slice(1));
}

test('enables the Tauri v2 asset protocol without a broad static filesystem scope', () => {
  assert.equal(config.$schema, 'https://schema.tauri.app/config/2');
  assert.deepEqual(config.app.security.assetProtocol, { enable: true, scope: [] });
  assert.match(cargoManifest, /^tauri = \{ version = "2", features = \["protocol-asset"\] \}$/m);

  const imgSources = sourcesFor(config.app.security.csp, 'img-src');
  for (const source of ["'self'", 'asset:', 'http://asset.localhost', 'data:']) {
    assert.ok(imgSources.has(source), `img-src must allow ${source}`);
  }
  assert.ok(!imgSources.has('*'), 'img-src must not use a wildcard source');
});

test('relies on the existing dialog-selected file scope for previews', () => {
  assert.deepEqual(defaultCapability.windows, ['main']);
  assert.ok(defaultCapability.permissions.includes('dialog:default'));
  assert.deepEqual(config.app.security.assetProtocol.scope, [], 'only runtime-selected files may enter the asset scope');
});
