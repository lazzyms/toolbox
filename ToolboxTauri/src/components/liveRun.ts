import type { JobOutcome } from "../contracts";
import { parseJobOutcome } from "../jobOutcome";

export const BATCH_PROGRESS_EVENT = "batch-progress";

export type RunEvent =
    | { runId: string; index: number; phase: "started" }
    | { runId: string; index: number; phase: "pages"; done: number; total: number }
    | { runId: string; index: number; phase: "finished"; outcome: JobOutcome };

export interface LiveRun {
    runId: string;
    paths: string[];
    running: ReadonlySet<number>;
    finished: ReadonlyArray<JobOutcome | undefined>;
    /** Pages done and in total for files that report them, by index. */
    pages: ReadonlyArray<PageProgress | undefined>;
}

export interface PageProgress { done: number; total: number }

export type LiveFile =
    | { status: "finished"; outcome: JobOutcome }
    | { status: "running" | "waiting"; inputPath: string; pages?: PageProgress };

export const startRun = (runId: string, paths: string[]): LiveRun => ({
    runId,
    paths,
    running: new Set(),
    finished: paths.map(() => undefined),
    pages: paths.map(() => undefined),
});

const isCount = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= 0;

export const parseRunEvent = (value: unknown): RunEvent | null => {
    if (typeof value !== "object" || value === null) return null;
    const { runId, index, phase, outcome, done, total } = value as Record<string, unknown>;
    if (typeof runId !== "string" || !isCount(index)) return null;
    if (phase === "started") return { runId, index, phase };
    if (phase === "pages") return isCount(done) && isCount(total) && done <= total ? { runId, index, phase, done, total } : null;
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
    if (event.phase === "pages") {
        running.add(event.index);
        const pages = [...run.pages];
        pages[event.index] = { done: event.done, total: event.total };
        return { ...run, running, pages };
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
        return { status: run.running.has(index) ? "running" : "waiting", inputPath, pages: run.pages[index] };
    });

/** Pages done across the run, out of the pages known so far. A finished file counts the pages it reached. */
export const pageTally = (run: LiveRun): PageProgress =>
    run.pages.reduce<PageProgress>((tally, pages, index) => {
        if (!pages) return tally;
        const total = run.finished[index] ? pages.done : pages.total;
        return { done: tally.done + pages.done, total: tally.total + total };
    }, { done: 0, total: 0 });

/** The page being worked on, counting from 1, for a file that reports pages. */
export const currentPage = ({ done, total }: PageProgress): number => Math.min(done + 1, total);
