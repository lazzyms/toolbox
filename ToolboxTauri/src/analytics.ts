// Anonymous install analytics via PostHog (free tier: 1M events/month, no card).
//
// What is collected: `first_install` (once) and `app_opened` (per launch),
// each with `{ app_platform: "tauri" }`. Nothing else. The distinct_id is a
// random UUID generated on this device — it is not tied to any user, machine,
// or account, and no PII is ever sent.
//
// Setup: create a free PostHog Cloud project at https://posthog.com and paste
// its project API key below. EU-region projects must use
// "https://eu.i.posthog.com/capture/" as the capture URL.
// Until a real key is set, all events are silently skipped.

export const INSTALL_MARKER = "toolbox.analytics.install-recorded";
export const DISTINCT_ID_KEY = "toolbox.analytics.distinct-id";
export const OPT_OUT_KEY = "toolbox.analytics.opt-out";

type StorageLike = Pick<Storage, "getItem" | "setItem">;
type EventLogger = (name: string, parameters: Record<string, string>) => void;

const POSTHOG_API_KEY = "phc_REPLACE_WITH_YOUR_POSTHOG_PROJECT_KEY";
const POSTHOG_CAPTURE_URL = "https://us.i.posthog.com/capture/";

function isConfigured(): boolean {
  return (
    POSTHOG_API_KEY.startsWith("phc_") &&
    POSTHOG_API_KEY !== "phc_REPLACE_WITH_YOUR_POSTHOG_PROJECT_KEY"
  );
}

export function recordFirstInstall(
  storage: StorageLike,
  log: EventLogger,
): "recorded" | "already-recorded" {
  if (storage.getItem(INSTALL_MARKER)) return "already-recorded";

  log("first_install", { app_platform: "tauri" });
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

async function capture(
  event: string,
  distinctId: string,
  parameters: Record<string, string>,
): Promise<void> {
  if (!isConfigured()) return;
  await fetch(POSTHOG_CAPTURE_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      api_key: POSTHOG_API_KEY,
      event,
      distinct_id: distinctId,
      properties: { ...parameters, $lib: "toolbox-tauri" },
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

export async function initializeInstallAnalytics(): Promise<void> {
  if (import.meta.env.DEV) return;

  try {
    const storage = window.localStorage;
    if (isAnalyticsOptedOut(storage)) return;

    const distinctId = getDistinctId(storage);
    recordFirstInstall(storage, (name, parameters) => {
      void capture(name, distinctId, parameters);
    });
    await capture("app_opened", distinctId, { app_platform: "tauri" });
  } catch (error) {
    console.warn("Toolbox analytics unavailable", error);
  }
}
