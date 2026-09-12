import { useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";

export default function App() {
  const updateStarted = useRef(false);

  useEffect(() => {
    void initializeInstallAnalytics();
  }, []);

  useEffect(() => {
    if (
      import.meta.env.DEV ||
      import.meta.env.VITE_ENABLE_UPDATES !== "true" ||
      updateStarted.current
    ) {
      return;
    }
    updateStarted.current = true;

    const checkForUpdates = async () => {
      try {
        const update = await check();
        if (!update) return;

        const install = window.confirm(
          `Toolbox ${update.version} is available. Install it now?`,
        );
        if (!install) return;

        await update.downloadAndInstall();
        await relaunch();
      } catch (error) {
        console.warn("Toolbox update failed", error);
      }
    };

    const timer = window.setTimeout(() => void checkForUpdates(), 3000);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <>
      <MainPage />
      <UtilityIndex />
    </>
  );
}
