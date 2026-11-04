type UpdateEnv = { DEV?: boolean; VITE_ENABLE_UPDATES?: string };

/**
 * Microsoft Store builds set VITE_ENABLE_UPDATES=false. They are compiled without the updater
 * because the Store delivers new versions, so Settings must not offer an in-app install.
 */
export const inAppUpdatesEnabled = (env: UpdateEnv = import.meta.env) =>
  env.VITE_ENABLE_UPDATES !== "false";

/** Development builds never check at launch: an install would replace the dev app with a release. */
export const startupUpdateCheckEnabled = (env: UpdateEnv = import.meta.env) =>
  !env.DEV && inAppUpdatesEnabled(env);

export type AvailableUpdate = { version: string; downloadAndInstall(): Promise<void> };

export type UpdateActions = {
  confirm(message: string, options: { title: string; kind: "info" }): Promise<boolean>;
  relaunch(): Promise<void>;
};

/** Asks before installing; returns false when the user declines. */
export const installIfApproved = async (
  update: AvailableUpdate,
  actions: UpdateActions,
  onInstalling?: () => void,
) => {
  const approved = await actions.confirm(
    `Toolbox ${update.version} is available. Download and install it now? The app will restart.`,
    { title: "Update available", kind: "info" },
  );
  if (!approved) return false;
  onInstalling?.();
  await update.downloadAndInstall();
  await actions.relaunch();
  return true;
};

export const STARTUP_UPDATE_DELAY_MS = 3000;

/**
 * Checks for an update shortly after launch and offers to install it. Failures stay quiet
 * because the user did not ask; Settings → Updates reports them on a manual check.
 * Returns a cancel function for effect cleanup.
 */
export const scheduleStartupUpdateCheck = (
  actions: UpdateActions & { check(): Promise<AvailableUpdate | null> },
  env: UpdateEnv = import.meta.env,
  delayMs = STARTUP_UPDATE_DELAY_MS,
) => {
  if (!startupUpdateCheckEnabled(env)) return () => undefined;
  const timer = setTimeout(async () => {
    try {
      const update = await actions.check();
      if (update) await installIfApproved(update, actions);
    } catch (error) {
      console.warn("Startup update check failed", error);
    }
  }, delayMs);
  return () => clearTimeout(timer);
};
