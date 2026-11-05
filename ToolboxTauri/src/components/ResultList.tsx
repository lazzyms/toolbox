import { invoke } from "@tauri-apps/api/core";
import { Check, CircleMinus, CircleX, Clock, KeyRound, LoaderCircle, RotateCcw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome, OutcomePointer, Progress } from "../contracts";
import { liveFiles, type LiveFile, type LiveRun } from "./liveRun";
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

export const followPointer = (pointer: OutcomePointer) => {
    if (pointer.kind === "tool") {
        window.dispatchEvent(new CustomEvent("toolbox:open", { detail: pointer.toolId }));
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
    live: LiveRun | null;
    namedByOutput?: boolean;
    /** Lists each output of a file that wrote several, with its own Open and Reveal, behind a toggle on the row. */
    listOutputs?: boolean;
    onRetry?: (inputPath: string) => Promise<void>;
    onRetryWithPassword?: (inputPath: string, password: string) => Promise<void>;
}

export const ResultList = ({ results, progress, loading, resultVerb, runningVerb, runningDetail, live, namedByOutput = false, listOutputs = false, onRetry, onRetryWithPassword }: ResultListProps) => {
    const [actionStates, setActionStates] = useState<Record<string, RowActionState>>({});
    const [passwordRow, setPasswordRow] = useState<number | null>(null);
    const [expandedRows, setExpandedRows] = useState<ReadonlySet<number>>(new Set());
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates({});
        setPasswordRow(null);
        setExpandedRows(new Set());
        return () => {
            if (resultSetGeneration.current === generation) resultSetGeneration.current += 1;
        };
    }, [results]);

    const runAction = async (row: number | string, action: RowAction, perform: () => Promise<unknown>) => {
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
        const view = rowView(result, resultVerb, namedByOutput, listOutputs);
        const state = actionStates[index] ?? idleActionState;
        const busy = state.pending !== null;
        const listed = view.listedOutputs;
        const expanded = expandedRows.has(index);
        const outputsId = `result-outputs-${index}`;
        const toggleExpanded = () => setExpandedRows((rows) => {
            const next = new Set(rows);
            if (!next.delete(index)) next.add(index);
            return next;
        });
        const name = view.name;
        const Icon = toneIcons[view.tone];
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
                    {listed.length > 0 && (
                        <Button variant="outline" size="xs" aria-expanded={expanded} aria-controls={outputsId} onClick={toggleExpanded}>
                            {expanded ? "Hide files" : `Show ${listed.length === 1 ? "1 file" : `${listed.length} files`}`}
                        </Button>
                    )}
                    {view.pointer && (
                        <Button variant="outline" size="xs" onClick={() => view.pointer && followPointer(view.pointer.pointer)}>
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
                {view.canRetryWithPassword && onRetryWithPassword && !live && passwordRow === index && (
                    <PasswordRetryForm
                        name={name}
                        busy={busy}
                        onRetry={(password) => void runAction(index, "retry", () => onRetryWithPassword(result.inputPath, password))}
                        onCancel={() => setPasswordRow(null)}
                    />
                )}
                {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
                {listed.length > 0 && expanded && (
                    <ul id={outputsId} className="result-outputs" aria-label={`Files from ${name}`}>
                        {listed.map((path, outputIndex) => {
                            const key = `${index}:${outputIndex}`;
                            const outputState = actionStates[key] ?? idleActionState;
                            const outputBusy = outputState.pending !== null;
                            const outputName = fileName(path);
                            return (
                                <li key={path} className="result-output">
                                    <span className="result-output-name" title={path}>{outputName}</span>
                                    <Button
                                        variant="ghost"
                                        size="xs"
                                        aria-label={`Open ${outputName}`}
                                        disabled={outputBusy}
                                        onClick={() => void runAction(key, "open", () => invoke("open_output_path", { path }))}
                                    >
                                        {outputState.pending === "open" ? "Opening..." : "Open"}
                                    </Button>
                                    <Button
                                        variant="ghost"
                                        size="xs"
                                        aria-label={`${reveal}: ${outputName}`}
                                        disabled={outputBusy}
                                        onClick={() => void runAction(key, "reveal", () => invoke("reveal_output_path", { path }))}
                                    >
                                        {outputState.pending === "reveal" ? "Showing..." : reveal}
                                    </Button>
                                    {outputState.error && <p className="result-action-error" role="alert">{outputState.error}</p>}
                                </li>
                            );
                        })}
                    </ul>
                )}
            </li>
        );
    };

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
                            {files.map((file, index) => {
                                if (file.status === "finished") return finishedRow(file.outcome, index);
                                const running = file.status === "running";
                                const Icon = running ? LoaderCircle : Clock;
                                const label = running ? runningVerb : "Waiting";
                                return (
                                    <li key={`${index}:${file.inputPath}`} className="result-row" data-tone={file.status} data-state={file.status}>
                                        <span className="result-row-icon" aria-hidden="true"><Icon /></span>
                                        <span className="result-row-text">
                                            <span className="result-row-name" title={file.inputPath}>{fileName(file.inputPath)}</span>
                                            {running && runningDetail && <span className="result-row-detail">{runningDetail}</span>}
                                        </span>
                                        <span className="result-row-state">{label}</span>
                                        <span className="result-row-actions" />
                                    </li>
                                );
                            })}
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
