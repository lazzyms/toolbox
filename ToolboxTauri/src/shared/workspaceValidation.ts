export type ValidationIssue = {
  field: string;
  message: string;
};

export const positiveInteger = (value: number, field: string): ValidationIssue | null =>
  Number.isInteger(value) && value > 0
    ? null
    : { field, message: `${field} must be a positive whole number.` };

export const boundedNumber = (
  value: number,
  field: string,
  minimum: number,
  maximum: number,
): ValidationIssue | null =>
  Number.isFinite(value) && value >= minimum && value <= maximum
    ? null
    : { field, message: `${field} must be between ${minimum} and ${maximum}.` };

export const parsePageRange = (value: string, pageCount: number): number[] | ValidationIssue => {
  if (!value.trim()) return Array.from({ length: pageCount }, (_, index) => index);

  const pages = new Set<number>();
  for (const token of value.split(",").map((part) => part.trim()).filter(Boolean)) {
    const match = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(token);
    if (!match) return { field: "pageRange", message: `Invalid page selection: ${token}.` };

    const start = Number(match[1]);
    const end = Number(match[2] ?? match[1]);
    if (start < 1 || end < start || end > pageCount) {
      return { field: "pageRange", message: `Pages must be between 1 and ${pageCount}.` };
    }
    for (let page = start; page <= end; page += 1) pages.add(page - 1);
  }

  return [...pages].sort((left, right) => left - right);
};

export const selectedPageLabel = (count: number) => `${count} ${count === 1 ? "page" : "pages"} selected`;

/**
 * Checks a Split PDF range string without knowing the page count. The messages match
 * the native parser so the inline alert and a rejected run read the same.
 */
export const splitRangesIssue = (value: string): string | null => {
  if (!value.trim()) return "Enter at least one page range to split.";
  const isPage = (text: string) => /^\d+$/.test(text);
  const problems = value.split(",").map((entry) => entry.trim()).flatMap((token, index) => {
    if (!token) return [`Entry ${index + 1} is empty.`];
    const dash = token.indexOf("-");
    if (dash < 0) {
      if (!isPage(token)) return [`${token} isn't a page.`];
      return Number(token) === 0 ? [`${token} isn't a page. Pages start at 1.`] : [];
    }
    const start = token.slice(0, dash).trim();
    const end = token.slice(dash + 1).trim();
    if (!isPage(start) || (end !== "" && !isPage(end))) return [`${token} isn't a range.`];
    if (Number(start) === 0 || (end !== "" && Number(end) === 0)) return [`${token} isn't a range. Pages start at 1.`];
    if (end !== "" && Number(start) > Number(end)) return [`${token} runs backwards (write ${Number(end)}-${Number(start)}).`];
    return [];
  });
  return problems.length ? problems.join(" ") : null;
};
