import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { parseRetiredToolIds, registeredToolIds, RetiredToolIds } from "../src/registry/retiredToolIds.ts";
import {
  favoritesStorageKey,
  migrateSavedToolIds,
  recentStorageKey,
  toolIdMigrationStorageKey,
} from "../src/savedToolIds.ts";

class MemoryStorage {
  readonly items = new Map<string, string>();
  writes = 0;
  failOnKey: string | null = null;

  constructor(seed: Record<string, unknown> = {}) {
    for (const [key, value] of Object.entries(seed)) this.items.set(key, JSON.stringify(value));
  }

  getItem(key: string) {
    return this.items.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    if (key === this.failOnKey) throw new Error("crashed mid-migration");
    this.writes += 1;
    this.items.set(key, value);
  }

  list(key: string) {
    return JSON.parse(this.items.get(key) ?? "null");
  }
}

const fixtureTable = parseRetiredToolIds({
  version: 2,
  retired: {
    "file-unlock": "pdf-unlock",
    "pdf-compress-legacy": "pdf-compress",
    "pdf-remove-blank-pages": "pdf-organize",
    "pdf-inspect": "removed",
  },
});

const seededLists = () => ({
  [favoritesStorageKey]: ["file-unlock", "pdf-merge", "pdf-unlock", "pdf-inspect", "mystery-tool", "pdf-merge", 7],
  [recentStorageKey]: ["pdf-remove-blank-pages", "pdf-compress-legacy", "pdf-organize", "never-shipped", "pdf-compress"],
});

const migratedFavorites = ["pdf-unlock", "pdf-merge"];
const migratedRecent = ["pdf-organize", "pdf-compress"];

test("saved favorites and recents move to replacements, drop removed and unknown ids, and keep first positions", () => {
  const storage = new MemoryStorage(seededLists());

  migrateSavedToolIds(storage, fixtureTable);

  assert.deepEqual(storage.list(favoritesStorageKey), migratedFavorites);
  assert.deepEqual(storage.list(recentStorageKey), migratedRecent);
  assert.equal(storage.getItem(toolIdMigrationStorageKey), "2");
});

test("the rewrite does not run again on a later launch", () => {
  const storage = new MemoryStorage(seededLists());
  migrateSavedToolIds(storage, fixtureTable);
  storage.items.set(favoritesStorageKey, JSON.stringify(["mystery-tool", "file-unlock"]));
  const writes = storage.writes;

  migrateSavedToolIds(storage, fixtureTable);

  assert.equal(storage.writes, writes, "no storage writes on the second launch");
  assert.deepEqual(storage.list(favoritesStorageKey), ["mystery-tool", "file-unlock"]);
});

test("a table with new rows runs once more from the stored marker", () => {
  const storage = new MemoryStorage({ ...seededLists(), [toolIdMigrationStorageKey]: 1 });
  migrateSavedToolIds(storage, fixtureTable);
  assert.deepEqual(storage.list(favoritesStorageKey), migratedFavorites);
  assert.equal(storage.getItem(toolIdMigrationStorageKey), "2");
});

test("a crash mid-migration keeps every favorite and the next launch finishes the same rewrite", () => {
  const storage = new MemoryStorage(seededLists());
  storage.failOnKey = recentStorageKey;

  assert.throws(() => migrateSavedToolIds(storage, fixtureTable), /crashed mid-migration/);
  assert.deepEqual(storage.list(favoritesStorageKey), migratedFavorites);
  assert.deepEqual(storage.list(recentStorageKey), seededLists()[recentStorageKey], "the unwritten list is untouched");
  assert.equal(storage.getItem(toolIdMigrationStorageKey), null);

  storage.failOnKey = null;
  migrateSavedToolIds(storage, fixtureTable);

  assert.deepEqual(storage.list(favoritesStorageKey), migratedFavorites);
  assert.deepEqual(storage.list(recentStorageKey), migratedRecent);
  assert.equal(storage.getItem(toolIdMigrationStorageKey), "2");
});

test("unsaved and unreadable lists are left as they are", () => {
  const storage = new MemoryStorage();
  storage.items.set(recentStorageKey, "{not json");

  migrateSavedToolIds(storage, fixtureTable);

  assert.equal(storage.getItem(favoritesStorageKey), null);
  assert.equal(storage.getItem(recentStorageKey), "{not json");
  assert.equal(storage.getItem(toolIdMigrationStorageKey), "2");
});

test("the shipped table runs at most once and leaves a marker only when it has a version", () => {
  const storage = new MemoryStorage(seededLists());
  migrateSavedToolIds(storage);
  migrateSavedToolIds(storage);
  assert.equal(storage.writes, RetiredToolIds.version === 0 ? 0 : 3);
  assert.equal(storage.getItem(toolIdMigrationStorageKey), RetiredToolIds.version === 0 ? null : String(RetiredToolIds.version));
});

test("the retired tool id table rejects ids that are still registered or map nowhere", () => {
  const table = (retired: Record<string, unknown>, version: unknown = 1) => () => parseRetiredToolIds({ version, retired });
  assert.throws(table({ "pdf-unlock": "pdf-protect" }), /still a registered tool/);
  assert.throws(table({ "file-unlock": "file-lock" }), /registered tool id or removed/);
  assert.throws(table({ "file-unlock": "file-lock", "file-lock": "pdf-unlock" }), /registered tool id or removed/);
  assert.throws(table({ "file-unlock": null }), /registered tool id or removed/);
  assert.throws(table({}, -1), /non-negative integer version/);
  assert.throws(() => parseRetiredToolIds({ version: 1, retired: {}, aliases: {} }), /only version and retired/);
});

const rustSources = (directory: URL): string[] =>
  readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = new URL(entry.name + (entry.isDirectory() ? "/" : ""), directory);
    if (entry.isDirectory()) return rustSources(path);
    return entry.name.endsWith(".rs") ? [readFileSync(path, "utf8")] : [];
  });

const declaredPointerTargets = () => {
  const sources = [
    ...rustSources(new URL("../src-tauri/src/", import.meta.url)),
    readFileSync(new URL("../shared/job-outcome-samples.json", import.meta.url), "utf8"),
  ];
  return sources.flatMap((source) => [...source.matchAll(/(?:tool_id: |"toolId": )"([^"]+)"/g)].map((match) => match[1]));
};

test("tool pointer targets declared by native tools are registered and never retired", () => {
  const targets = declaredPointerTargets();
  assert.ok(targets.includes("heic-convert"), "the scan finds the declared pointer targets");
  for (const target of targets) {
    assert.ok(registeredToolIds.has(target), `${target} is a registered tool id`);
    assert.ok(!RetiredToolIds.replacements.has(target), `${target} is not a retired tool id`);
  }
});
