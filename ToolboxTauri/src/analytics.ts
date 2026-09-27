// Anonymous install analytics via PostHog (free tier: 1M events/month, no card).
//
// What is collected: `first_install` (once) and `app_opened` (per launch),
// each with `{ app_platform: "tauri" }`. Nothing else. The distinct_id is a
// random UUID generated on this device — it is not tied to any user, machine,
// or account, and no PII is ever sent.
//
// Setup: create a free PostHog Cloud project at https://posthog.com and set
// VITE_POSTHOG_KEY to its project API key at build time. EU-region projects
// must use "https://eu.i.posthog.com/capture/" as the capture URL (override
// with VITE_POSTHOG_CAPTURE_URL). Until a real key is set, all events are
// silently skipped.

export const INSTALL_MARKER = "toolbox.analytics.install-recorded";
export const DISTINCT_ID_KEY = "toolbox.analytics.distinct-id";
export const OPT_OUT_KEY = "toolbox.analytics.opt-out";

type StorageLike = Pick<Storage, "getItem" | "setItem">;
type AnalyticsEvent = "first_install" | "app_opened";
type EventLogger = (name: AnalyticsEvent) => void;

function readBuildVar(name: string, fallback: string): string {
  const env = (import.meta as { env?: Record<string, string | undefined> }).env;
  const viteValue = env?.[name];
  if (viteValue) return viteValue;

  const processLike: unknown = Reflect.get(globalThis, "process");
  if (typeof processLike !== "object" || processLike === null) return fallback;
  const processEnv: unknown = Reflect.get(processLike, "env");
  if (typeof processEnv !== "object" || processEnv === null) return fallback;
  const processValue: unknown = Reflect.get(processEnv, name);
  return typeof processValue === "string" ? processValue : fallback;
}

function getPosthogKey(): string {
  return readBuildVar("VITE_POSTHOG_KEY", "");
}

function getCaptureUrl(): string {
  return readBuildVar(
    "VITE_POSTHOG_CAPTURE_URL",
    "https://us.i.posthog.com/capture/",
  );
}

export function isConfigured(): boolean {
  const key = getPosthogKey();
  return key.startsWith("phc_") && key.length > 10;
}

export function recordFirstInstall(
  storage: StorageLike,
  log: EventLogger,
): "recorded" | "already-recorded" {
  if (storage.getItem(INSTALL_MARKER)) return "already-recorded";

  log("first_install");
  storage.setItem(INSTALL_MARKER, "1");
  return "recorded";
}

function getDistinctId(storage: StorageLike): string {
  const existing = storage.getItem(DISTINCT_ID_KEY);
  if (existing) return existing;
  const fresh =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `anon-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
  storage.setItem(DISTINCT_ID_KEY, fresh);
  return fresh;
}

type FetchLike = (
  input: string,
  init?: Record<string, unknown>,
) => Promise<unknown>;

// Exported for tests: sends one event to PostHog. Silently skips when no
// valid project key is configured. The fetch implementation is injectable so
// tests never hit the real network.
export async function captureEvent(
  event: AnalyticsEvent,
  distinctId: string,
  fetchImpl: FetchLike = fetch,
): Promise<void> {
  if (event !== "first_install" && event !== "app_opened") return;
  const key = getPosthogKey();
  if (!isConfigured()) return;
  await fetchImpl(getCaptureUrl(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      api_key: key,
      event,
      distinct_id: distinctId,
      properties: {
        app_platform: "tauri",
        $lib: "toolbox-tauri",
        $geoip_disable: true,
      },
    }),
    keepalive: true,
  });
}

export function isAnalyticsOptedOut(storage: StorageLike): boolean {
  return storage.getItem(OPT_OUT_KEY) === "1";
}

export function setAnalyticsOptOut(storage: StorageLike, optedOut: boolean): void {
  storage.setItem(OPT_OUT_KEY, optedOut ? "1" : "0");
}

export interface AnalyticsDeps {
  storage: StorageLike;
  isDev: boolean;
  fetchImpl?: FetchLike;
}

// Exported for tests: the testable core of initializeInstallAnalytics.
export async function initializeInstallAnalyticsWith(
  deps: AnalyticsDeps,
): Promise<void> {
  if (deps.isDev) return;

  try {
    const storage = deps.storage;
    if (isAnalyticsOptedOut(storage)) return;
    // Don't burn the first_install marker when no project key is configured:
    // a later correctly-configured build must still be able to count this install.
    if (!isConfigured()) return;

    const distinctId = getDistinctId(storage);
    recordFirstInstall(storage, (name) => {
      // Fire-and-forget: analytics must never break the app.
      captureEvent(name, distinctId, deps.fetchImpl).catch(() => {});
    });
    await captureEvent(
      "app_opened",
      distinctId,
      deps.fetchImpl,
    );
  } catch (error) {
    console.warn("Toolbox analytics unavailable", error);
  }
}

export async function initializeInstallAnalytics(): Promise<void> {
  await initializeInstallAnalyticsWith({
    storage: window.localStorage,
    isDev: import.meta.env.DEV,
  });
}
