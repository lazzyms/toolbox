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
