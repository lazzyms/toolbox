import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const reactPath = '/node_modules/.vite/deps/react.js';
    const domPath = '/node_modules/.vite/deps/react-dom_client.js';
    const canvasPath = '/src/features/pdf-editor/SceneCanvas.tsx';
    const { default: React } = await import(reactPath);
    const { default: { createRoot } } = await import(domPath);
    const { SceneCanvas } = await import(canvasPath);
    const host = document.createElement('div');
    Object.assign(host.style, { position: 'fixed', inset: '0', zIndex: '9999', background: 'white' });
    document.body.append(host);
    function Harness() {
      const [page, setPage] = React.useState({ id: 'p', sourceIndex: null, width: 600, height: 800, rotation: 0, crop: null, objects: [] });
      const [tool, setTool] = React.useState('text');
      const [selectedId, onSelect] = React.useState(null);
      (window as any).canvasState = page;
      (window as any).setCanvasTool = setTool;
      (window as any).setCanvasPage = setPage;
      return React.createElement(SceneCanvas, { page, tool, selectedId, onSelect, zoom: 1, sourcePreview: null, renderedPreview: null,
        onInteraction: () => {}, onCommit: (next: any) => { (window as any).commits = ((window as any).commits ?? 0) + 1; setPage(next); },
        makeObject: (kind: string, rect: any) => ({ id: crypto.randomUUID(), kind, rect, text: 'Local text', fontSize: 18, color: '#202020', opacity: 1, strokes: [] }),
      });
    }
    createRoot(host).render(React.createElement(Harness));
  });
});

test('canvas pointer gestures create drag resize and cancel without extra history', async ({ page }) => {
  const canvas = page.getByRole('group', { name: 'PDF page canvas' });
  const b = (await canvas.boundingBox())!;
  await page.mouse.move(b.x + b.width * .2, b.y + b.height * .2);
  await page.mouse.down(); await page.mouse.move(b.x + b.width * .5, b.y + b.height * .3, { steps: 8 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).commits)).toBe(1);
  const object = page.getByRole('button', { name: 'text object 1' });
  const ob = (await object.boundingBox())!;
  await page.mouse.move(ob.x + 10, ob.y + 10); await page.mouse.down(); await page.mouse.move(ob.x + 40, ob.y + 40, { steps: 5 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).commits)).toBe(2);
  const handle = page.getByRole('button', { name: 'Resize text se' });
  const hb = (await handle.boundingBox())!;
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2); await page.mouse.down(); await page.mouse.move(hb.x + 50, hb.y + 30, { steps: 5 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).commits)).toBe(3);
  expect(await page.evaluate(() => (window as any).canvasState.objects[0].rect.width)).toBeGreaterThan(200);
  await canvas.dispatchEvent('pointerdown', { pointerId: 99, button: 0, clientX: b.x + 30, clientY: b.y + 30 });
  await canvas.dispatchEvent('pointercancel', { pointerId: 99 });
  expect(await page.evaluate(() => (window as any).commits)).toBe(3);
});

test('canvas maps drawing through crop and rotation into source coordinates', async ({ page }) => {
  await page.evaluate(() => {
    (window as any).setCanvasPage({ id: 'p', sourceIndex: null, width: 600, height: 800, crop: { x: 100, y: 150, width: 300, height: 400 }, rotation: 90, objects: [] });
    (window as any).setCanvasTool('shape');
  });
  const b = (await page.getByRole('group', { name: 'PDF page canvas' }).boundingBox())!;
  await page.mouse.move(b.x + b.width * .2, b.y + b.height * .2); await page.mouse.down();
  await page.mouse.move(b.x + b.width * .4, b.y + b.height * .4, { steps: 4 }); await page.mouse.up();
  const rect = await page.evaluate(() => (window as any).canvasState.objects[0].rect);
  expect(rect.x).toBeCloseTo(160, 0); expect(rect.y).toBeCloseTo(390, 0);
  expect(rect.width).toBeCloseTo(60, 0); expect(rect.height).toBeCloseTo(80, 0);
});
