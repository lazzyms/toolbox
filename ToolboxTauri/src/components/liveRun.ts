import type { JobOutcome } from "../contracts";
import { parseJobOutcome } from "../jobOutcome";

export const BATCH_PROGRESS_EVENT = "batch-progress";

export type RunEvent =
    | { runId: string; index: number; phase: "started" }
    | { runId: string; index: number; phase: "finished"; outcome: JobOutcome };

export interface LiveRun {
    runId: string;
    paths: string[];
    running: ReadonlySet<number>;
    finished: ReadonlyArray<JobOutcome | undefined>;
}

export type LiveFile =
    | { status: "finished"; outcome: JobOutcome }
    | { status: "running" | "waiting"; inputPath: string };

export const startRun = (runId: string, paths: string[]): LiveRun => ({
    runId,
    paths,
    running: new Set(),
    finished: paths.map(() => undefined),
});

export const parseRunEvent = (value: unknown): RunEvent | null => {
    if (typeof value !== "object" || value === null) return null;
    const { runId, index, phase, outcome } = value as Record<string, unknown>;
    if (typeof runId !== "string" || typeof index !== "number" || !Number.isInteger(index) || index < 0) return null;
    if (phase === "started") return { runId, index, phase };
    if (phase !== "finished") return null;
    try {
        return { runId, index, phase, outcome: parseJobOutcome(outcome, "batch-progress outcome") };
    } catch {
        return null;
    }
};

export const foldRunEvent = (run: LiveRun, event: RunEvent): LiveRun => {
    if (event.runId !== run.runId || event.index >= run.paths.length || run.finished[event.index]) return run;
    const running = new Set(run.running);
    if (event.phase === "started") {
        running.add(event.index);
        return { ...run, running };
    }
    running.delete(event.index);
    const finished = [...run.finished];
    finished[event.index] = event.outcome;
    return { ...run, running, finished };
};

export const liveFiles = (run: LiveRun): LiveFile[] =>
    run.paths.map((inputPath, index) => {
        const outcome = run.finished[index];
        if (outcome) return { status: "finished", outcome };
        return { status: run.running.has(index) ? "running" : "waiting", inputPath };
    });
