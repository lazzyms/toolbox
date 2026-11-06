import { invoke } from "@tauri-apps/api/core";
import { Check, CircleMinus, CircleX, Clock, Info, KeyRound, LoaderCircle, RotateCcw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Progress as ProgressBar } from "@/components/ui/progress";
import type { JobOutcome, OutcomePointer, Progress } from "../contracts";
import { currentPage, liveFiles, pageTally, type LiveFile, type LiveRun } from "./liveRun";
import { fileName, resultSummary, resultTally, rowView, type RowTone } from "./resultRow";
import { PasswordRetryForm } from "./PasswordFields";

type RowAction = "open" | "reveal" | "retry";

type RowActionState = {
    pending: RowAction | null;
    error: string | null;
};

const idleActionState: RowActionState = { pending: null, error: null };

const toneIcons: Record<RowTone, typeof Check> = {
    ok: Check,
    warn: TriangleAlert,
    note: Info,
    skip: CircleMinus,
    fail: CircleX,
};

const PROPORTIONAL_BAR_ABOVE = 60;

type SegmentTone = "ok" | "skip" | "fail" | "wait";

const segmentTone = (file: LiveFile): SegmentTone => {
    if (file.status !== "finished") return "wait";
    if (file.outcome.state === "succeeded") return "ok";
    return file.outcome.state === "skipped" ? "skip" : "fail";
};

const revealButtonLabel = () => {
    const platform = navigator.platform.toLowerCase();
    if (platform.includes("mac")) return "Show in Finder";
    if (platform.includes("win")) return "Show in File Explorer";
    return "Show in file manager";
};

const followPointer = (pointer: OutcomePointer, inputPath: string) => {
    if (pointer.kind === "tool") {
        window.dispatchEvent(new CustomEvent("toolbox:open", { detail: { toolId: pointer.toolId, paths: [inputPath] } }));
    } else if (pointer.kind === "outputFolder") {
        window.dispatchEvent(new CustomEvent("toolbox:open-settings", { detail: "output-location" }));
    } else {
        window.dispatchEvent(new CustomEvent("toolbox:install-pack", { detail: pointer.packId }));
    }
};

interface ResultListProps {
    results: JobOutcome[];
    progress: Progress;
    loading: boolean;
    resultVerb: string;
    runningVerb: string;
    runningDetail?: string;
    /** Count the live run in pages, for tools whose single files can take minutes. */
    progressByPage?: boolean;
    signatureRemovedNote?: string;
    live: LiveRun | null;
    namedByOutput?: boolean;
    onRetry?: (inputPath: string) => Promise<void>;
    onRetryWithPassword?: (inputPath: string, password: string) => Promise<void>;
}

export const ResultList = ({ results, progress, loading, resultVerb, runningVerb, runningDetail, progressByPage = false, signatureRemovedNote, live, namedByOutput = false, onRetry, onRetryWithPassword }: ResultListProps) => {
    const [actionStates, setActionStates] = useState<Record<number, RowActionState>>({});
    const [passwordRow, setPasswordRow] = useState<number | null>(null);
    // Page details start open, the way the board shows them; a row can fold its own away.
    const [foldedDetails, setFoldedDetails] = useState<Record<number, boolean>>({});
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates({});
        setPasswordRow(null);
        setFoldedDetails({});
        return () => {
            if (resultSetGeneration.current === generation) resultSetGeneration.current += 1;
        };
    }, [results]);

    const runAction = async (row: number, action: RowAction, perform: () => Promise<unknown>) => {
        const generation = resultSetGeneration.current;
        setActionStates((states) => ({ ...states, [row]: { pending: action, error: null } }));

        let error: string | null = null;
        try {
            await perform();
        } catch (reason) {
            error = reason instanceof Error ? reason.message : String(reason);
        }

        if (resultSetGeneration.current !== generation) return;
        setActionStates((states) => ({ ...states, [row]: { pending: null, error } }));
    };

    const reveal = revealButtonLabel();

    const finishedRow = (result: JobOutcome, index: number) => {
        const view = rowView(result, resultVerb, namedByOutput, signatureRemovedNote);
        const state = actionStates[index] ?? idleActionState;
        const busy = state.pending !== null;
        const name = view.name;
        const Icon = toneIcons[view.tone];
        const pageDetails = result.pageDetails ?? [];
        const detailsOpen = !foldedDetails[index];
        const detailsId = `page-details-${index}`;
        return (
            <li key={`${index}:${result.inputPath}`} className="result-row" data-tone={view.tone} data-state={result.state}>
                <span className="result-row-icon" aria-hidden="true"><Icon /></span>
                <span className="result-row-text">
                    <span className="result-row-name" title={view.fullPath}>{name}</span>
                    {view.detailLines.map((line, lineIndex) => (
                        <span key={lineIndex} className="result-row-detail">{line}</span>
                    ))}
                </span>
                <span className="result-row-state">
                    <span aria-hidden="true">{view.stateLabel}</span>
                    <span className="sr-only">{view.spokenState}</span>
                </span>
                <span className="result-row-actions">
                    {pageDetails.length > 0 && (
                        <Button
                            variant="ghost"
                            size="xs"
                            aria-expanded={detailsOpen}
                            aria-controls={detailsId}
                            onClick={() => setFoldedDetails((folded) => ({ ...folded, [index]: detailsOpen }))}
                        >
                            {detailsOpen ? "Hide page details" : "Show page details"}
                        </Button>
                    )}
                    {view.openOutput && (
                        <Button
                            variant="ghost"
                            size="xs"
                            aria-label={`Open ${fileName(view.openOutput.path)}`}
                            disabled={busy}
                            onClick={() => void runAction(index, "open", () => invoke("open_output_path", { path: view.openOutput?.path }))}
                        >
                            {state.pending === "open" ? "Opening..." : view.openOutput.label}
                        </Button>
                    )}
                    {view.revealOutput && (
                        <Button
                            variant="ghost"
                            size="xs"
                            aria-label={`${reveal}: ${fileName(view.revealOutput)}`}
                            disabled={busy}
                            onClick={() => void runAction(index, "reveal", () => invoke("reveal_output_path", { path: view.revealOutput }))}
                        >
                            {state.pending === "reveal" ? "Showing..." : reveal}
                        </Button>
                    )}
                    {view.pointer && (
                        <Button variant="outline" size="xs" onClick={() => view.pointer && followPointer(view.pointer.pointer, result.inputPath)}>
                            {view.pointer.label}
                        </Button>
                    )}
                    {view.canRetry && onRetry && !live && (
                        <Button
                            variant="outline"
                            size="xs"
                            aria-label={`Retry ${name}`}
                            disabled={busy}
                            onClick={() => void runAction(index, "retry", () => onRetry(result.inputPath))}
                        >
                            <RotateCcw />
                            {state.pending === "retry" ? "Retrying..." : "Retry"}
                        </Button>
                    )}
                    {view.canRetryWithPassword && onRetryWithPassword && !live && (
                        <Button variant="outline" size="xs" aria-expanded={passwordRow === index} disabled={busy} onClick={() => setPasswordRow(passwordRow === index ? null : index)}>
                            <KeyRound />
                            Try another password
                        </Button>
                    )}
                </span>
                {pageDetails.length > 0 && detailsOpen && (
                    <ul id={detailsId} className="result-page-details" aria-label={`Page details for ${name}`}>
                        {pageDetails.map((detail) => (
                            <li key={detail.label}>
                                <span className="result-page-details-label">{detail.label}</span>
                                <span>{detail.value}</span>
                            </li>
                        ))}
                    </ul>
                )}
                {view.canRetryWithPassword && onRetryWithPassword && !live && passwordRow === index && (
                    <PasswordRetryForm
                        name={name}
                        busy={busy}
                        onRetry={(password) => void runAction(index, "retry", () => onRetryWithPassword(result.inputPath, password))}
                        onCancel={() => setPasswordRow(null)}
                    />
                )}
                {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
            </li>
        );
    };

    const liveRow = (file: Exclude<LiveFile, { status: "finished" }>, index: number) => {
        const running = file.status === "running";
        const Icon = running ? LoaderCircle : Clock;
        const label = running ? runningVerb : "Waiting";
        const name = fileName(file.inputPath);
        const pages = file.pages;
        const detail = running
            ? pages && progressByPage ? `Page ${currentPage(pages)} of ${pages.total}` : runningDetail
            : progressByPage ? "Page count unknown" : undefined;
        return (
            <li key={`${index}:${file.inputPath}`} className="result-row" data-tone={file.status} data-state={file.status}>
                <span className="result-row-icon" aria-hidden="true"><Icon /></span>
                <span className="result-row-text">
                    <span className="result-row-name" title={file.inputPath}>{name}</span>
                    {detail && <span className="result-row-detail">{detail}</span>}
                    {running && progressByPage && pages && pages.total > 0 && (
                        <ProgressBar
                            className="live-run-minibar mt-1 h-1"
                            aria-label={name}
                            value={pages.done}
                            max={pages.total}
                            getValueLabel={(value, max) => `Page ${currentPage({ done: value, total: max })} of ${max}, ${value} done`}
                        />
                    )}
                </span>
                <span className="result-row-state">{label}</span>
                <span className="result-row-actions" />
            </li>
        );
    };

    if (loading && live && progressByPage) {
        const files = liveFiles(live);
        const finished = files.filter((file) => file.status === "finished").length;
        const tally = pageTally(live);
        const reading = files.flatMap((file) => file.status === "running" && file.pages ? [{ name: fileName(file.inputPath), pages: file.pages }] : [])[0];
        const pagesDone = `${tally.done} of ${tally.total} pages done`;
        return (
            <div className="result-list" data-live="true">
                <div className="live-run-header">
                    <div className="result-summary" role="status" aria-live="polite">
                        <strong>{runningVerb} file {Math.min(finished + 1, files.length)} of {files.length}</strong>
                        {reading && <span className="result-summary-hint">{reading.name}, page {currentPage(reading.pages)} of {reading.pages.total}</span>}
                    </div>
                    <ProgressBar
                        className="live-run-pages"
                        aria-label="Pages recognized"
                        value={tally.done}
                        max={Math.max(tally.total, 1)}
                        getValueLabel={() => pagesDone}
                    />
                    <p className="result-summary-hint live-run-note">{pagesDone}. Long scans take a while; the bar counts pages, not files.</p>
                    <p className="result-summary-hint live-run-note">Each file is recognized on its own, so one that fails doesn't stop the others.</p>
                </div>
                <Card className="result-card gap-0 py-0">
                    <CardContent className="p-0">
                        <ul className="result-rows" aria-label="Files and results">
                            {files.map((file, index) => file.status === "finished" ? finishedRow(file.outcome, index) : liveRow(file, index))}
                        </ul>
                    </CardContent>
                </Card>
            </div>
        );
    }
    if (loading && live) {
        const files = liveFiles(live);
        const done = files.flatMap((file) => file.status === "finished" ? [file.outcome] : []);
        const total = files.length;
        const proportional = total > PROPORTIONAL_BAR_ABOVE;
        const soFar = done.length > 0 ? `${resultTally(done, resultVerb)} so far.` : null;
        const segments = files.map(segmentTone);
        return (
            <div className="result-list" data-live="true">
                <div className="live-run-header">
                    <div className="result-summary" role="status" aria-live="polite">
                        <strong>{runningVerb} {done.length} of {total}</strong>
                        {soFar && <span className="result-summary-hint">{proportional ? soFar : `${soFar} Finished files can be opened now.`}</span>}
                    </div>
                    {proportional ? (
                        <div className="live-run-bar" data-mode="proportional" aria-hidden="true">
                            {(["ok", "skip", "fail", "wait"] as const).map((tone) => (
                                <span key={tone} data-tone={tone} style={{ flexGrow: segments.filter((segment) => segment === tone).length }} />
                            ))}
                        </div>
                    ) : (
                        <div className="live-run-bar" data-mode="segments" aria-hidden="true">
                            {segments.map((tone, index) => <span key={index} data-tone={tone} />)}
                        </div>
                    )}
                    {proportional && <p className="result-summary-hint live-run-note">Over {PROPORTIONAL_BAR_ABOVE} files, the bar shows proportions. Rows for finished files can be opened now.</p>}
                </div>
                <Card className="result-card gap-0 py-0">
                    <CardContent className="p-0">
                        <ul className="result-rows" aria-label="Files and results">
                            {files.map((file, index) => file.status === "finished" ? finishedRow(file.outcome, index) : liveRow(file, index))}
                        </ul>
                    </CardContent>
                </Card>
            </div>
        );
    }
    if (loading) {
        return <p className="muted-copy">Processing {progress.completed} of {progress.total} files...</p>;
    }
    if (results.length === 0) return null;

    const summary = resultSummary(results, resultVerb);
    return (
        <div className="result-list">
            <div className="result-summary" role="status" aria-live="polite">
                <strong>{summary.headline}</strong>
                {summary.hints.map((hint) => <span key={hint} className="result-summary-hint">{hint}</span>)}
            </div>
            <Card className="result-card gap-0 py-0">
                <CardContent className="p-0">
                    <ul className="result-rows" aria-label="Results">
                        {results.map(finishedRow)}
                    </ul>
                </CardContent>
            </Card>
        </div>
    );
};
