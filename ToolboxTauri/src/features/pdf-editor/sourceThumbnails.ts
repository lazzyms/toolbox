import type { ScenePreview } from './scene';

/** What `render_pdf_source_thumbnails` returns for one requested source page. */
export type SourceThumbnail = { pageIndex: number; thumbnail: ScenePreview | null; error: string | null };
export type SourceThumbnailState = { status: 'ready'; value: ScenePreview } | { status: 'failed' } | { status: 'loading' };
export type SourceThumbnailQueue = {
  /** Replaces the pages to load, nearest first. Queued pages that are no longer wanted are dropped before they are requested. */
  want: (sourceIndices: readonly number[]) => void;
  get: (sourceIndex: number) => SourceThumbnailState;
  dispose: () => void;
};

/** Pages per helper request. Small batches let thumbnails appear while the person scrolls. Native accepts up to 12. */
export const SOURCE_THUMBNAIL_BATCH_SIZE = 6;
/** Thumbnails kept in memory for one open PDF. Scrolling back within this many pages needs no new render. */
export const SOURCE_THUMBNAIL_CACHE_MAX_ENTRIES = 240;

/**
 * Loads source-page thumbnails for the Organize grid one batch at a time, so at most one helper request is in flight.
 * The cache is a bounded LRU: wanted pages are refreshed, and the oldest pages that aren't wanted go first.
 * A page whose render failed stays failed for this document, so it shows a placeholder instead of retrying in a loop.
 */
export const createSourceThumbnailQueue = ({ fetch, onChange, batchSize = SOURCE_THUMBNAIL_BATCH_SIZE, maxEntries = SOURCE_THUMBNAIL_CACHE_MAX_ENTRIES }: {
  fetch: (pageIndices: number[]) => Promise<SourceThumbnail[]>;
  onChange: () => void;
  batchSize?: number;
  maxEntries?: number;
}): SourceThumbnailQueue => {
  const cache = new Map<number, ScenePreview>();
  const failed = new Set<number>();
  let wanted: number[] = [];
  let running = false;
  let disposed = false;
  const store = (index: number, value: ScenePreview) => {
    cache.delete(index);
    cache.set(index, value);
    const keep = new Set(wanted);
    for (const oldest of cache.keys()) {
      if (cache.size <= Math.max(1, maxEntries)) break;
      if (!keep.has(oldest)) cache.delete(oldest);
    }
  };
  const pump = async () => {
    if (running || disposed) return;
    const batch = wanted.filter((index) => !cache.has(index) && !failed.has(index)).slice(0, Math.max(1, batchSize));
    if (!batch.length) return;
    running = true;
    try {
      const results = await fetch(batch);
      if (disposed) return;
      const byIndex = new Map(results.map((result) => [result.pageIndex, result]));
      for (const index of batch) {
        const thumbnail = byIndex.get(index)?.thumbnail;
        if (thumbnail) store(index, thumbnail); else failed.add(index);
      }
    } catch {
      if (disposed) return;
      for (const index of batch) failed.add(index);
    } finally {
      running = false;
    }
    onChange();
    void pump();
  };
  return {
    want: (sourceIndices) => {
      wanted = [...new Set(sourceIndices)];
      for (const index of wanted) {
        const value = cache.get(index);
        if (value) { cache.delete(index); cache.set(index, value); }
      }
      void pump();
    },
    get: (sourceIndex) => {
      const value = cache.get(sourceIndex);
      return value ? { status: 'ready', value } : failed.has(sourceIndex) ? { status: 'failed' } : { status: 'loading' };
    },
    dispose: () => { disposed = true; wanted = []; },
  };
};
