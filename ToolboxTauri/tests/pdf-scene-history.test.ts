import assert from "node:assert/strict";
import test from "node:test";
import { SCENE_HISTORY_LIMIT, changesLabel, commitScene, markSceneExported, redoScene, startSceneHistory, undoScene, unexportedChanges } from "../src/features/pdf-editor/history";
import type { PdfScene } from "../src/features/pdf-editor/scene";

const scene = (...order: number[]): PdfScene => ({ pages: order.map((sourceIndex) => ({ id: `p${sourceIndex}`, sourceIndex, rotation: 0, width: 612, height: 792, objects: [] })) as PdfScene["pages"] });
const opened = scene(0, 1, 2);

test("a fresh history has no changes and no subtitle", () => {
  const history = startSceneHistory(opened);
  assert.equal(unexportedChanges(history), 0);
  assert.equal(changesLabel(history, false), null);
});

test("each edit is one step that undo and redo move across", () => {
  let history = startSceneHistory(opened);
  history = commitScene(history, scene(1, 0, 2));
  history = commitScene(history, scene(2, 1, 0));
  assert.equal(unexportedChanges(history), 2);
  assert.equal(changesLabel(history, false), "2 changes not exported");
  history = undoScene(history);
  assert.deepEqual(history.present, scene(1, 0, 2));
  assert.equal(changesLabel(history, false), "1 change not exported");
  history = undoScene(history);
  assert.deepEqual(history.present, opened);
  assert.equal(changesLabel(history, false), "No changes");
  assert.equal(undoScene(history), history);
  history = redoScene(redoScene(history));
  assert.deepEqual(history.present, scene(2, 1, 0));
  assert.equal(redoScene(history), history);
});

test("a no-op edit adds no step, and a continuous edit folds into one step", () => {
  let history = startSceneHistory(opened);
  assert.equal(commitScene(history, scene(0, 1, 2)), history);
  history = commitScene(history, scene(1, 0, 2));
  history = commitScene(history, scene(1, 2, 0), true);
  assert.equal(history.past.length, 1);
  assert.equal(unexportedChanges(history), 1);
  assert.deepEqual(undoScene(history).present, opened);
});

test("the history keeps the latest 100 edits", () => {
  let history = startSceneHistory(opened);
  for (let step = 0; step < SCENE_HISTORY_LIMIT + 5; step += 1) history = commitScene(history, scene(step + 3, 1, 2));
  assert.equal(history.past.length, SCENE_HISTORY_LIMIT);
  while (history.past.length) history = undoScene(history);
  assert.deepEqual(history.present, scene(7, 1, 2));
  assert.equal(unexportedChanges(history), 5);
});

test("exporting moves the saved point, and changes count from there", () => {
  let history = commitScene(commitScene(startSceneHistory(opened), scene(1, 0, 2)), scene(2, 1, 0));
  history = markSceneExported(history, history.present);
  assert.equal(changesLabel(history, false), "All changes exported");
  history = undoScene(history);
  assert.equal(changesLabel(history, false), "1 change not exported");
  assert.equal(changesLabel(history, true), "1 change, exporting");
  // An edit after undo drops the exported step, so the export is now two steps away.
  history = commitScene(history, scene(0, 2, 1));
  assert.equal(unexportedChanges(history), 2);
  history = undoScene(history);
  assert.equal(unexportedChanges(history), 1);
});

test("a scene that is no longer in the history is not marked exported", () => {
  const history = commitScene(startSceneHistory(opened), scene(1, 0, 2));
  assert.equal(markSceneExported(history, scene(1, 0, 2)), history);
});
