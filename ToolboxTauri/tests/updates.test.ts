import assert from "node:assert/strict";
import test from "node:test";
import {
  inAppUpdatesEnabled,
  scheduleStartupUpdateCheck,
  startupUpdateCheckEnabled,
  type AvailableUpdate,
} from "../src/updates";

test("GitHub and development builds offer in-app updates", () => {
  assert.equal(inAppUpdatesEnabled({ VITE_ENABLE_UPDATES: "true" }), true);
  assert.equal(inAppUpdatesEnabled({}), true);
});

test("the Microsoft Store build leaves updates to the Store", () => {
  assert.equal(inAppUpdatesEnabled({ VITE_ENABLE_UPDATES: "false" }), false);
});

test("only release GitHub builds check for updates at launch", () => {
  assert.equal(startupUpdateCheckEnabled({ DEV: false, VITE_ENABLE_UPDATES: "true" }), true);
  assert.equal(startupUpdateCheckEnabled({ DEV: false }), true);
  assert.equal(startupUpdateCheckEnabled({ DEV: true, VITE_ENABLE_UPDATES: "true" }), false);
  assert.equal(startupUpdateCheckEnabled({ DEV: false, VITE_ENABLE_UPDATES: "false" }), false);
});

const release = { DEV: false, VITE_ENABLE_UPDATES: "true" };
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

const harness = (options: { update?: AvailableUpdate | null; approve?: boolean; checkError?: Error } = {}) => {
  const calls: string[] = [];
  const prompts: string[] = [];
  const update = options.update === undefined
    ? { version: "9.9.9", downloadAndInstall: async () => { calls.push("install"); } }
    : options.update;
  const actions = {
    check: async () => {
      calls.push("check");
      if (options.checkError) throw options.checkError;
      return update;
    },
    confirm: async (message: string) => {
      calls.push("confirm");
      prompts.push(message);
      return options.approve ?? false;
    },
    relaunch: async () => { calls.push("relaunch"); },
  };
  return { calls, prompts, actions };
};

test("launch waits before checking, then asks before installing and restarting", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { calls, prompts, actions } = harness({ approve: true });
  scheduleStartupUpdateCheck(actions, release, 3000);

  t.mock.timers.tick(2999);
  assert.deepEqual(calls, []);

  t.mock.timers.tick(1);
  t.mock.timers.reset();
  await settle();
  assert.deepEqual(calls, ["check", "confirm", "install", "relaunch"]);
  assert.match(prompts[0], /Toolbox 9\.9\.9 is available/);
});

test("declining the launch prompt installs nothing", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { calls, actions } = harness({ approve: false });
  scheduleStartupUpdateCheck(actions, release, 10);
  t.mock.timers.tick(10);
  t.mock.timers.reset();
  await settle();
  assert.deepEqual(calls, ["check", "confirm"]);
});

test("an up-to-date launch shows no prompt", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { calls, actions } = harness({ update: null });
  scheduleStartupUpdateCheck(actions, release, 10);
  t.mock.timers.tick(10);
  t.mock.timers.reset();
  await settle();
  assert.deepEqual(calls, ["check"]);
});

test("a failed launch check stays quiet", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const warn = t.mock.method(console, "warn", () => undefined);
  const { calls, actions } = harness({ checkError: new Error("offline") });
  scheduleStartupUpdateCheck(actions, release, 10);
  t.mock.timers.tick(10);
  t.mock.timers.reset();
  await settle();
  assert.deepEqual(calls, ["check"]);
  assert.equal(warn.mock.callCount(), 1);
});

test("cancelling before the delay skips the check", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { calls, actions } = harness();
  const cancel = scheduleStartupUpdateCheck(actions, release, 10);
  cancel();
  t.mock.timers.tick(10);
  assert.deepEqual(calls, []);
});

test("development and Store builds never check at launch", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  for (const env of [{ DEV: true }, { DEV: false, VITE_ENABLE_UPDATES: "false" }]) {
    const { calls, actions } = harness();
    scheduleStartupUpdateCheck(actions, env, 10);
    t.mock.timers.tick(10);
    assert.deepEqual(calls, []);
  }
});
