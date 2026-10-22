/**
 * Microsoft Store builds set VITE_ENABLE_UPDATES=false. They are compiled without the updater
 * because the Store delivers new versions, so Settings must not offer an in-app install.
 */
export const inAppUpdatesEnabled = (env: Record<string, string | undefined> = import.meta.env) =>
  env.VITE_ENABLE_UPDATES !== "false";
