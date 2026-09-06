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
    )
      return;
    updateStarted.current = true;

    const update = async () => {
      try {
        const available = await check();
        if (!available) return;

        const install = window.confirm(
          `Toolbox ${available.version} is available. Install it now?`,
        );
        if (!install) return;

        await available.downloadAndInstall();
        await relaunch();
      } catch (error) {
        console.warn("Toolbox update failed", error);
      }
    };

    const timer = window.setTimeout(update, 3000);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <>
      <MainPage />
      <UtilityIndex />
    </>
  );
}
