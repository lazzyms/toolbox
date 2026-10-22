import assert from "node:assert/strict";
import test from "node:test";
import { inAppUpdatesEnabled } from "../src/updates";

test("GitHub and development builds offer in-app updates", () => {
  assert.equal(inAppUpdatesEnabled({ VITE_ENABLE_UPDATES: "true" }), true);
  assert.equal(inAppUpdatesEnabled({}), true);
});

test("the Microsoft Store build leaves updates to the Store", () => {
  assert.equal(inAppUpdatesEnabled({ VITE_ENABLE_UPDATES: "false" }), false);
});
