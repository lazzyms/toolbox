export type PageSpan = { first: number; last: number | null };

const EXAMPLE = "1-3, 5, 8-";

const pageNumber = (entry: string, value: string): number | string => {
  if (!/^\d+$/.test(value)) return `${entry} isn't a page or range. Use pages and ranges like ${EXAMPLE}.`;
  const page = Number(value);
  if (page === 0) return `${entry} uses page 0. Page numbers start at 1.`;
  return Number.isSafeInteger(page) ? page : `${entry} isn't a page or range. Use pages and ranges like ${EXAMPLE}.`;
};

export const parsePageRanges = (raw: string): { spans: PageSpan[] } | { error: string } => {
  const entries = raw.split(",").map((entry) => entry.trim());
  if (entries.every((entry) => !entry)) return { error: `Enter at least one page or range, for example ${EXAMPLE}.` };
  const spans: PageSpan[] = [];
  for (const [index, entry] of entries.entries()) {
    if (!entry) return { error: `Entry ${index + 1} of the page range is empty. Remove the extra comma.` };
    const dash = entry.indexOf("-");
    const firstText = dash < 0 ? entry : entry.slice(0, dash).trim();
    const lastText = dash < 0 ? null : entry.slice(dash + 1).trim();
    const first = pageNumber(entry, firstText);
    if (typeof first === "string") return { error: first };
    let last: number | null = first;
    if (lastText === "") last = null;
    else if (lastText !== null) {
      const parsed = pageNumber(entry, lastText);
      if (typeof parsed === "string") return { error: parsed };
      last = parsed;
    }
    if (last !== null && last < first) return { error: `${entry} runs backwards. Write it as ${last}-${first}.` };
    spans.push({ first, last });
  }
  return { spans };
};

export const describeSpans = (spans: PageSpan[]) =>
  spans.map(({ first, last }) => (last === first ? `${first}` : `${first}-${last ?? ""}`)).join(", ");

export const pagesInSpans = (spans: PageSpan[], pageCount: number) => {
  const pages = new Set<number>();
  for (const { first, last } of spans) {
    for (let page = first; page <= Math.min(last ?? pageCount, pageCount); page += 1) pages.add(page - 1);
  }
  return [...pages].sort((left, right) => left - right);
};

export const isShortened = (spans: PageSpan[], pageCount: number) =>
  spans.some(({ first, last }) => first > pageCount || (last !== null && last > pageCount));

export const listPages = (pages: number[]) => {
  const numbers = pages.map((page) => String(page + 1));
  return numbers.length > 1 ? `${numbers.slice(0, -1).join(", ")} and ${numbers[numbers.length - 1]}` : numbers.join("");
};
