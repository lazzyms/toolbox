export type PageSpan = { first: number; last: number | null };

export type Parsed<T> = { ok: true; value: T } | { ok: false; message: string };

const EXAMPLE = "1-3, 5, 8-";
const LARGEST_PAGE = 2n ** 64n - 1n;

const pagesLabel = (count: number) => (count === 1 ? "1 page" : `${count} pages`);

const describe = ({ first, last }: PageSpan) => {
  if (last === first) return `page ${first}`;
  return last === null ? `pages ${first}-` : `pages ${first}-${last}`;
};

const pageNumber = (entry: string, value: string): Parsed<number> => {
  if (!/^[0-9]+$/.test(value) || BigInt(value) > LARGEST_PAGE) {
    return { ok: false, message: `"${entry}" isn't a page or range. Use pages and ranges like ${EXAMPLE}.` };
  }
  const page = Number(value);
  if (page === 0) return { ok: false, message: `"${entry}" uses page 0. Page numbers start at 1.` };
  return { ok: true, value: page };
};

const parseEntry = (index: number, entry: string): Parsed<PageSpan> => {
  if (!entry) return { ok: false, message: `Entry ${index + 1} of the page range is empty. Remove the extra comma.` };
  const dash = entry.indexOf("-");
  const first = pageNumber(entry, dash < 0 ? entry : entry.slice(0, dash).trim());
  if (!first.ok) return first;
  if (dash < 0) return { ok: true, value: { first: first.value, last: first.value } };
  const lastText = entry.slice(dash + 1).trim();
  if (!lastText) return { ok: true, value: { first: first.value, last: null } };
  const last = pageNumber(entry, lastText);
  if (!last.ok) return last;
  if (last.value < first.value) {
    return { ok: false, message: `"${entry}" runs backwards. Put the lower page first, for example ${last.value}-${first.value}.` };
  }
  return { ok: true, value: { first: first.value, last: last.value } };
};

export const parsePageRanges = (raw: string): Parsed<PageSpan[]> => {
  const entries = raw.split(",").map((entry) => entry.trim());
  if (entries.every((entry) => !entry)) {
    return { ok: false, message: `Enter at least one page or range, for example ${EXAMPLE}.` };
  }
  const spans: PageSpan[] = [];
  for (const [index, entry] of entries.entries()) {
    const span = parseEntry(index, entry);
    if (!span.ok) return span;
    spans.push(span.value);
  }
  return { ok: true, value: spans };
};

export const pageRangesWithin = (spans: readonly PageSpan[], pageCount: number): Parsed<[number, number][]> => {
  const resolved: [number, number][] = [];
  for (const span of spans) {
    const last = span.last ?? pageCount;
    if (span.first > pageCount || last > pageCount) {
      return { ok: false, message: `The PDF has ${pagesLabel(pageCount)}, so ${describe(span)} can't be used.` };
    }
    resolved.push([span.first, last]);
  }
  return { ok: true, value: resolved };
};

/** Writes spans the way people type them, like "1-3, 5, 8-". */
export const formatPageSpans = (spans: readonly PageSpan[]) =>
  spans.map(({ first, last }) => (last === first ? String(first) : `${first}-${last ?? ""}`)).join(", ");

/** The same spans in page order, which is the order they run in. */
export const spansInPageOrder = (spans: readonly PageSpan[]) =>
  [...spans].sort((left, right) => left.first - right.first || (left.last ?? Infinity) - (right.last ?? Infinity));

/** The 0-based pages of a document the spans cover, leaving out pages past its end. */
export const pagesWithin = (spans: readonly PageSpan[], pageCount: number) => {
  const pages = new Set<number>();
  for (const { first, last } of spans) {
    for (let page = first; page <= Math.min(last ?? pageCount, pageCount); page += 1) pages.add(page - 1);
  }
  return [...pages].sort((left, right) => left - right);
};
