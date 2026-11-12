import { pagePositionsLabel, positionsList } from './pageSelection';
import type { ScenePage } from './scene';

/** What `classify_pdf_blank_pages` returns for one requested source page. */
export type BlankPageClassification = { pageIndex: number; blank: boolean; note: string | null };
/** Source pages, by 0-based index: those found blank, and those that couldn't be checked. */
export type BlankPageReport = { blank: number[]; unchecked: number[] };
export type BlankPageFinder = {
  /** Classifies the source pages, reusing earlier answers. Resolves null once disposed; rejects when the helper fails. */
  find: (sourceIndices: readonly number[]) => Promise<BlankPageReport | null>;
  dispose: () => void;
};

/** Pages per helper request. Native accepts up to 12. */
export const BLANK_PAGE_BATCH_SIZE = 12;

/**
 * Classifies source pages for "Blank pages" one batch at a time, so at most one helper request is in flight.
 * Answers are kept for this document, so asking again only checks pages that weren't checked yet. A page that couldn't
 * be checked isn't kept, so the next run tries it again. Disposing is the cancel handle: later batches are never sent.
 */
export const createBlankPageFinder = ({ fetch, batchSize = BLANK_PAGE_BATCH_SIZE }: {
  fetch: (pageIndices: number[]) => Promise<BlankPageClassification[]>;
  batchSize?: number;
}): BlankPageFinder => {
  const answers = new Map<number, boolean>();
  let disposed = false;
  return {
    find: async (sourceIndices) => {
      const wanted = [...new Set(sourceIndices)];
      const missing = wanted.filter((index) => !answers.has(index));
      const unchecked = new Set<number>();
      for (let start = 0; start < missing.length; start += Math.max(1, batchSize)) {
        if (disposed) return null;
        const batch = missing.slice(start, start + Math.max(1, batchSize));
        let results: BlankPageClassification[];
        try {
          results = await fetch(batch);
        } catch (error) {
          if (disposed) return null;
          throw error;
        }
        if (disposed) return null;
        const byIndex = new Map(results.map((result) => [result.pageIndex, result]));
        for (const index of batch) {
          const result = byIndex.get(index);
          if (result && result.note === null) answers.set(index, result.blank); else unchecked.add(index);
        }
      }
      if (disposed) return null;
      return { blank: wanted.filter((index) => answers.get(index) === true), unchecked: wanted.filter((index) => unchecked.has(index)) };
    },
    dispose: () => { disposed = true; },
  };
};

/**
 * The current pages that are blank: their source page is blank and nothing was added to them.
 * Every copy of a blank source page counts, wherever it sits now.
 */
export const blankScenePageIds = (pages: readonly Pick<ScenePage, 'id' | 'sourceIndex' | 'objects'>[], blankSources: readonly number[]) => {
  const blank = new Set(blankSources);
  return pages.filter((page) => page.sourceIndex !== null && blank.has(page.sourceIndex) && page.objects.length === 0).map((page) => page.id);
};

/** The chip on the Blank pages button after a check that found pages. */
export const blankPagesFoundLabel = (count: number) => (count === 0 ? null : `${count} found`);

/**
 * Says what "Blank pages" did, with 1-based positions:
 * "3 blank pages found and selected: 7, 11 and 16. Nothing was removed."
 */
export const blankPagesAnnouncement = (positions: readonly number[], uncheckedPositions: readonly number[] = []) => {
  const found = positions.length === 0 ? 'No blank pages found.'
    : `${positions.length} blank ${positions.length === 1 ? 'page' : 'pages'} found and selected: ${positionsList(positions)}.`;
  const unchecked = uncheckedPositions.length === 0 ? ''
    : ` ${pagePositionsLabel(uncheckedPositions)} couldn't be checked, so ${uncheckedPositions.length === 1 ? "it wasn't" : "they weren't"} selected.`;
  return `${found}${unchecked} Nothing was removed.`;
};

export const blankPagesRunningAnnouncement = 'Looking for blank pages…';
export const blankPagesFailedAnnouncement = "Couldn't look for blank pages. The selection didn't change, and nothing was removed.";
