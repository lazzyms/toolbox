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
    | { status: "running" | "waiting"; inputPath: string; steps?: FileProgress };

export const startRun = (runId: string, paths: string[]): LiveRun => ({
    runId,
    paths,
    running: new Set(),
    finished: paths.map(() => undefined),
    steps: paths.map(() => undefined),
});

const isCount = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= 0;

export const parseRunEvent = (value: unknown): RunEvent | null => {
    if (typeof value !== "object" || value === null) return null;
    const { runId, index, phase, outcome, completed, total } = value as Record<string, unknown>;
    if (typeof runId !== "string" || !isCount(index)) return null;
    if (phase === "started") return { runId, index, phase };
    if (phase === "progress") return isCount(completed) && isCount(total) && completed <= total && total > 0 ? { runId, index, phase, completed, total } : null;
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
        return { status: run.running.has(index) ? "running" : "waiting", inputPath, steps: run.steps[index] };
    });

/** Pages done across the run, out of the pages known so far, for tools whose files report pages. A finished file counts the pages it reached. */
export const pageTally = (run: LiveRun): FileProgress =>
    run.steps.reduce<FileProgress>((tally, pages, index) => {
        if (!pages) return tally;
        const total = run.finished[index] ? pages.completed : pages.total;
        return { completed: tally.completed + pages.completed, total: tally.total + total };
    }, { completed: 0, total: 0 });

/** The page being worked on, counting from 1, for a file that reports pages. */
export const currentPage = ({ completed, total }: FileProgress): number => Math.min(completed + 1, total);
