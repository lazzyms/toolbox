import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import test from "node:test";
import {
  captureEvent,
  DISTINCT_ID_KEY,
  initializeInstallAnalyticsWith,
  INSTALL_MARKER,
  isAnalyticsOptedOut,
  isConfigured,
  recordFirstInstall,
  setAnalyticsOptOut,
} from "../src/analytics";

class MemoryStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

interface CapturedCall {
  url: string;
  init: Record<string, unknown>;
}

function mockFetch(calls: CapturedCall[]) {
  return async (input: string, init?: Record<string, unknown>) => {
    calls.push({ url: input, init: init ?? {} });
  };
}

// Each test that needs a configured key sets it explicitly and restores the
// previous value, so tests never depend on ambient environment state.
async function withPosthogKey<T>(
  key: string | undefined,
  fn: () => T | Promise<T>,
): Promise<T> {
  const previous = process.env.VITE_POSTHOG_KEY;
  if (key === undefined) delete process.env.VITE_POSTHOG_KEY;
  else process.env.VITE_POSTHOG_KEY = key;
  try {
    return await fn();
  } finally {
    if (previous === undefined) delete process.env.VITE_POSTHOG_KEY;
    else process.env.VITE_POSTHOG_KEY = previous;
  }
}

test("records the first install once and ignores later app launches", () => {
  const storage = new MemoryStorage();
  const events: Array<{ name: string }> = [];
  const logEvent = (name: string) => {
    events.push({ name });
  };

  assert.equal(recordFirstInstall(storage, logEvent), "recorded");
  assert.deepEqual(events, [{ name: "first_install" }]);
  assert.equal(storage.getItem(INSTALL_MARKER), "1");

  assert.equal(recordFirstInstall(storage, logEvent), "already-recorded");
  assert.equal(events.length, 1);
});

test("opt-out persists in storage", () => {
  const storage = new MemoryStorage();
  assert.equal(isAnalyticsOptedOut(storage), false);

  setAnalyticsOptOut(storage, true);
  assert.equal(isAnalyticsOptedOut(storage), true);

  setAnalyticsOptOut(storage, false);
  assert.equal(isAnalyticsOptedOut(storage), false);
});

test("isConfigured requires a PostHog project key", async () => {
  await withPosthogKey(undefined, () => assert.equal(isConfigured(), false));
  await withPosthogKey("", () => assert.equal(isConfigured(), false));
  await withPosthogKey("not-a-key", () => assert.equal(isConfigured(), false));
  await withPosthogKey("phc_testkey123456", () =>
    assert.equal(isConfigured(), true),
  );
});

test("sends nothing when no PostHog key is configured", async () => {
  await withPosthogKey(undefined, async () => {
    const calls: CapturedCall[] = [];
    const storage = new MemoryStorage();
    await initializeInstallAnalyticsWith({
      storage,
      isDev: false,
      fetchImpl: mockFetch(calls),
    });
    assert.equal(calls.length, 0);
  });
});

test("does not burn the first_install marker while unconfigured", async () => {
  const calls: CapturedCall[] = [];
  const storage = new MemoryStorage();
  const deps = {
    storage,
    isDev: false,
    fetchImpl: mockFetch(calls),
  };

  await withPosthogKey(undefined, () => initializeInstallAnalyticsWith(deps));
  assert.equal(storage.getItem(INSTALL_MARKER), null);

  await withPosthogKey("phc_testkey123456", () =>
    initializeInstallAnalyticsWith(deps),
  );
  const events = calls.map((call) =>
    JSON.parse(call.init.body as string).event,
  );
  assert.deepEqual(events, ["first_install", "app_opened"]);
});

test("sends nothing in dev builds", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    await initializeInstallAnalyticsWith({
      storage: new MemoryStorage(),
      isDev: true,
      fetchImpl: mockFetch(calls),
    });
    assert.equal(calls.length, 0);
  });
});

test("sends nothing when the user opted out", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    const storage = new MemoryStorage();
    setAnalyticsOptOut(storage, true);
    await initializeInstallAnalyticsWith({
      storage,
      isDev: false,
      fetchImpl: mockFetch(calls),
    });
    assert.equal(calls.length, 0);
  });
});

test("reuses the anonymous id and records first install only once", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    const storage = new MemoryStorage();
    const deps = {
      storage,
      isDev: false,
      fetchImpl: mockFetch(calls),
    };

    await initializeInstallAnalyticsWith(deps);
    await initializeInstallAnalyticsWith(deps);

    const events = calls.map((call) =>
      JSON.parse(call.init.body as string).event,
    );
    assert.deepEqual(events, ["first_install", "app_opened", "app_opened"]);

    const distinctIds = new Set(
      calls.map((call) => JSON.parse(call.init.body as string).distinct_id),
    );
    assert.equal(distinctIds.size, 1);
    assert.equal(storage.getItem(DISTINCT_ID_KEY), [...distinctIds][0]);
  });
});

test("capture payload carries api key, event, distinct id and properties", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    await captureEvent(
      "app_opened",
      "test-distinct-id",
      mockFetch(calls),
    );

    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "https://us.i.posthog.com/capture/");
    const body = JSON.parse(calls[0].init.body as string);
    assert.equal(body.api_key, "phc_testkey123456");
    assert.equal(body.event, "app_opened");
    assert.equal(body.distinct_id, "test-distinct-id");
    assert.equal(body.properties.app_platform, "tauri");
    assert.equal(body.properties.$lib, "toolbox-tauri");
    assert.equal(body.properties.$geoip_disable, true);
  });
});

test("capture respects a custom capture url (EU projects)", async () => {
  const previous = process.env.VITE_POSTHOG_CAPTURE_URL;
  process.env.VITE_POSTHOG_CAPTURE_URL = "https://eu.i.posthog.com/capture/";
  try {
    await withPosthogKey("phc_testkey123456", async () => {
      const calls: CapturedCall[] = [];
      await captureEvent(
        "app_opened",
        "test-distinct-id",
        mockFetch(calls),
      );
      assert.equal(calls[0].url, "https://eu.i.posthog.com/capture/");
    });
  } finally {
    if (previous === undefined) delete process.env.VITE_POSTHOG_CAPTURE_URL;
    else process.env.VITE_POSTHOG_CAPTURE_URL = previous;
  }
});

// Packs add no analytics: installing, updating, repairing and uninstalling a pack send no event
// and no property beyond the fixed launch set.
const projectRoot = join(import.meta.dirname, "..");
const PACK_COMMAND = /invoke(?:<[^>]*>)?\("(?:list_packs|install_pack|install_pack_file|inspect_pack_file|uninstall_pack)"/;

function sourceFiles(folder: string): string[] {
  return readdirSync(folder).flatMap((name) => {
    const path = join(folder, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx|rs)$/.test(name) ? [path] : [];
  });
}

/** Every project module `files` import, directly or through other modules. */
function importClosure(files: string[]): Set<string> {
  const seen = new Set<string>();
  const pending = [...files];
  while (pending.length) {
    const file = pending.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    for (const [, specifier] of readFileSync(file, "utf8").matchAll(/(?:from|import)\s*\(?\s*"(\.{1,2}\/[^"]+)"/g)) {
      const base = join(dirname(file), specifier);
      const resolved = [base, `${base}.ts`, `${base}.tsx`, join(base, "index.ts"), join(base, "index.tsx")]
        .find((candidate) => existsSync(candidate) && statSync(candidate).isFile());
      if (resolved && /\.tsx?$/.test(resolved)) pending.push(resolved);
    }
  }
  return seen;
}

test("pack lifecycle events are never sent", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    for (const event of ["pack_installed", "pack_updated", "pack_repaired", "pack_uninstalled", "install_pack"]) {
      await captureEvent(event as Parameters<typeof captureEvent>[0], "test-distinct-id", mockFetch(calls));
    }
    assert.equal(calls.length, 0);
  });
});

test("every event carries exactly the fixed launch properties", async () => {
  await withPosthogKey("phc_testkey123456", async () => {
    const calls: CapturedCall[] = [];
    await initializeInstallAnalyticsWith({ storage: new MemoryStorage(), isDev: false, fetchImpl: mockFetch(calls) });
    const bodies = calls.map((call) => JSON.parse(call.init.body as string));
    assert.deepEqual(bodies.map((body) => body.event), ["first_install", "app_opened"]);
    for (const body of bodies) {
      assert.deepEqual(Object.keys(body).sort(), ["api_key", "distinct_id", "event", "properties"]);
      assert.deepEqual(body.properties, { app_platform: "tauri", $lib: "toolbox-tauri", $geoip_disable: true });
    }
  });
});

test("the code that installs, updates, repairs and uninstalls packs cannot reach analytics", () => {
  const callers = sourceFiles(join(projectRoot, "src")).filter((file) => PACK_COMMAND.test(readFileSync(file, "utf8")));
  assert.deepEqual(callers.map((file) => relative(projectRoot, file).replaceAll("\\", "/")).sort(), [
    "src/components/ResultList.tsx", "src/views/OcrLanguagePicker.tsx", "src/views/PacksSection.tsx",
  ]);
  const reached = [...importClosure(callers)].map((file) => relative(projectRoot, file).replaceAll("\\", "/"));
  assert.deepEqual(reached.filter((file) => /^src\/(analytics|toolFlags)\.ts$/.test(file)), []);

  const nativePacks = [join(projectRoot, "src-tauri/src/packs.rs"), ...sourceFiles(join(projectRoot, "src-tauri/src/packs"))];
  for (const file of nativePacks) {
    assert.doesNotMatch(readFileSync(file, "utf8"), /posthog|analytics|capture_event/i, relative(projectRoot, file));
  }
});
