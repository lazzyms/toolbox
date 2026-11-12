// Bates numbering model. It mirrors the native set-aside rules in kit/pdf/bates.rs, so the file list can show each
// file's numbers before the run and the results can name the gaps a failed file leaves.

export type BatesPosition = "bottom-right" | "bottom-center" | "bottom-left" | "top-right";

export interface BatesFormat {
    prefix: string;
    suffix: string;
    start: number;
    digits: number;
    position: BatesPosition;
    fontFamily: string;
    fontSize: number;
}

export interface BatesRange {
    first: number;
    pages: number;
}

export const BATES_MAX_DIGITS = 12;
export const BATES_MAX_AFFIX = 40;
export const BATES_MAX_START = 999_999_999_999;

export const defaultBatesFormat: BatesFormat = {
    prefix: "",
    suffix: "",
    start: 1,
    digits: 6,
    position: "bottom-right",
    fontFamily: "Noto Sans",
    fontSize: 10,
};

export const batesPositions: { value: BatesPosition; label: string }[] = [
    { value: "bottom-right", label: "Bottom right" },
    { value: "bottom-center", label: "Bottom centre" },
    { value: "bottom-left", label: "Bottom left" },
    { value: "top-right", label: "Top right" },
];

export const batesIdentifier = (format: Pick<BatesFormat, "prefix" | "suffix" | "digits">, number: number) =>
    `${format.prefix}${String(number).padStart(format.digits, "0")}${format.suffix}`;

export const batesRangeLabel = (format: Pick<BatesFormat, "prefix" | "suffix" | "digits">, range: BatesRange) =>
    range.pages === 1
        ? batesIdentifier(format, range.first)
        : `${batesIdentifier(format, range.first)} to ${batesIdentifier(format, range.first + range.pages - 1)}`;

/** Sets numbers aside in file order. A file whose page count is unknown (null) gets no range; the next file continues. */
export const assignBatesRanges = (start: number, pageCounts: readonly (number | null)[]): (BatesRange | null)[] => {
    let next = start;
    return pageCounts.map((pages) => {
        if (pages === null || pages < 1) return null;
        const range = { first: next, pages };
        next += pages;
        return range;
    });
};

/** The first problem with the format, worded for the field it belongs to, or null when the native run will accept it. */
export const batesFormatIssue = (format: BatesFormat): string | null => {
    for (const [name, value] of [["prefix", format.prefix], ["suffix", format.suffix]] as const) {
        if ([...value].length > BATES_MAX_AFFIX) return `The ${name} can be at most ${BATES_MAX_AFFIX} characters.`;
        // eslint-disable-next-line no-control-regex
        if (/[\u0000-\u001f\u007f]/.test(value)) return `The ${name} can't contain line breaks or tabs.`;
    }
    if (!Number.isInteger(format.start) || format.start < 0 || format.start > BATES_MAX_START) return `Start at must be a whole number from 0 to ${BATES_MAX_START}.`;
    if (!Number.isInteger(format.digits) || format.digits < 1 || format.digits > BATES_MAX_DIGITS) return `Digits must be from 1 to ${BATES_MAX_DIGITS}.`;
    if (!Number.isFinite(format.fontSize) || format.fontSize < 6 || format.fontSize > 72) return "Size must be from 6 to 72 points.";
    return null;
};

/** The ranges a run left unused: the numbers set aside for files that failed. */
export const unusedBatesRanges = (ranges: readonly (BatesRange | null)[], failed: readonly boolean[]) =>
    ranges.filter((range, index): range is BatesRange => range !== null && failed[index]);
