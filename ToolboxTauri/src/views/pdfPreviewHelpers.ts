import type { ScenePreview } from '../features/pdf-editor/scene';

export type PreviewRenderSettings = Readonly<{ maxDimension: number }>;
export type PreviewCache = {
  get: (key: string) => ScenePreview | undefined;
  set: (key: string, value: ScenePreview) => void;
  clear: () => void;
};
export type GenerationState = { current: number };
export type PdfEditorErrorPhase = 'open' | 'preview' | 'export';

export const PREVIEW_CACHE_MAX_ENTRIES = 12;
export const PREVIEW_RENDER_SETTINGS: PreviewRenderSettings = { maxDimension: 1600 };
export const previewCacheKey = (path: string, pageIndex: number, serializedScene: string, renderSettings: PreviewRenderSettings) =>
  JSON.stringify([path, pageIndex, serializedScene, renderSettings]);

const errorText = (reason: unknown) => {
  const text = reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : String(reason);
  return text.replace(/^(?:Error:\s*)+/i, '').trim();
};

export const pdfEditorErrorMessage = (reason: unknown, phase: PdfEditorErrorPhase) => {
  const text = errorText(reason);
  if (/bookmarks? or destinations?/i.test(text)) {
    return 'This PDF contains bookmarks or destinations that cannot be preserved after this page edit. Undo the crop or rotation, or use a copy without them.';
  }
  if (/annotations?|form widgets?/i.test(text)) {
    return 'This PDF has annotations or form fields that cannot be preserved after this page edit. Undo the crop or rotation, or use a copy without them.';
  }
  if (/navigation or form structures?/i.test(text)) {
    return 'This page edit could break bookmarks or form fields. Undo it, or use a copy without those structures.';
  }
  if (/tagged pdf|structure mappings?|structtreeroot|parenttree/i.test(text)) {
    return 'This tagged PDF uses accessibility structure that cannot be preserved after this edit. Undo the page edit or use a copy without tagged structure.';
  }
  if (/nested or unsupported page tree|page-tree/i.test(text)) {
    return 'This PDF uses a page structure that Toolbox cannot edit safely. Use a copy with a standard page tree.';
  }
  if (phase === 'preview') return `Preview could not be verified. ${text || 'Try again or use a different copy.'}`;
  if (phase === 'export') return `This PDF could not be exported safely. ${text || 'Undo the last edit and try again.'}`;
  return `This PDF could not be opened. ${text || 'Try a different copy.'}`;
};
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
