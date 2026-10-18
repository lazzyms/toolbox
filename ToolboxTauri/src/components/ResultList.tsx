import { invoke } from "@tauri-apps/api/core";
import { Check, CircleMinus, CircleX, RotateCcw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome, OutcomePointer, Progress } from "../contracts";
import { fileName, resultSummary, rowView, type RowTone } from "./resultRow";

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

const revealButtonLabel = () => {
    const platform = navigator.platform.toLowerCase();
    if (platform.includes("mac")) return "Show in Finder";
    if (platform.includes("win")) return "Show in File Explorer";
    return "Show in file manager";
};

const followPointer = (pointer: OutcomePointer) => {
    if (pointer.kind === "tool") {
        window.dispatchEvent(new CustomEvent("toolbox:open", { detail: pointer.toolId }));
    } else {
        window.dispatchEvent(new CustomEvent("toolbox:install-pack", { detail: pointer.packId }));
    }
};

interface ResultListProps {
    results: JobOutcome[];
    progress: Progress;
    loading: boolean;
    resultVerb: string;
    onRetry?: (inputPath: string) => Promise<void>;
}

export const ResultList = ({ results, progress, loading, resultVerb, onRetry }: ResultListProps) => {
    const [actionStates, setActionStates] = useState<Record<number, RowActionState>>({});
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates({});
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

    if (loading) {
        return <p className="muted-copy">Processing {progress.completed} of {progress.total} files...</p>;
    }
    if (results.length === 0) return null;

    const summary = resultSummary(results, resultVerb);
    const reveal = revealButtonLabel();
    return (
        <div className="result-list">
            <div className="result-summary" role="status" aria-live="polite">
                <strong>{summary.headline}</strong>
                {summary.warningHint && <span className="result-summary-hint">{summary.warningHint}</span>}
            </div>
            <Card className="result-card gap-0 py-0">
                <CardContent className="p-0">
                    <ul className="result-rows" aria-label="Results">
                        {results.map((result, index) => {
                            const view = rowView(result, resultVerb);
                            const state = actionStates[index] ?? idleActionState;
                            const busy = state.pending !== null;
                            const name = fileName(result.inputPath);
                            const Icon = toneIcons[view.tone];
                            return (
                                <li key={`${index}:${result.inputPath}`} className="result-row" data-tone={view.tone} data-state={result.state}>
                                    <span className="result-row-icon" aria-hidden="true"><Icon /></span>
                                    <span className="result-row-text">
                                        <span className="result-row-name" title={result.inputPath}>{name}</span>
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
                                        {view.pointer && (
                                            <Button variant="outline" size="xs" onClick={() => view.pointer && followPointer(view.pointer.pointer)}>
                                                {view.pointer.label}
                                            </Button>
                                        )}
                                        {view.canRetry && onRetry && (
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
                                    </span>
                                    {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
                                </li>
                            );
                        })}
                    </ul>
                </CardContent>
            </Card>
        </div>
    );
};
