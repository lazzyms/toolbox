import type { PackStatus } from '../../views/PacksSection';

/** A script Edit PDF writes with a font from a pack, as the `script_packs` query reports it. */
export interface ScriptPackState {
  /** The Unicode script's full name, as in `Devanagari`. */
  script: string;
  packId: string;
  packName: string;
  status: PackStatus;
  downloadBytes: number | null;
}

/** Text that can't be exported until its script's pack is usable. */
export interface ScriptPackNeed {
  /** The first word of `text` in that script, as in `आशा`. */
  sample: string;
  state: ScriptPackState;
}

export const scriptPackUsable = (state: ScriptPackState) => state.status === 'installed' || state.status === 'updateAvailable';

/**
 * The first script in `text` whose pack isn't usable. The query names every script a pack writes,
 * so text the app writes itself, as in Latin, Cyrillic and Greek, never matches. Export checks the
 * pack again at its run, so this only warns early.
 */
export const scriptPackNeed = (text: string, states: readonly ScriptPackState[]): ScriptPackNeed | null => {
  let first: { index: number; need: ScriptPackNeed } | null = null;
  for (const state of states) {
    if (scriptPackUsable(state)) continue;
    let word: RegExp;
    try {
      word = new RegExp(`\\p{Script=${state.script}}[\\p{Script=${state.script}}\\p{Script=Inherited}]*`, 'u');
    } catch {
      continue;
    }
    const match = word.exec(text);
    if (match && (!first || match.index < first.index)) first = { index: match.index, need: { sample: match[0], state } };
  }
  return first?.need ?? null;
};
