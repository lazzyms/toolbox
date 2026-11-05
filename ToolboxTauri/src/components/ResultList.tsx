import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Check, CircleMinus, CircleX, Clock, KeyRound, LoaderCircle, RotateCcw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome, OutcomePointer, PackNeed, Progress } from "../contracts";
import { formatBytes, PACK_CHANGED_EVENT, PROGRESS_EVENT, type PackProgress } from "../views/PacksSection";
import { liveFiles, type LiveFile, type LiveRun } from "./liveRun";
import { fileName, neededPack, resultSummary, resultTally, rowView, type RowTone } from "./resultRow";
import { PasswordRetryForm } from "./PasswordFields";

type RowAction = "open" | "reveal" | "retry" | "install";

type PackPointer = Extract<OutcomePointer, { kind: "pack" }>;

/** A pack install started from a result row, keyed by pack id. */
type PackJob = { runId: string; reason: PackNeed; name: string; receivedBytes: number; totalBytes: number };

const packJobVerbs: Record<PackNeed, string> = { notInstalled: "Installing", damaged: "Repairing", incompatible: "Updating" };

/** "7 of 12 MB", or "900 KB of 12 MB" when the units differ. */
const bytesOf = (received: number, total: number) => {
    const [done, all] = [formatBytes(received), formatBytes(total)];
    const unit = all.split(" ")[1];
    return `${done.endsWith(` ${unit}`) ? done.slice(0, -unit.length - 1) : done} of ${all}`;
};

const filesCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

/** The summary lines for packs being installed from the rows, or installed since the run. */
const packHints = (results: JobOutcome[], jobs: Record<string, PackJob>, installed: ReadonlySet<string>) => {
    const needs = new Map<string, { name: string; count: number }>();
    for (const result of results) {
        const pack = neededPack(result);
        if (!pack) continue;
        const entry = needs.get(pack.packId) ?? { name: pack.name, count: 0 };
        needs.set(pack.packId, { ...entry, count: entry.count + 1 });
    }
    return [...needs].flatMap(([packId, { name, count }]) => {
        if (installed.has(packId)) return [`${name} pack installed. ${filesCount(count)} ${count === 1 ? "is" : "are"} ready to run again.`];
        const job = jobs[packId];
        if (job) return [`${packJobVerbs[job.reason]} the ${name} pack. The ${count === 1 ? `${name} file stays` : `${count} ${name} files stay`} in the list.`];
        return [];
    });
};

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

const followPointer = (pointer: Exclude<OutcomePointer, PackPointer>) => {
    if (pointer.kind === "tool") {
        window.dispatchEvent(new CustomEvent("toolbox:open", { detail: pointer.toolId }));
    } else {
        window.dispatchEvent(new CustomEvent("toolbox:open-settings", { detail: "output-location" }));
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
    onRetry?: (inputPath: string) => Promise<void>;
    onRetryWithPassword?: (inputPath: string, password: string) => Promise<void>;
}

export const ResultList = ({ results, progress, loading, resultVerb, runningVerb, runningDetail, live, namedByOutput = false, onRetry, onRetryWithPassword }: ResultListProps) => {
    const [actionStates, setActionStates] = useState<Record<number, RowActionState>>({});
    const [passwordRow, setPasswordRow] = useState<number | null>(null);
    const [packJobs, setPackJobs] = useState<Record<string, PackJob>>({});
    const [installedPacks, setInstalledPacks] = useState<ReadonlySet<string>>(new Set());
    const resultSetGeneration = useRef(0);

    useEffect(() => {
        let live = true;
        const stops: UnlistenFn[] = [];
        const keep = (unlisten: UnlistenFn) => { if (live) stops.push(unlisten); else unlisten(); };
        // Rows keep their files; a pack installed anywhere marks its rows ready to run again.
        listen<{ packId: string }>(PACK_CHANGED_EVENT, ({ payload }) => {
            setInstalledPacks((current) => new Set(current).add(payload.packId));
        }).then(keep).catch(() => undefined);
        listen<PackProgress>(PROGRESS_EVENT, ({ payload }) => {
            setPackJobs((current) => {
                const job = current[payload.packId];
                if (job?.runId !== payload.runId) return current;
                return { ...current, [payload.packId]: { ...job, receivedBytes: payload.receivedBytes, totalBytes: payload.totalBytes } };
            });
        }).then(keep).catch(() => undefined);
        return () => {
            live = false;
            stops.forEach((stop) => stop());
        };
    }, []);

    useEffect(() => {
        const generation = resultSetGeneration.current + 1;
        resultSetGeneration.current = generation;
        setActionStates({});
        setPasswordRow(null);
        setInstalledPacks(new Set());
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

    const installPack = (row: number, pack: PackPointer) => runAction(row, "install", async () => {
        const runId = crypto.randomUUID();
        setPackJobs((current) => ({ ...current, [pack.packId]: { runId, reason: pack.reason, name: pack.name, receivedBytes: 0, totalBytes: 0 } }));
        try {
            await invoke("install_pack", { packId: pack.packId, runId });
        } catch (reason) {
            // A pack error carries its wording; a cancelled install leaves the row as it was.
            const failure = reason as { kind?: string; message?: string } | null;
            if (failure?.kind === "cancelled") return;
            throw new Error(failure?.message || String(reason));
        } finally {
            setPackJobs((current) => {
                const { [pack.packId]: finished, ...rest } = current;
                return finished?.runId === runId ? rest : current;
            });
        }
    });

    const reveal = revealButtonLabel();

    const finishedRow = (result: JobOutcome, index: number) => {
        const view = rowView(result, resultVerb, namedByOutput, installedPacks);
        const pack = view.pointer?.pointer.kind === "pack" ? view.pointer.pointer : null;
        const packJob = pack ? packJobs[pack.packId] : undefined;
        const state = actionStates[index] ?? idleActionState;
        const busy = state.pending !== null;
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
                    {packJob && (
                        <>
                            <span
                                role="progressbar"
                                aria-label={`${packJobVerbs[packJob.reason]} ${packJob.name} pack`}
                                aria-valuenow={packJob.totalBytes > 0 ? Math.floor(Math.min(packJob.receivedBytes / packJob.totalBytes, 1) * 100) : 0}
                                aria-valuemin={0}
                                aria-valuemax={100}
                                className="pack-progress"
                            >
                                <span style={{ width: `${packJob.totalBytes > 0 ? Math.min(packJob.receivedBytes / packJob.totalBytes, 1) * 100 : 0}%` }} />
                            </span>
                            {packJob.totalBytes > 0 && <span className="result-row-detail">{bytesOf(packJob.receivedBytes, packJob.totalBytes)}</span>}
                        </>
                    )}
                    {view.packInstalled && <Badge variant="outline">Pack installed</Badge>}
                    {view.pointer && !packJob && (
                        pack ? (
                            <Button variant="outline" size="xs" aria-label={`${view.pointer.label} for ${name}`} disabled={busy} onClick={() => void installPack(index, pack)}>
                                {view.pointer.label}
                            </Button>
                        ) : (
                            <Button variant="outline" size="xs" onClick={() => view.pointer && view.pointer.pointer.kind !== "pack" && followPointer(view.pointer.pointer)}>
                                {view.pointer.label}
                            </Button>
                        )
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
    const hints = [...packHints(results, packJobs, installedPacks), ...summary.hints];
    return (
        <div className="result-list">
            <div className="result-summary" role="status" aria-live="polite">
                <strong>{summary.headline}</strong>
                {hints.map((hint) => <span key={hint} className="result-summary-hint">{hint}</span>)}
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
