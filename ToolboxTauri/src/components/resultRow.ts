import type { ErrorKind, FontSource, JobOutcome, JpegColour, OutcomePointer, OutcomeWarning, PackNeed, ToolError } from "../contracts";
import { UtilityRegistry } from "../registry";

/** A note is a warning that needs nothing from the user, such as a photo saved at a lower bit depth. */
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
    /** Set when the file failed only for want of a pack that has since been installed. */
    packInstalled: boolean;
    /** Set when the file is in a format read in basic fidelity, so its output may differ from it more than usual. */
    basicFidelity: boolean;
    fonts: RowFont[];
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

const isRetryable = (failure: ToolError): boolean => {
    switch (failure.kind) {
        case "processing":
            return true;
        case "limitExceeded":
            // Size, page count and expansion belong to the file, so only a run that ran out of time can pass on another try.
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

/** A tool's `signatureRemovedNote` replaces the shared signature wording on its rows. */
export const warningText = (warning: OutcomeWarning, signatureRemovedNote?: string): string => {
    switch (warning.code) {
        case "signatureRemoved":
            return signatureRemovedNote ?? "Saved. The digital signature was removed, because changing the file breaks it.";
        case "fontSubstituted":
            return `The font ${warning.details.requestedFamily} wasn’t available, so ${warning.details.actualFamily} was used.`;
        case "renamedToAvoidReplacing":
            return `Saved as ${warning.details.finalName}. Renamed to avoid replacing an existing file.`;
        case "precisionReduced":
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
    }
};

export const packVerbs: Record<PackNeed, string> = { notInstalled: "Install", damaged: "Repair", incompatible: "Update" };

const warningDetail = (warning: OutcomeWarning, signatureRemovedNote?: string): RowDetail => ({
    text: warningText(warning, signatureRemovedNote),
    tone: warning.code === "omittedContent" ? "warn" : null,
});

/** Pages without text tell what OCR can still do for a file; the row points at OCR, and the summary doesn't count them. */
const pointsAtOcr = (warning: OutcomeWarning) => warning.code === "pagesWithoutText";

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

const isNote = (warning: OutcomeWarning) => warning.code === "precisionReduced" || warning.code === "approximateLayout" || pointsAtOcr(warning);

/** The pack a failed file needs before it can run, if that is why it failed. */
export const neededPack = (outcome: JobOutcome): PackPointer | null =>
    outcome.state === "failed" && outcome.failure.pointer?.kind === "pack" ? outcome.failure.pointer : null;

/**
 * A file that failed for want of a pack stays in the list without Retry, since running it again
 * without the pack fails the same way. Once that pack is installed, the row says the file is ready
 * to run again. `installedPacks` holds the packs installed, repaired or updated since the run.
 */
export const rowView = (outcome: JobOutcome, resultVerb: string, namedByOutput = false, signatureRemovedNote?: string, installedPacks: ReadonlySet<string> = new Set()): RowView => {
    const warnings = outcome.warnings.map((warning) => warningDetail(warning, signatureRemovedNote));
    const input = { name: fileName(outcome.inputPath), fullPath: outcome.inputPath };
    const basicFidelity = outcome.basicFidelity === true;
    switch (outcome.state) {
        case "succeeded": {
            const first = outcome.outputPaths[0] ?? null;
            const warned = warnings.length > 0;
            const noted = warned && outcome.warnings.every(isNote);
            // Precision notes read as one sentence ahead of the detail; layout and page notes keep their own line.
            const joined = noted && outcome.warnings.every((warning) => warning.code === "precisionReduced");
            const flagged = noted ? (warnings.length === 1 ? "a note" : "notes") : warnings.length === 1 ? "a warning" : "warnings";
            const byOutput = namedByOutput && first !== null;
            return {
                ...(byOutput ? { name: fileName(first), fullPath: first } : input),
                tone: noted ? "note" : warned ? "warn" : "ok",
                stateLabel: resultVerb,
                spokenState: warned ? `${resultVerb} with ${flagged}` : resultVerb,
                detailLines: nonEmpty(joined
                    ? [[...warnings.map((warning) => warning.text), outcome.detail].filter((line) => line.trim()).join(" ")]
                    : warned ? [...warnings, outcome.detail] : [first && !byOutput ? outputSummary(outcome.outputPaths) : "", outcome.detail]),
                openOutput: first ? { path: first, label: outcome.outputPaths.length > 1 ? "Open first" : "Open" } : null,
                revealOutput: first,
                pointer: outcome.warnings.some(pointsAtOcr) ? pointerView(ocrPointer) : null,
                canRetry: false,
                canRetryWithPassword: false,
                packInstalled: false,
                basicFidelity,
                fonts: (outcome.fonts ?? []).map((font) => ({ requested: font.requestedFamily, used: font.fontUsed, source: fontSources[font.source] })),
            };
        }
        case "skipped":
            return {
                ...input,
                tone: "skip",
                stateLabel: "Skipped",
                spokenState: "Skipped",
                detailLines: nonEmpty([outcome.reason.message, ...warnings]),
                openOutput: null,
                revealOutput: null,
                pointer: pointerView(outcome.reason.pointer),
                canRetry: false,
                canRetryWithPassword: false,
                packInstalled: false,
                basicFidelity,
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
                    detailLines: installed ? nonEmpty([`The ${pack.name} pack is installed now. Run again to read this file.`]) : nonEmpty([outcome.failure.message, ...warnings]),
                    openOutput: null,
                    revealOutput: null,
                    pointer: installed ? null : pointerView(pack),
                    canRetry: false,
                    canRetryWithPassword: false,
                    packInstalled: installed,
                    basicFidelity,
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
                basicFidelity,
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
    const flagged = results.filter((result) => result.state === "succeeded" && result.warnings.some((warning) => !pointsAtOcr(warning)));
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
