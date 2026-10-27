import assert from 'node:assert/strict';
import test from 'node:test';
import { createPreviewCache, previewCacheKey, requestWithGeneration } from '../src/views/pdfPreviewHelpers';

const preview = (id: string) => ({ dataUrl: `data:image/png;base64,${id}`, width: 1, height: 1 });

test('preview cache evicts the least recently used entry at its bound', () => {
  const cache = createPreviewCache(2);
  cache.set('first', preview('first'));
  cache.set('second', preview('second'));

  assert.equal(cache.get('first')?.dataUrl, preview('first').dataUrl);
  cache.set('third', preview('third'));

  assert.equal(cache.get('first')?.dataUrl, preview('first').dataUrl);
  assert.equal(cache.get('second'), undefined);
  assert.equal(cache.get('third')?.dataUrl, preview('third').dataUrl);
});

test('preview cache keys include every render identity component', () => {
  const settings = { maxDimension: 1600 };
  const key = previewCacheKey('/local/document.pdf', 1, '{"pages":[]}', settings);

  assert.equal(key, previewCacheKey('/local/document.pdf', 1, '{"pages":[]}', settings));
  assert.notEqual(key, previewCacheKey('/other/document.pdf', 1, '{"pages":[]}', settings));
  assert.notEqual(key, previewCacheKey('/local/document.pdf', 2, '{"pages":[]}', settings));
  assert.notEqual(key, previewCacheKey('/local/document.pdf', 1, '{"pages":[1]}', settings));
  assert.notEqual(key, previewCacheKey('/local/document.pdf', 1, '{"pages":[]}', { maxDimension: 800 }));
});

test('requestWithGeneration suppresses stale value, error, and finally callbacks', async () => {
  const generation = { current: 1 };
  let resolveRequest!: (value: string) => void;
  const events: string[] = [];
  const pending = new Promise<string>((resolve) => { resolveRequest = resolve; });
  const request = requestWithGeneration(generation, 1, () => pending,
    (value) => events.push(`value:${value}`),
    () => events.push('error'),
    () => events.push('finally'));

  generation.current = 2;
  resolveRequest('stale');
  await request;

  assert.deepEqual(events, []);
});

test('requestWithGeneration delivers callbacks for the current generation', async () => {
  const events: string[] = [];
  await requestWithGeneration({ current: 3 }, 3, async () => 'fresh',
    (value) => events.push(`value:${value}`),
    () => events.push('error'),
    () => events.push('finally'));

  assert.deepEqual(events, ['value:fresh', 'finally']);
});
