import type { JobOutcome } from "../contracts";
import { parseJobOutcome } from "../jobOutcome";

export const BATCH_PROGRESS_EVENT = "batch-progress";

export type RunEvent =
    | { runId: string; index: number; phase: "started" }
    | { runId: string; index: number; phase: "finished"; outcome: JobOutcome }
    | { runId: string; index: number; phase: "progress"; completed: number; total: number };

export type FileProgress = { completed: number; total: number };

export interface LiveRun {
    runId: string;
    paths: string[];
    running: ReadonlySet<number>;
    finished: ReadonlyArray<JobOutcome | undefined>;
    /** How far each running file has got, for jobs that report it, such as a PDF export's pages. */
    steps: ReadonlyArray<FileProgress | undefined>;
}

export type LiveFile =
    | { status: "finished"; outcome: JobOutcome }
    | { status: "running" | "waiting"; inputPath: string };

export const startRun = (runId: string, paths: string[]): LiveRun => ({
    runId,
    paths,
    running: new Set(),
    finished: paths.map(() => undefined),
    steps: paths.map(() => undefined),
});

export const parseRunEvent = (value: unknown): RunEvent | null => {
    if (typeof value !== "object" || value === null) return null;
    const { runId, index, phase, outcome, completed, total } = value as Record<string, unknown>;
    if (typeof runId !== "string" || typeof index !== "number" || !Number.isInteger(index) || index < 0) return null;
    if (phase === "started") return { runId, index, phase };
    if (phase === "progress") {
        const count = (n: unknown): n is number => typeof n === "number" && Number.isInteger(n) && n >= 0;
        return count(completed) && count(total) && completed <= total && total > 0 ? { runId, index, phase, completed, total } : null;
    }
    if (phase !== "finished") return null;
    try {
        return { runId, index, phase, outcome: parseJobOutcome(outcome, "batch-progress outcome") };
    } catch {
        return null;
    }
};

export const foldRunEvent = (run: LiveRun, event: RunEvent): LiveRun => {
    if (event.runId !== run.runId || event.index >= run.paths.length || run.finished[event.index]) return run;
    if (event.phase === "progress") {
        const steps = [...run.steps];
        steps[event.index] = { completed: event.completed, total: event.total };
        return { ...run, steps };
    }
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
