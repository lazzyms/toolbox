import type { ErrorKind, JobOutcome, OutcomePointer, OutcomeWarning, PackNeed, ToolError } from "../contracts";
import { UtilityRegistry } from "../registry";

export type RowTone = "ok" | "warn" | "skip" | "fail";

export type RowPointer = { label: string; pointer: OutcomePointer };

type PackPointer = Extract<OutcomePointer, { kind: "pack" }>;

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
    /** Set when the file failed only for want of a pack that has since been installed. */
    packInstalled: boolean;
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
    }
};

export const packVerbs: Record<PackNeed, string> = { notInstalled: "Install", damaged: "Repair", incompatible: "Update" };

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

const nonEmpty = (lines: string[]) => lines.filter((line) => line.trim().length > 0);

/** The pack a failed file needs before it can run, if that is why it failed. */
export const neededPack = (outcome: JobOutcome): PackPointer | null =>
    outcome.state === "failed" && outcome.failure.pointer?.kind === "pack" ? outcome.failure.pointer : null;

/**
 * A file that failed for want of a pack stays in the list without Retry, since running it again
 * without the pack fails the same way. Once that pack is installed, the row says the file is ready
 * to run again. `installedPacks` holds the packs installed, repaired or updated since the run.
 */
export const rowView = (outcome: JobOutcome, resultVerb: string, namedByOutput = false, installedPacks: ReadonlySet<string> = new Set()): RowView => {
    const warnings = outcome.warnings.map(warningText);
    const input = { name: fileName(outcome.inputPath), fullPath: outcome.inputPath };
    switch (outcome.state) {
        case "succeeded": {
            const first = outcome.outputPaths[0] ?? null;
            const warned = warnings.length > 0;
            const byOutput = namedByOutput && first !== null;
            return {
                ...(byOutput ? { name: fileName(first), fullPath: first } : input),
                tone: warned ? "warn" : "ok",
                stateLabel: resultVerb,
                spokenState: warned ? `${resultVerb} with ${warnings.length === 1 ? "a warning" : "warnings"}` : resultVerb,
                detailLines: nonEmpty(warned ? [...warnings, outcome.detail] : [first && !byOutput ? outputSummary(outcome.outputPaths) : "", outcome.detail]),
                openOutput: first ? { path: first, label: outcome.outputPaths.length > 1 ? "Open first" : "Open" } : null,
                revealOutput: first,
                pointer: null,
                canRetry: false,
                canRetryWithPassword: false,
                packInstalled: false,
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
                    detailLines: installed ? [`The ${pack.name} pack is installed now. Run again to read this file.`] : nonEmpty([outcome.failure.message, ...warnings]),
                    openOutput: null,
                    revealOutput: null,
                    pointer: installed ? null : pointerView(pack),
                    canRetry: false,
                    canRetryWithPassword: false,
                    packInstalled: installed,
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

export const resultSummary = (results: JobOutcome[], resultVerb: string): ResultSummary => {
    const verb = resultVerb.toLowerCase();
    const warned = results.filter((result) => result.state === "succeeded" && result.warnings.length > 0).length;
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
