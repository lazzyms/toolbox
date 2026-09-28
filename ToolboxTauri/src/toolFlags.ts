import { getToolFlagRequestCredentials } from "./analytics";
import { UtilityRegistry } from "./registry";
import type { AtomicToolId } from "./contracts";

export type ToolPlatform = "macos" | "windows" | "unsupported";

export type ToolAvailabilitySnapshot = Readonly<{
  platform: ToolPlatform;
  enabledByTool: Readonly<Record<AtomicToolId, boolean>>;
}>;

const toolIds = UtilityRegistry.map(({ id }) => id);
const evaluationTimeoutMs = 900;
const snapshotStorageKey = (platform: "macos" | "windows") =>
  `tool-availability-${platform}`;
let launchAvailability: Promise<ToolAvailabilitySnapshot> | null = null;

function enabledSnapshot(platform: ToolPlatform): ToolAvailabilitySnapshot {
  const enabledByTool = Object.fromEntries(
    toolIds.map((id) => [id, true]),
  ) as Record<AtomicToolId, boolean>;
  return Object.freeze({
    platform,
    enabledByTool: Object.freeze(enabledByTool),
  });
}

function parseFeatureFlags(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  if (Reflect.get(value, "errorsWhileComputingFlags")) return null;
  const featureFlags = Reflect.get(value, "featureFlags");
  if (
    typeof featureFlags !== "object" ||
    featureFlags === null ||
    Array.isArray(featureFlags)
  ) {
    return null;
  }
  return featureFlags as Record<string, unknown>;
}

function readCachedSnapshot(
  storage: Storage,
  platform: "macos" | "windows",
): ToolAvailabilitySnapshot | null {
  try {
    const raw = storage.getItem(snapshotStorageKey(platform));
    if (!raw) return null;
    const value: unknown = JSON.parse(raw);
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return null;
    }
    if (Reflect.get(value, "platform") !== platform) return null;
    const enabledByTool = Reflect.get(value, "enabledByTool");
    if (
      typeof enabledByTool !== "object" ||
      enabledByTool === null ||
      Array.isArray(enabledByTool) ||
      !toolIds.every((id) => typeof Reflect.get(enabledByTool, id) === "boolean")
    ) {
      return null;
    }
    return Object.freeze({
      platform,
      enabledByTool: Object.freeze(
        Object.fromEntries(
          toolIds.map((id) => [id, Reflect.get(enabledByTool, id)]),
        ) as Record<AtomicToolId, boolean>,
      ),
    });
  } catch {
    return null;
  }
}

function persistSnapshot(
  storage: Storage,
  snapshot: ToolAvailabilitySnapshot,
): void {
  if (snapshot.platform === "unsupported") return;
  try {
    storage.setItem(snapshotStorageKey(snapshot.platform), JSON.stringify(snapshot));
  } catch {
    return;
  }
}

async function requestSnapshot(
  platform: ToolPlatform,
  fallback: ToolAvailabilitySnapshot,
  storage: Storage,
  credentials: { apiKey: string; distinctId: string; endpoint: string },
): Promise<ToolAvailabilitySnapshot> {
  const controller = new AbortController();
  let timeout: number | undefined;
  try {
    return await new Promise<ToolAvailabilitySnapshot>((resolve) => {
      timeout = window.setTimeout(() => {
        controller.abort();
        resolve(fallback);
      }, evaluationTimeoutMs);
      void fetch(credentials.endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          api_key: credentials.apiKey,
          distinct_id: credentials.distinctId,
          geoip_disable: true,
        }),
        cache: "no-store",
        signal: controller.signal,
      })
        .then(async (response) => {
          if (!response.ok) return fallback;
          const featureFlags = parseFeatureFlags(await response.json());
          if (!featureFlags) return fallback;
          const enabledByTool = Object.fromEntries(
            toolIds.map((id) => [
              id,
              featureFlags[`tool-${id}-${platform}`] === true,
            ]),
          ) as Record<AtomicToolId, boolean>;
          const snapshot = Object.freeze({
            platform,
            enabledByTool: Object.freeze(enabledByTool),
          });
          persistSnapshot(storage, snapshot);
          return snapshot;
        })
        .then(resolve)
        .catch(() => resolve(fallback))
        .finally(() => {
          if (timeout !== undefined) window.clearTimeout(timeout);
        });
    });
  } catch {
    return fallback;
  } finally {
    if (timeout !== undefined) window.clearTimeout(timeout);
  }
}

export async function loadToolAvailability(input: {
  platform: unknown;
  isDev: boolean;
}): Promise<ToolAvailabilitySnapshot> {
  if (launchAvailability) return launchAvailability;
  const platform: ToolPlatform = input.platform === "macos" || input.platform === "windows"
    ? input.platform
    : "unsupported";
  if (input.isDev || platform === "unsupported") {
    return enabledSnapshot(platform);
  }
  launchAvailability = (async () => {
    const fallback = enabledSnapshot(platform);
    try {
      const storage = window.localStorage;
      const cached = readCachedSnapshot(storage, platform) ?? fallback;
      const credentials = getToolFlagRequestCredentials(storage);
      if (!credentials) return cached;
      return await requestSnapshot(platform, cached, storage, credentials);
    } catch {
      return fallback;
    }
  })();
  return launchAvailability;
}
