import type { ErrorKind, FontSource, JobOutcome, JpegColour, OutcomePointer, OutcomeWarning, PackNeed, SucceededJobOutcome, ToolError } from "../contracts";
import { UtilityRegistry } from "../registry";
import { formatBatchSaving, formatBytes, formatSizeChange } from "../shared/fileSize";

export type RowTone = "ok" | "warn" | "note" | "skip" | "fail";

export type RowPointer = { label: string; pointer: OutcomePointer };

type PackPointer = Extract<OutcomePointer, { kind: "pack" }>;

export interface RowView {
    name: string;
    fullPath: string;
    tone: RowTone;
    stateLabel: string;
    spokenState: string;
    detailLines: RowDetail[];
    openOutput: { path: string; label: "Open" | "Open first" } | null;
    revealOutput: string | null;
    pointer: RowPointer | null;
    canRetry: boolean;
    canRetryWithPassword: boolean;
    fonts: RowFont[];
    /** Set when the file failed only for want of a pack that has since been installed. */
    packInstalled: boolean;
}

export type RowFont = { requested: string; used: string; source: string };

export type RowDetail = { text: string; tone: "warn" | null };

const fontSources: Record<FontSource, string> = {
    embedded: "Embedded in the file",
    "bundled fallback": "Bundled fallback, same widths",
    system: "System font on this device",
    "bundled default": "Bundled default font",
};

export const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/** One part of a grouped result: a single file, or a numbered set of files shown as one row. */
export interface OutputGroupPart {
    kind: "file" | "set";
    name: string;
    hint: string;
    /** The file Open acts on, or null when the part has no single file to open. */
    open: string | null;
    reveal: string;
}

/** A succeeded file whose several outputs show as one row that expands into its parts. */
export interface OutputGroup {
    hint: string;
    /** The output the row's own Open and Reveal act on. */
    primary: string;
    parts: OutputGroupPart[];
}

export type OutputGrouping = (outcome: SucceededJobOutcome) => OutputGroup | null;

const isRetryable = (failure: ToolError): boolean => {
    switch (failure.kind) {
        case "processing":
            return true;
        case "limitExceeded":
            // Size, page count, dimensions and expansion belong to the file, so only a run that ran out of time can pass on another try.
            return failure.details.limit === "time";
        default:
            return false;
    }
};

const spokenKinds: Record<ErrorKind, string> = {
    invalidInput: "Failed, the input can't be used",
    unavailable: "Unavailable",
    limitExceeded: "Failed, over the limit",
    processing: "Failed while processing",
    wrongPassword: "Failed, wrong password",
    cancelled: "Failed, cancelled while it was running",
};

export const wrongPasswordDetail = "The password didn't open this file. Check it, or try the password this file was sent with.";

export const warningText = (warning: OutcomeWarning): string => {
    switch (warning.code) {
        case "signatureRemoved":
            return "Saved. The digital signature was removed, because changing the file breaks it.";
        case "fontSubstituted":
            return `The font ${warning.details.requestedFamily} wasn’t available, so ${warning.details.actualFamily} was used.`;
        case "renamedToAvoidReplacing":
            return `Saved as ${warning.details.finalName}. Renamed to avoid replacing an existing file.`;
        case "precisionReduced":
            // The native side names what was reduced, such as "10-bit HEIC saved as 8-bit sRGB."
            return warning.message.trim() || "Some values were rounded to the precision this format can store.";
        case "bookmarksNotCarried":
            return `Bookmarks weren’t carried into ${warning.details.into}.`;
        case "tagsNotCarried":
            return `Accessibility tags weren’t carried into ${warning.details.into}.`;
        case "omittedContent":
        case "approximateLayout":
            return warning.message;
        case "imageUnsupported":
            return `${imagePlace(warning.details)} — ${warning.details.feature} isn't supported in this version. Not saved.`;
        case "imageDamaged":
            return `${imagePlace(warning.details)} — The image data is damaged. Not saved.`;
        case "imageTooLarge":
            return `${imagePlace(warning.details)} — Would be ${grouped(warning.details.width)} × ${grouped(warning.details.height)} px when decoded, over the size limit. Not saved.`;
        case "jpegColourKept":
            return jpegColourText(warning.details);
        case "pagesWithoutText":
            return warning.details.pages.length === 1
                ? `${pageList(warning.details.pages)} has no selectable text. OCR can read it.`
                : `${pageList(warning.details.pages)} have no selectable text. OCR can read them.`;
        case "transparencyFlattened":
            return "Transparent areas were flattened onto white, because JPEG has none.";
    }
};

const ocrPointer: OutcomePointer = { kind: "tool", toolId: "pdf-ocr" };

const imagePlace = ({ page, image }: { page: number; image: number }) => `Page ${page}, image ${image}`;

const grouped = (value: number) => value.toLocaleString("en-US");

const pageList = (pages: number[]) => {
    const runs: [number, number][] = [];
    for (const page of pages) {
        const last = runs[runs.length - 1];
        if (last && last[1] + 1 === page) last[1] = page;
        else runs.push([page, page]);
    }
    const label = runs.map(([first, last]) => (first === last ? `${first}` : `${first}-${last}`)).join(", ");
    return `${pages.length === 1 ? "Page" : "Pages"} ${label}`;
};

const jpegColourText = ({ colour, pages, images }: { colour: JpegColour; pages: number[]; images: number }) => {
    const one = images === 1;
    const place = pageList(pages);
    if (colour === "cmyk") {
        return one
            ? `${place} — 1 JPEG uses CMYK colour and was copied as it is. Some viewers show it with the wrong colours.`
            : `${place} — ${images} JPEGs use CMYK colour and were copied as they are. Some viewers show them with the wrong colours.`;
    }
    return one
        ? `${place} — 1 inverted JPEG, copied as it is. Some viewers show it in negative.`
        : `${place} — ${images} inverted JPEGs, copied as they are. Some viewers show them in negative.`;
};

export const packVerbs: Record<PackNeed, string> = { notInstalled: "Install", damaged: "Repair", incompatible: "Update" };
/** Notes tell what was approximated or what another tool can still do for a file; they don't say anything went wrong. */
const isNote = (warning: OutcomeWarning) => warning.code === "approximateLayout" || warning.code === "pagesWithoutText";
/** Saved-as notes say how a finished file was saved. They show on its row after the detail but leave it done, not warned. */
const isSavedAs = (warning: OutcomeWarning) => warning.code === "transparencyFlattened";

export const warningDetail = (warning: OutcomeWarning): RowDetail => ({ text: warningText(warning), tone: warning.code === "omittedContent" ? "warn" : null });

const pointerView = (pointer: OutcomePointer | null): RowPointer | null => {
    if (!pointer) return null;
    if (pointer.kind === "pack") return { label: `${packVerbs[pointer.reason]} ${pointer.name} pack`, pointer };
    if (pointer.kind === "outputFolder") return { label: "Choose output folder", pointer };
    const tool = UtilityRegistry.find((item) => item.id === pointer.toolId);
    return tool ? { label: `Open ${tool.title}`, pointer } : null;
};

const outputSummary = (paths: string[]) =>
    paths.length === 1
        ? fileName(paths[0])
        : `${paths.length} files, ${fileName(paths[0])} to ${fileName(paths[paths.length - 1])}`;

const nonEmpty = (lines: (string | RowDetail)[]): RowDetail[] =>
    lines.map((line) => (typeof line === "string" ? { text: line, tone: null } : line)).filter((line) => line.text.trim().length > 0);

const andList = (items: string[]) => items.length > 1 ? `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}` : items.join("");

const withoutExtension = (name: string) => name.replace(/\.[^.]+$/, "");

type RenamedWarning = Extract<OutcomeWarning, { code: "renamedToAvoidReplacing" }>;

const renamedOutputsDetail = (inputPath: string, outputCount: number, outputNoun: string, renamed: RenamedWarning[]) => {
    const stem = withoutExtension(fileName(inputPath));
    const chosen = renamed.map(({ details }, index) => index > 0 && details.chosenName.startsWith(stem) ? details.chosenName.slice(stem.length) : details.chosenName);
    const endings = [...new Set(renamed.map(({ details }) => withoutExtension(details.finalName).slice(withoutExtension(details.chosenName).length)))];
    const ending = `${renamed.length === 1 ? "It ends" : "They end"} in ${endings.join(" or ")}.`;
    return `${outputCount} ${outputNoun}. ${renamed.length} renamed, because ${andList(chosen)} already existed. ${ending}`;
};

const sizeLine = (outcome: JobOutcome) =>
    outcome.originalBytes === undefined || outcome.outputBytes === undefined ? "" : formatSizeChange(outcome.originalBytes, outcome.outputBytes);

/** The pack a failed file needs before it can run, if that is why it failed. */
export const neededPack = (outcome: JobOutcome): PackPointer | null =>
    outcome.state === "failed" && outcome.failure.pointer?.kind === "pack" ? outcome.failure.pointer : null;

/**
 * A file that failed for want of a pack stays in the list without Retry, since running it again
 * without the pack fails the same way. Once that pack is installed, the row says the file is ready
 * to run again. `installedPacks` holds the packs installed, repaired or updated since the run.
 */
export const rowView = (outcome: JobOutcome, resultVerb: string, namedByOutput = false, outputNoun?: string, installedPacks: ReadonlySet<string> = new Set()): RowView => {
    const warnings = outcome.warnings.map(warningDetail);
    const sizes = sizeLine(outcome);
    const input = { name: fileName(outcome.inputPath), fullPath: outcome.inputPath };
    switch (outcome.state) {
        case "succeeded": {
            const first = outcome.outputPaths[0] ?? null;
            const flaggedWarnings = outcome.warnings.filter((warning) => !isSavedAs(warning));
            const cautions = flaggedWarnings.map(warningDetail);
            const notes = outcome.warnings.filter(isSavedAs).map(warningText);
            const warned = cautions.length > 0;
            const noted = warned && flaggedWarnings.every(isNote);
            const flagged = noted ? (cautions.length === 1 ? "a note" : "notes") : cautions.length === 1 ? "a warning" : "warnings";
            const pointsAtOcr = outcome.warnings.some((warning) => warning.code === "pagesWithoutText");
            const byOutput = namedByOutput && first !== null;
            const renamed = outcome.warnings.filter((warning): warning is RenamedWarning => warning.code === "renamedToAvoidReplacing");
            const fonts = (outcome.fonts ?? []).map((font) => ({ requested: font.requestedFamily, used: font.fontUsed, source: fontSources[font.source] }));
            if (outputNoun && renamed.length > 0 && renamed.length === outcome.warnings.length) {
                return {
                    ...input,
                    tone: "warn",
                    stateLabel: "Renamed",
                    spokenState: `${resultVerb}, ${renamed.length} renamed to avoid replacing existing files`,
                    detailLines: nonEmpty([renamedOutputsDetail(outcome.inputPath, outcome.outputPaths.length, outputNoun, renamed)]),
                    openOutput: first ? { path: first, label: outcome.outputPaths.length > 1 ? "Open first" : "Open" } : null,
                    revealOutput: first,
                    pointer: null,
                    canRetry: false,
                    canRetryWithPassword: false,
                    packInstalled: false,
                    fonts,
                };
            }
            return {
                ...(byOutput ? { name: fileName(first), fullPath: first } : input),
                tone: noted ? "note" : warned ? "warn" : "ok",
                stateLabel: resultVerb,
                spokenState: warned ? `${resultVerb} with ${flagged}` : resultVerb,
                detailLines: nonEmpty(warned ? [sizes, ...cautions, outcome.detail, ...notes] : [sizes, first && !byOutput && !outputNoun ? outputSummary(outcome.outputPaths) : "", outcome.detail, ...notes]),
                openOutput: first ? { path: first, label: outcome.outputPaths.length > 1 ? "Open first" : "Open" } : null,
                revealOutput: first,
                pointer: pointsAtOcr ? pointerView(ocrPointer) : null,
                canRetry: false,
                canRetryWithPassword: false,
                packInstalled: false,
                fonts,
            };
        }
        case "skipped":
            return {
                ...input,
                tone: "skip",
                stateLabel: "Skipped",
                spokenState: "Skipped",
                detailLines: nonEmpty([sizes, outcome.reason.message, ...warnings]),
                openOutput: null,
                revealOutput: null,
                pointer: pointerView(outcome.reason.pointer),
                canRetry: false,
                canRetryWithPassword: false,
                packInstalled: false,
                fonts: [],
            };
        case "failed": {
            const pack = neededPack(outcome);
            if (pack) {
                const installed = installedPacks.has(pack.packId);
                return {
                    ...input,
                    tone: installed ? "ok" : "fail",
                    stateLabel: installed ? "Ready to run" : "Needs a pack",
                    spokenState: installed ? `Ready to run, the ${pack.name} pack is installed` : `Needs the ${pack.name} pack`,
                    detailLines: nonEmpty(installed ? [`The ${pack.name} pack is installed now. Run again to read this file.`] : [outcome.failure.message, ...warnings]),
                    openOutput: null,
                    revealOutput: null,
                    pointer: installed ? null : pointerView(pack),
                    canRetry: false,
                    canRetryWithPassword: false,
                    packInstalled: installed,
                    fonts: [],
                };
            }
            const unavailable = outcome.failure.kind === "unavailable";
            const wrongPassword = outcome.failure.kind === "wrongPassword";
            return {
                ...input,
                tone: unavailable ? "skip" : "fail",
                stateLabel: unavailable ? "Unavailable" : "Failed",
                spokenState: spokenKinds[outcome.failure.kind],
                detailLines: nonEmpty([wrongPassword ? wrongPasswordDetail : outcome.failure.message, ...warnings]),
                openOutput: null,
                revealOutput: outcome.outputPaths[0] ?? null,
                pointer: pointerView(outcome.failure.pointer),
                // A retry reuses the run's output location, so it cannot fix a failure that points at that location.
                canRetry: isRetryable(outcome.failure) && outcome.failure.pointer?.kind !== "outputFolder",
                canRetryWithPassword: wrongPassword,
                packInstalled: false,
                fonts: [],
            };
        }
    }
};

export interface ResultSummary {
    headline: string;
    hints: string[];
}

const skippedAsCancelled = (result: JobOutcome) => result.state === "skipped" && result.reason.code === "cancelled";

const isCancelled = (result: JobOutcome) => skippedAsCancelled(result) || (result.state === "failed" && result.failure.kind === "cancelled");

export const resultTally = (results: JobOutcome[], resultVerb: string): string => {
    const count = (state: JobOutcome["state"]) => results.filter((result) => result.state === state).length;
    const cancelled = results.filter(skippedAsCancelled).length;
    const skipped = count("skipped") - cancelled;
    const needPack = results.filter((result) => neededPack(result) !== null).length;
    const failed = count("failed") - needPack;
    const parts = [`${count("succeeded")} ${resultVerb.toLowerCase()}`];
    if (skipped > 0) parts.push(`${skipped} skipped`);
    if (cancelled > 0) parts.push(`${cancelled} skipped as cancelled`);
    if (failed > 0) parts.push(`${failed} failed`);
    if (needPack > 0) parts.push(`${needPack} ${needPack === 1 ? "needs" : "need"} a pack`);
    return parts.join(", ");
};

export const resultSummary = (results: JobOutcome[], resultVerb: string, resultNoun?: string): ResultSummary => {
    const verb = resultVerb.toLowerCase();
    const flagged = results.filter((result) => result.state === "succeeded" && result.warnings.some((warning) => !isSavedAs(warning)));
    const warned = flagged.length;
    const noted = flagged.some((result) => result.warnings.some(isNote));
    const files = resultNoun ? (warned === 1 ? resultNoun : `${resultNoun}s`) : `${verb} ${warned === 1 ? "file" : "files"}`;
    const flag = noted ? `${warned === 1 ? "has" : "have"} a warning or note` : warned === 1 ? "has a warning" : "have warnings";
    const cancelled = results.some(isCancelled);
    const hints = [
        cancelled ? "Finished files were kept. Nothing was half-written." : null,
        warned === 0 ? null : `${warned} ${files} ${flag}.`,
    ];
    return {
        headline: `${cancelled ? "Cancelled" : "Done"}. ${resultTally(results, resultVerb)}`,
        hints: hints.filter((hint): hint is string => hint !== null),
    };
};

export interface BatchTotal {
    savedLine: string;
    totalLine: string;
    note: string;
}

/** While a batch runs, how much the finished copies saved, as in "14.3 MB saved so far", or null when no finished file reports sizes. */
export const savedSoFar = (finished: JobOutcome[]): string | null => {
    const measured = finished.filter((result) => result.state === "succeeded" && result.originalBytes !== undefined && result.outputBytes !== undefined);
    if (measured.length === 0) return null;
    const saved = measured.reduce((sum, result) => sum + (result.originalBytes ?? 0) - (result.outputBytes ?? 0), 0);
    return `${formatBytes(saved)} saved so far`;
};

/** Sums only files that got a smaller copy. Skipped and failed files kept their size and got no copy. */
export const batchTotal = (results: JobOutcome[], resultVerb: string): BatchTotal | null => {
    const measured = results.filter((result) => result.state === "succeeded" && result.originalBytes !== undefined && result.outputBytes !== undefined);
    if (measured.length === 0) return null;
    const before = measured.reduce((sum, result) => sum + (result.originalBytes ?? 0), 0);
    const after = measured.reduce((sum, result) => sum + (result.outputBytes ?? 0), 0);
    const verb = resultVerb.toLowerCase();
    return {
        savedLine: formatBatchSaving(before, after),
        totalLine: `${formatBytes(before)} → ${formatBytes(after)} across ${measured.length} ${verb} ${measured.length === 1 ? "file" : "files"}`,
        note: `Counts only ${verb} files. Skipped and failed files kept their size and got no copy.`,
    };
};
