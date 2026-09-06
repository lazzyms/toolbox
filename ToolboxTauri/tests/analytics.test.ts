import assert from "node:assert/strict";
import test from "node:test";
import { INSTALL_MARKER, recordFirstInstall } from "../src/analytics";

class MemoryStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

test("records the first install once and ignores later app launches", () => {
  const storage = new MemoryStorage();
  const events: Array<{ name: string; parameters: Record<string, string> }> = [];
  const logEvent = (name: string, parameters: Record<string, string>) => {
    events.push({ name, parameters });
  };

  assert.equal(
    recordFirstInstall(storage, logEvent),
    "recorded",
  );
  assert.deepEqual(events, [
    { name: "first_install", parameters: { app_platform: "tauri" } },
  ]);
  assert.equal(storage.getItem(INSTALL_MARKER), "1");

  assert.equal(
    recordFirstInstall(storage, logEvent),
    "already-recorded",
  );
  assert.equal(events.length, 1);
});
