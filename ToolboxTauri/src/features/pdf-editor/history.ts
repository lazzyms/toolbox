import type { PdfScene } from './scene';

/** The editor keeps the latest 100 edits. Older ones drop off the start of the undo stack. */
export const SCENE_HISTORY_LIMIT = 100;

/**
 * The whole-scene undo history. `position` counts steps from the opened scene and never wraps, so the last
 * saved point (the opened file, or the last export) keeps its place even after old steps drop off.
 * `saved.anchor` is the step on the current path closest to the saved scene, and `saved.depth` counts the
 * steps from there to the saved scene along a branch that a later edit replaced.
 */
export type SceneHistory = {
  past: PdfScene[]; present: PdfScene; future: PdfScene[]; position: number;
  saved: { scene: PdfScene; anchor: number; depth: number; exported: boolean };
};

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export const startSceneHistory = (scene: PdfScene): SceneHistory => ({
  past: [], present: scene, future: [], position: 0, saved: { scene, anchor: 0, depth: 0, exported: false },
});

/** Adds an edit as one undo step, or folds it into the current step while one continuous edit goes on. */
export function commitScene(history: SceneHistory, next: PdfScene, merge = false): SceneHistory {
  if (same(history.present, next)) return history;
  let { anchor, depth } = history.saved;
  // An edit after undo drops the redo steps. A saved point among them is now reached through this step.
  if (anchor > history.position) { depth += anchor - history.position; anchor = history.position; }
  if (merge) {
    if (anchor === history.position && depth === 0) depth = 1;
    return { ...history, present: next, future: [], saved: { ...history.saved, anchor, depth } };
  }
  return {
    past: [...history.past, history.present].slice(-SCENE_HISTORY_LIMIT), present: next, future: [],
    position: history.position + 1, saved: { ...history.saved, anchor, depth },
  };
}

export const undoScene = (history: SceneHistory): SceneHistory => history.past.length ? {
  ...history, past: history.past.slice(0, -1), present: history.past[history.past.length - 1],
  future: [history.present, ...history.future], position: history.position - 1,
} : history;

export const redoScene = (history: SceneHistory): SceneHistory => history.future.length ? {
  ...history, past: [...history.past, history.present].slice(-SCENE_HISTORY_LIMIT), present: history.future[0],
  future: history.future.slice(1), position: history.position + 1,
} : history;

/** Marks `scene` as exported. It must be a step in this history, matched by identity. */
export function markSceneExported(history: SceneHistory, scene: PdfScene): SceneHistory {
  const offset = history.present === scene ? 0
    : history.past.lastIndexOf(scene) >= 0 ? history.past.lastIndexOf(scene) - history.past.length
      : history.future.indexOf(scene) >= 0 ? history.future.indexOf(scene) + 1 : null;
  if (offset === null) return history;
  return { ...history, saved: { scene, anchor: history.position + offset, depth: 0, exported: true } };
}

/** How many undo steps separate the scene from the opened file, or from the last export. */
export const unexportedChanges = (history: SceneHistory) => same(history.present, history.saved.scene)
  ? 0 : Math.max(1, Math.abs(history.position - history.saved.anchor) + history.saved.depth);

/** The Organize header subtitle after the page count: "No changes", "3 changes not exported", and so on. */
export function changesLabel(history: SceneHistory, exporting: boolean) {
  const changes = unexportedChanges(history);
  if (changes === 0) return history.saved.exported ? 'All changes exported' : history.past.length || history.future.length ? 'No changes' : null;
  const count = `${changes} ${changes === 1 ? 'change' : 'changes'}`;
  return exporting ? `${count}, exporting` : `${count} not exported`;
}
