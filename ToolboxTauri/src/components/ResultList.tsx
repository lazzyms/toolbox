import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { JobOutcome, Progress } from "../contracts";

type OutputAction = "open" | "reveal";

type OutputActionState = {
    pending: OutputAction | null;
    error: string | null;
};

const idleActionState = (): OutputActionState => ({ pending: null, error: null });

const outputId = (resultIndex: number, outputIndex: number) => `${resultIndex}:${outputIndex}`;

const actionStatesFor = (results: JobOutcome[]): Record<string, OutputActionState> => {
    const states: Record<string, OutputActionState> = {};
    results.forEach((result, resultIndex) => {
        result.outputPaths.forEach((_, outputIndex) => {
            states[outputId(resultIndex, outputIndex)] = idleActionState();
        });
    });
    return states;
};

const outputName = (path: string) => path.split(/[\\/]/).pop() || path;

const revealButtonLabel = () => {
    const platform = navigator.platform.toLowerCase();
    if (platform.includes("mac")) return "Show in Finder";
    if (platform.includes("win")) return "Open file location";
    return "Show in file manager";
};

export const ResultList = ({ results, progress, loading }: { results: JobOutcome[]; progress: Progress; loading: boolean }) => {
    const [actionStates, setActionStates] = useState<Record<string, OutputActionState>>(() => actionStatesFor(results));
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates(actionStatesFor(results));
        return () => {
            if (resultSetGeneration.current === generation) resultSetGeneration.current += 1;
        };
    }, [results]);

    const runAction = async (id: string, path: string, action: OutputAction) => {
        const generation = resultSetGeneration.current;
        setActionStates((states) => ({
            ...states,
            [id]: { pending: action, error: null },
        }));

        let error: string | null = null;
        try {
            await invoke(action === "open" ? "open_output_path" : "reveal_output_path", { path });
        } catch (reason) {
            error = reason instanceof Error ? reason.message : String(reason);
        }

        if (resultSetGeneration.current !== generation) return;
        setActionStates((states) => ({
            ...states,
            [id]: { pending: null, error },
        }));
    };

    if (loading) {
        return <p className="muted-copy">Processing {progress.completed} of {progress.total} files...</p>;
    }
    if (results.length === 0) return null;

    const failures = results.filter((result) => result.failure !== null).length;
    return (
        <div className="result-list" aria-live="polite">
            <p className="result-summary" role="status">
                {failures === 0 ? `${results.length} files completed` : `${failures} of ${results.length} files failed`}
            </p>
            {results.map((result, resultIndex) => (
                <Card role="article" key={result.inputPath} className="result-card py-0">
                    <CardHeader className="result-heading py-3">
                        <CardTitle className="result-input" title={result.inputPath}>{outputName(result.inputPath)}</CardTitle>
                        <Badge variant={result.failure ? "destructive" : "secondary"}>
                            {result.failure?.message ?? result.detail}
                        </Badge>
                    </CardHeader>
                    {result.outputPaths.length > 0 && (
                        <CardContent className="result-outputs-content py-3">
                        <ul className="result-outputs">
                            {result.outputPaths.map((path, index) => {
                                const id = outputId(resultIndex, index);
                                const state = actionStates[id] ?? idleActionState();
                                return (
                                    <li key={id} className="result-output">
                                        <span className="result-output-name" title={path}>{outputName(path)}</span>
                                        <div className="result-output-actions">
                                            <Button
                                                variant="ghost"
                                                size="sm"
                                                disabled={state.pending !== null}
                                                onClick={() => void runAction(id, path, "open")}
                                            >
                                                {state.pending === "open" ? "Opening..." : "Open file"}
                                            </Button>
                                            <Button
                                                variant="ghost"
                                                size="sm"
                                                disabled={state.pending !== null}
                                                onClick={() => void runAction(id, path, "reveal")}
                                            >
                                                {state.pending === "reveal" ? "Showing..." : revealButtonLabel()}
                                            </Button>
                                        </div>
                                        {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
                                    </li>
                                );
                            })}
                        </ul>
                        </CardContent>
                    )}
                </Card>
            ))}
        </div>
    );
};
