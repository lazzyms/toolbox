import type { ErrorKind, JobOutcome, JpegColour, OutcomePointer, OutcomeWarning, ToolError } from "../contracts";
import { UtilityRegistry } from "../registry";

export type RowTone = "ok" | "warn" | "note" | "skip" | "fail";

export type RowPointer = { label: string; pointer: OutcomePointer };

export interface RowView {
    name: string;
    fullPath: string;
    tone: RowTone;
    stateLabel: string;
    spokenState: string;
    detailLines: string[];
    openOutput: { path: string; label: "Open" | "Open first" } | null;
    revealOutput: string | null;
    pointer: RowPointer | null;
    canRetry: boolean;
    canRetryWithPassword: boolean;
}

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

export const warningText = (warning: OutcomeWarning): string => {
    switch (warning.code) {
        case "signatureRemoved":
            return "Saved. The digital signature was removed, because changing the file breaks it.";
        case "fontSubstituted":
            return `The font ${warning.details.requestedFamily} wasn’t available, so ${warning.details.actualFamily} was used.`;
        case "renamedToAvoidReplacing":
            return `Saved as ${warning.details.finalName}. Renamed to avoid replacing an existing file.`;
        case "precisionReduced":
            return "Some values were rounded to the precision this format can store.";
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

/** Notes tell what another tool can still do for a file; they don't say anything went wrong. */
const isNote = (warning: OutcomeWarning) => warning.code === "pagesWithoutText";

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
    if (pointer.kind === "pack") return { label: `Install ${pointer.packId.replace(/-/g, " ")}`, pointer };
    if (pointer.kind === "outputFolder") return { label: "Choose output folder", pointer };
    const tool = UtilityRegistry.find((item) => item.id === pointer.toolId);
    return tool ? { label: `Open ${tool.title}`, pointer } : null;
};

const outputSummary = (paths: string[]) =>
    paths.length === 1
        ? fileName(paths[0])
        : `${paths.length} files, ${fileName(paths[0])} to ${fileName(paths[paths.length - 1])}`;

const nonEmpty = (lines: string[]) => lines.filter((line) => line.trim().length > 0);

export const rowView = (outcome: JobOutcome, resultVerb: string, namedByOutput = false): RowView => {
    const warnings = outcome.warnings.map(warningText);
    const input = { name: fileName(outcome.inputPath), fullPath: outcome.inputPath };
    switch (outcome.state) {
        case "succeeded": {
            const first = outcome.outputPaths[0] ?? null;
            const warned = warnings.length > 0;
            const noted = outcome.warnings.filter(isNote).length;
            const onlyNotes = warned && noted === warnings.length;
            const byOutput = namedByOutput && first !== null;
            return {
                ...(byOutput ? { name: fileName(first), fullPath: first } : input),
                tone: onlyNotes ? "note" : warned ? "warn" : "ok",
                stateLabel: resultVerb,
                spokenState: onlyNotes
                    ? `${resultVerb} with ${noted === 1 ? "a note" : "notes"}`
                    : warned ? `${resultVerb} with ${warnings.length === 1 ? "a warning" : "warnings"}` : resultVerb,
                detailLines: nonEmpty(warned ? [...warnings, outcome.detail] : [first && !byOutput ? outputSummary(outcome.outputPaths) : "", outcome.detail]),
                openOutput: first ? { path: first, label: outcome.outputPaths.length > 1 ? "Open first" : "Open" } : null,
                revealOutput: first,
                pointer: noted > 0 ? pointerView(ocrPointer) : null,
                canRetry: false,
                canRetryWithPassword: false,
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
            };
        case "failed": {
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
    const failed = count("failed");
    const parts = [`${count("succeeded")} ${resultVerb.toLowerCase()}`];
    if (skipped > 0) parts.push(`${skipped} skipped`);
    if (cancelled > 0) parts.push(`${cancelled} skipped as cancelled`);
    if (failed > 0) parts.push(`${failed} failed`);
    return parts.join(", ");
};

export const resultSummary = (results: JobOutcome[], resultVerb: string): ResultSummary => {
    const verb = resultVerb.toLowerCase();
    const warned = results.filter((result) => result.state === "succeeded" && result.warnings.some((warning) => !isNote(warning))).length;
    const cancelled = results.some(isCancelled);
    const hints = [
        cancelled ? "Finished files were kept. Nothing was half-written." : null,
        warned === 0 ? null : `${warned} ${verb} ${warned === 1 ? "file has a warning" : "files have warnings"}.`,
    ];
    return {
        headline: `${cancelled ? "Cancelled" : "Done"}. ${resultTally(results, resultVerb)}`,
        hints: hints.filter((hint): hint is string => hint !== null),
    };
};
