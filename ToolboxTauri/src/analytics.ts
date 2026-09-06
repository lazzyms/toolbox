import { getApp, getApps, initializeApp } from "firebase/app";
import {
  getAnalytics,
  isSupported,
  logEvent,
  type Analytics,
} from "firebase/analytics";

export const INSTALL_MARKER = "toolbox.analytics.install-recorded";

type StorageLike = Pick<Storage, "getItem" | "setItem">;
type EventLogger = (name: string, parameters: Record<string, string>) => void;

const firebaseConfig = {
  apiKey: "AIzaSyDeYyZEobmuRm-NfcyubVG_kbioyMMa4Hk",
  authDomain: "toolbox-2c916.firebaseapp.com",
  projectId: "toolbox-2c916",
  storageBucket: "toolbox-2c916.firebasestorage.app",
  messagingSenderId: "353688399550",
  appId: "1:353688399550:web:149c6f415d20afeea3b8ef",
  measurementId: "G-GN78CYTEG6",
};

export function recordFirstInstall(
  storage: StorageLike,
  log: EventLogger,
): "recorded" | "already-recorded" {
  if (storage.getItem(INSTALL_MARKER)) return "already-recorded";

  log("first_install", { app_platform: "tauri" });
  storage.setItem(INSTALL_MARKER, "1");
  return "recorded";
}

function firebaseAnalytics(): Analytics {
  const app = getApps().length > 0 ? getApp() : initializeApp(firebaseConfig);
  return getAnalytics(app);
}

export async function initializeInstallAnalytics(): Promise<void> {
  if (import.meta.env.DEV) return;

  try {
    if (!(await isSupported())) return;

    const analytics = firebaseAnalytics();
    recordFirstInstall(window.localStorage, (name, parameters) => {
      logEvent(analytics, name, parameters);
    });
    logEvent(analytics, "app_opened", { app_platform: "tauri" });
  } catch (error) {
    console.warn("Toolbox analytics unavailable", error);
  }
}
