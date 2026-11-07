import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronDown, ChevronRight, CircleMinus, CircleX, Clock, FileText, Info, KeyRound, Layers, LoaderCircle, RotateCcw, TriangleAlert } from "lucide-react";
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome, OutcomePointer, Progress } from "../contracts";
import { shortenHomePath, useHomeDir } from "../homePath";
import type { CombinedResult } from "../registry/saveSteps";
import { liveFiles, type LiveFile, type LiveRun } from "./liveRun";
import { batchTotal, fileName, resultSummary, resultTally, rowView, warningDetail, warningText, type OutputGroup, type OutputGrouping, type RowTone } from "./resultRow";
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

export const revealButtonLabels = () => {
    const platform = navigator.platform.toLowerCase();
    if (platform.includes("mac")) return { full: "Show in Finder", short: "Show in Finder" };
    if (platform.includes("win")) return { full: "Show in File Explorer", short: "Show in Explorer" };
    return { full: "Show in file manager", short: "Show in file manager" };
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
    resultNoun?: string;
    runningVerb: string;
    runningDetail?: string;
    live: LiveRun | null;
    combinedInputs?: string[] | null;
    combined?: CombinedResult;
    outputNoun?: string;
    onRetry?: (inputPath: string) => Promise<void>;
    onRetryWithPassword?: (inputPath: string, password: string) => Promise<void>;
    /** Shows a succeeded file's outputs as one row that expands into its parts. */
    outputGroup?: OutputGrouping;
    /** A line after the finished headline, such as a total across files. */
    resultsHint?: (results: JobOutcome[]) => string | null;
    /** Names each row by its output file, as a combined result's row is. */
    namedByOutput?: boolean;
    /** Replaces the finished summary line. */
    summary?: ReactNode;
}

export const ResultList = ({ results, progress, loading, resultVerb, resultNoun, runningVerb, runningDetail, live, combinedInputs = null, combined, outputNoun, onRetry, onRetryWithPassword, outputGroup, resultsHint, namedByOutput: namedByOutputProp = false, summary: customSummary }: ResultListProps) => {
    const namedByOutput = namedByOutputProp || combinedInputs !== null;
    const home = useHomeDir();
    const [actionStates, setActionStates] = useState<Record<string, RowActionState>>({});
    const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
    const [passwordRow, setPasswordRow] = useState<number | null>(null);
    const [fontRows, setFontRows] = useState<ReadonlySet<number>>(new Set());
    const [expandedRows, setExpandedRows] = useState<ReadonlySet<number>>(new Set());
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates({});
        setExpanded(new Set());
        setPasswordRow(null);
        setFontRows(new Set());
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

    const { full: reveal, short: revealShort } = revealButtonLabels();

    const toggleOutputs = (row: number) => setExpandedRows((rows) => {
        const next = new Set(rows);
        if (!next.delete(row)) next.add(row);
        return next;
    });

    const toggleGroup = (index: number) => setExpanded((open) => {
        const next = new Set(open);
        if (!next.delete(index)) next.add(index);
        return next;
    });

    const partRow = (group: OutputGroup, index: number) => group.parts.map((part, partIndex) => {
        const key = `${index}:${partIndex}`;
        const state = actionStates[key] ?? idleActionState;
        const busy = state.pending !== null;
        const PartIcon = part.kind === "set" ? Layers : FileText;
        return (
            <li key={key} className="result-row result-row-part" data-tone="part">
                <span aria-hidden="true" />
                <span className="result-row-icon" aria-hidden="true"><PartIcon /></span>
                <span className="result-row-text">
                    <span className="result-row-name">{part.name}</span>
                    <span className="result-row-detail">{part.hint}</span>
                </span>
                <span className="result-row-state" />
                <span className="result-row-actions">
                    {part.open && (
                        <Button variant="ghost" size="xs" aria-label={`Open ${part.name}`} disabled={busy} onClick={() => void runAction(key, "open", () => invoke("open_output_path", { path: part.open }))}>
                            {state.pending === "open" ? "Opening..." : "Open"}
                        </Button>
                    )}
                    <Button variant="ghost" size="xs" aria-label={`${reveal}: ${part.name}`} disabled={busy} onClick={() => void runAction(key, "reveal", () => invoke("reveal_output_path", { path: part.reveal }))}>
                        {state.pending === "reveal" ? "Showing..." : reveal}
                    </Button>
                </span>
                {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
            </li>
        );
    });

    const toggleFonts = (row: number) =>
        setFontRows((rows) => {
            const next = new Set(rows);
            if (!next.delete(row)) next.add(row);
            return next;
        });

    const finishedRow = (result: JobOutcome, index: number) => {
        const group = result.state === "succeeded" ? outputGroup?.(result) ?? null : null;
        const baseView = rowView(result, resultVerb, namedByOutput, outputNoun);
        const view = group
            ? { ...baseView, detailLines: [{ text: group.hint, tone: null }, ...result.warnings.map(warningDetail)], openOutput: { path: group.primary, label: "Open" as const }, revealOutput: group.primary }
            : baseView;
        const open = group !== null && expanded.has(index);
        const state = actionStates[index] ?? idleActionState;
        const busy = state.pending !== null;
        const name = view.name;
        const Icon = toneIcons[view.tone];
        const listedOutputs = outputNoun && result.state === "succeeded" && result.outputPaths.length > 1 ? result.outputPaths : null;
        const outputsListed = listedOutputs !== null && expandedRows.has(index);
        const outputListId = `result-outputs-${index}`;
        const fontsOpen = fontRows.has(index);
        const fontListId = `result-fonts-${index}`;
        return (
            <Fragment key={`${index}:${result.inputPath}`}>
            <li className="result-row" data-tone={view.tone} data-state={result.state}>
                {outputGroup && (group ? (
                    <Button variant="ghost" size="icon-xs" className="result-row-toggle" aria-expanded={open} aria-label={`${open ? "Hide" : "Show"} outputs of ${name}`} onClick={() => toggleGroup(index)}>
                        {open ? <ChevronDown /> : <ChevronRight />}
                    </Button>
                ) : <span aria-hidden="true" />)}
                <span className="result-row-icon" aria-hidden="true"><Icon /></span>
                <span className="result-row-text">
                    <span className="result-row-name" title={view.fullPath}>{name}</span>
                    {view.detailLines.map((line, lineIndex) => (
                        <span key={lineIndex} className="result-row-detail" data-tone={line.tone ?? undefined}>{line.text}</span>
                    ))}
                </span>
                <span className="result-row-state">
                    <span aria-hidden="true">{view.stateLabel}</span>
                    <span className="sr-only">{view.spokenState}</span>
                </span>
                <span className="result-row-actions">
                    {view.fonts.length > 0 && (
                        <Button variant="ghost" size="xs" aria-expanded={fontsOpen} aria-controls={fontListId} onClick={() => toggleFonts(index)}>
                            {fontsOpen ? "Hide fonts" : "Show fonts"}
                        </Button>
                    )}
                    {listedOutputs && (
                        <Button variant="outline" size="xs" aria-expanded={outputsListed} aria-controls={outputListId} onClick={() => toggleOutputs(index)}>
                            {outputsListed ? `Hide ${outputNoun}` : `Show ${listedOutputs.length} ${outputNoun}`}
                        </Button>
                    )}
                    {!listedOutputs && view.openOutput && (
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
                    {!listedOutputs && view.revealOutput && (
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
                {view.canRetryWithPassword && onRetryWithPassword && !live && passwordRow === index && (
                    <PasswordRetryForm
                        name={name}
                        busy={busy}
                        onRetry={(password) => void runAction(index, "retry", () => onRetryWithPassword(result.inputPath, password))}
                        onCancel={() => setPasswordRow(null)}
                    />
                )}
                {fontsOpen && view.fonts.length > 0 && (
                    <ul id={fontListId} className="result-row-fonts" aria-label={`Fonts used in ${name}`}>
                        {view.fonts.map((font, fontIndex) => (
                            <li key={fontIndex}>
                                <span className="result-row-font-requested">{font.requested}</span>
                                <span aria-hidden="true">→</span>
                                <span className="sr-only">used</span>
                                <span>{font.used}</span>
                                <span className="result-row-detail">{font.source}</span>
                            </li>
                        ))}
                    </ul>
                )}
                {listedOutputs && outputsListed && (
                    <ul id={outputListId} className="result-outputs" aria-label={`${outputNoun?.replace(/^./, (letter) => letter.toUpperCase())} from ${name}`}>
                        {listedOutputs.map((path) => {
                            const output = fileName(path);
                            return (
                                <li key={path}>
                                    <span className="result-output-name" title={path}>{output}</span>
                                    <Button variant="ghost" size="xs" aria-label={`Open ${output}`} disabled={busy} onClick={() => void runAction(index, "open", () => invoke("open_output_path", { path }))}>
                                        Open
                                    </Button>
                                    <Button variant="ghost" size="xs" aria-label={`${reveal}: ${output}`} disabled={busy} onClick={() => void runAction(index, "reveal", () => invoke("reveal_output_path", { path }))}>
                                        {revealShort}
                                    </Button>
                                </li>
                            );
                        })}
                    </ul>
                )}
                {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
            </li>
            {open && group && partRow(group, index)}
            </Fragment>
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
                        <ul className="result-rows" aria-label="Files and results" data-grouped={outputGroup ? "true" : undefined}>
                            {files.map((file, index) => {
                                if (file.status === "finished") return finishedRow(file.outcome, index);
                                const running = file.status === "running";
                                const Icon = running ? LoaderCircle : Clock;
                                const label = running ? runningVerb : "Waiting";
                                return (
                                    <li key={`${index}:${file.inputPath}`} className="result-row" data-tone={file.status} data-state={file.status}>
                                        {outputGroup && <span aria-hidden="true" />}
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

    const shared = results[0];
    if (combined && combinedInputs && results.length === 1 && shared.state === "succeeded" && shared.outputPaths.length === 1) {
        const output = shared.outputPaths[0];
        const outputName = fileName(output);
        const folder = output.slice(0, Math.max(0, output.length - outputName.length - 1));
        const state = actionStates[0] ?? idleActionState;
        const busy = state.pending !== null;
        const [one, many] = combined.input;
        const inputs = `${combinedInputs.length} ${combinedInputs.length === 1 ? one : many}`;
        const warningCount = shared.warnings.length;
        const done = warningCount === 0 ? "Done." : `Done with ${warningCount} ${warningCount === 1 ? "warning" : "warnings"}.`;
        const inputsHeading = `${inputs} in this ${combined.output}`;
        return (
            <div className="result-list" data-combined="true">
                <div className="result-summary" role="status" aria-live="polite">
                    <strong>{done} Made 1 {combined.output} from {inputs}</strong>
                    <span className="result-summary-hint">Every {one} is in, in the order shown.</span>
                </div>
                <Card className="result-output gap-0 py-0">
                    <CardContent className="result-output-content">
                        <FileText className="result-output-icon" aria-hidden="true" />
                        <span className="result-row-text">
                            <strong className="result-row-name" title={output}>{outputName}</strong>
                            {folder && <span className="result-row-detail">in {shortenHomePath(folder, home)}</span>}
                            {shared.warnings.map((warning, index) => (
                                <span key={index} className="result-output-warning">
                                    <TriangleAlert aria-hidden="true" />
                                    {warning.code === "renamedToAvoidReplacing"
                                        ? `Renamed. ${warning.details.chosenName} already exists and was left as it is.`
                                        : warningText(warning)}
                                </span>
                            ))}
                        </span>
                        <span className="result-row-actions">
                            <Button variant="outline" size="sm" aria-label={`Open ${outputName}`} disabled={busy} onClick={() => void runAction(0, "open", () => invoke("open_output_path", { path: output }))}>
                                {state.pending === "open" ? "Opening..." : "Open"}
                            </Button>
                            <Button variant="outline" size="sm" aria-label={`${reveal}: ${outputName}`} disabled={busy} onClick={() => void runAction(0, "reveal", () => invoke("reveal_output_path", { path: output }))}>
                                {state.pending === "reveal" ? "Showing..." : reveal}
                            </Button>
                        </span>
                        {state.error && <p className="result-action-error" role="alert">{state.error}</p>}
                    </CardContent>
                </Card>
                <h2 className="result-inputs-heading">{inputsHeading}</h2>
                <Card className="result-card gap-0 py-0">
                    <CardContent className="p-0">
                        <ul className="result-rows" aria-label={inputsHeading}>
                            {combinedInputs.map((input, index) => (
                                <li key={`${index}:${input}`} className="result-row" data-tone="ok" data-state="succeeded">
                                    <span className="result-row-icon" aria-hidden="true"><Check /></span>
                                    <span className="result-row-text">
                                        <span className="result-row-name" title={input}>{fileName(input)}</span>
                                        <span className="result-row-detail">{combined.part(index + 1)}</span>
                                    </span>
                                    <span className="result-row-state">Added</span>
                                    <span className="result-row-actions" />
                                </li>
                            ))}
                        </ul>
                    </CardContent>
                </Card>
            </div>
        );
    }

    const summary = resultSummary(results, resultVerb, resultNoun);
    const total = batchTotal(results, resultVerb);
    const extraHint = resultsHint?.(results) ?? null;
    return (
        <div className="result-list">
            {customSummary ?? (
                <div className="result-summary" role="status" aria-live="polite">
                    <strong>{summary.headline}</strong>
                    {extraHint && <span className="result-summary-hint">{extraHint}</span>}
                    {summary.hints.map((hint) => <span key={hint} className="result-summary-hint">{hint}</span>)}
                    {total && (
                        <span className="result-summary-total">
                            <span className="result-summary-saved">{total.savedLine}</span>
                            <span className="result-summary-hint">{total.totalLine}</span>
                            <span className="result-summary-hint">{total.note}</span>
                        </span>
                    )}
                </div>
            )}
            <Card className="result-card gap-0 py-0">
                <CardContent className="p-0">
                    <ul className="result-rows" aria-label="Results" data-grouped={outputGroup ? "true" : undefined}>
                        {results.map(finishedRow)}
                    </ul>
                </CardContent>
            </Card>
        </div>
    );
};
