import { pageRangesWithin, parsePageRanges } from "./pageRanges";

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

  const parsed = parsePageRanges(value);
  const resolved = parsed.ok ? pageRangesWithin(parsed.value, pageCount) : parsed;
  if (!resolved.ok) return { field: "pageRange", message: resolved.message };

  const pages = new Set<number>();
  for (const [first, last] of resolved.value) {
    for (let page = first; page <= last; page += 1) pages.add(page - 1);
  }
  return [...pages].sort((left, right) => left - right);
};

export const selectedPageLabel = (count: number) => `${count} ${count === 1 ? "page" : "pages"} selected`;
