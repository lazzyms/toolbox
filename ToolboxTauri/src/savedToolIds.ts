import type { AtomicToolId } from "./contracts";
import { RetiredToolIds, registeredToolIds, type RetiredToolIdTable } from "./registry/retiredToolIds";

export const favoritesStorageKey = "toolbox-favorites";
export const recentStorageKey = "toolbox-recent";
export const toolIdMigrationStorageKey = "toolbox-tool-id-migration";

type SavedToolIdStorage = Pick<Storage, "getItem" | "setItem">;

const rewriteToolIds = (ids: unknown[], table: RetiredToolIdTable): AtomicToolId[] => {
  const rewritten: AtomicToolId[] = [];
  for (const id of ids) {
    if (typeof id !== "string") continue;
    const current = table.replacements.get(id) ?? id;
    if (registeredToolIds.has(current) && !rewritten.includes(current as AtomicToolId)) {
      rewritten.push(current as AtomicToolId);
    }
  }
  return rewritten;
};

const savedList = (storage: SavedToolIdStorage, key: string): unknown[] | null => {
  const raw = storage.getItem(key);
  if (raw === null) return null;
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
};

export const migrateSavedToolIds = (storage: SavedToolIdStorage, table: RetiredToolIdTable = RetiredToolIds) => {
  const migratedVersion = Number(storage.getItem(toolIdMigrationStorageKey) ?? 0);
  if (migratedVersion >= table.version) return;
  // Each list is rewritten in one setItem and the marker goes last, so a crash leaves
  // every list either untouched or fully rewritten, and the rewrite is safe to repeat.
  for (const key of [favoritesStorageKey, recentStorageKey]) {
    const ids = savedList(storage, key);
    if (ids) storage.setItem(key, JSON.stringify(rewriteToolIds(ids, table)));
  }
  storage.setItem(toolIdMigrationStorageKey, String(table.version));
};
