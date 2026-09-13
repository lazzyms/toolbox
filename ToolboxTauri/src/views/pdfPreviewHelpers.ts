import type { ScenePreview } from '../features/pdf-editor/scene';

export type PreviewRenderSettings = Readonly<{ maxDimension: number }>;
export type PreviewCache = {
  get: (key: string) => ScenePreview | undefined;
  set: (key: string, value: ScenePreview) => void;
  clear: () => void;
};
export type GenerationState = { current: number };

export const PREVIEW_CACHE_MAX_ENTRIES = 12;
export const PREVIEW_RENDER_SETTINGS: PreviewRenderSettings = { maxDimension: 1600 };
export const previewCacheKey = (path: string, pageIndex: number, serializedScene: string, renderSettings: PreviewRenderSettings) =>
  JSON.stringify([path, pageIndex, serializedScene, renderSettings]);
export const createPreviewCache = (maxEntries = PREVIEW_CACHE_MAX_ENTRIES): PreviewCache => {
  const limit = Math.max(1, Math.floor(maxEntries));
  const entries = new Map<string, ScenePreview>();
  return {
    get: (key) => {
      const value = entries.get(key);
      if (value) { entries.delete(key); entries.set(key, value); }
      return value;
    },
    set: (key, value) => {
      if (entries.has(key)) entries.delete(key);
      else if (entries.size >= limit) {
        const oldest = entries.keys().next().value;
        if (oldest !== undefined) entries.delete(oldest);
      }
      entries.set(key, value);
    },
    clear: () => entries.clear(),
  };
};
export const requestWithGeneration = async <T,>(generation: GenerationState, requestGeneration: number, request: () => Promise<T>, onValue: (value: T) => void, onError: (reason: unknown) => void, onFinally: () => void) => {
  try {
    const value = await request();
    if (generation.current === requestGeneration) onValue(value);
  } catch (reason) {
    if (generation.current === requestGeneration) onError(reason);
  } finally {
    if (generation.current === requestGeneration) onFinally();
  }
};
