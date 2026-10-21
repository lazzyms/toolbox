import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";
import { SaveDialogHost } from "./components/SaveDialog";
import { OutputLocationContext, OutputSettingsContext, outputLocationFor } from "./outputLocation";
import { ToolAvailabilityProvider } from "./ToolAvailabilityContext";
import {
  createToolAvailabilityStartup,
  type ToolAvailabilitySnapshot,
} from "./toolFlags";
import type { OutputSettings } from "./contracts";

const unreadableSettings: OutputSettings = { mode: "alongsideInput", folder: "" };

const toolAvailabilityStartup = {
  platform: import.meta.env.VITE_TOOLBOX_TARGET_PLATFORM,
  isDev: import.meta.env.DEV,
};

const parseOutputSettings = (value: unknown): OutputSettings => {
  const candidate = value as Partial<OutputSettings> | null;
  const valid = typeof candidate?.folder === "string" && (candidate.mode === "alongsideInput" || candidate.mode === "customFolder");
  return valid ? { mode: candidate.mode!, folder: candidate.folder! } : unreadableSettings;
};

export default function App() {
  const [outputSettings, setOutputSettings] = useState<OutputSettings | null>(null);
  const [startup] = useState(() => createToolAvailabilityStartup(toolAvailabilityStartup));
  const [availability, setAvailability] = useState<ToolAvailabilitySnapshot | null>(() =>
    startup.state === "ready" ? startup.snapshot : null,
  );

  useEffect(() => {
    void initializeInstallAnalytics();
  }, []);

  useEffect(() => {
    if (startup.state === "pending") {
      void startup.resolve().then(setAvailability);
    }
  }, [startup]);

  useEffect(() => {
    invoke<unknown>("get_output_settings").then(
      (value) => setOutputSettings(parseOutputSettings(value)),
      () => setOutputSettings(unreadableSettings),
    );
  }, []);

  const save = useCallback(async (settings: OutputSettings) => {
    setOutputSettings(parseOutputSettings(await invoke<unknown>("set_output_settings", { settings })));
  }, []);

  const store = useMemo(() => outputSettings && { settings: outputSettings, save }, [outputSettings, save]);
  const outputLocation = useMemo(() => outputSettings && outputLocationFor(outputSettings), [outputSettings]);

  if (!availability) return <div role="status" className="app-launch-loading">Loading Toolbox…</div>;
  // Runs started before the stored setting arrives would silently use the wrong destination.
  if (!store || !outputLocation) return null;

  return (
    <ToolAvailabilityProvider snapshot={availability}>
      <OutputSettingsContext.Provider value={store}>
        <OutputLocationContext.Provider value={outputLocation}>
          <MainPage />
          <UtilityIndex />
          <SaveDialogHost />
        </OutputLocationContext.Provider>
      </OutputSettingsContext.Provider>
    </ToolAvailabilityProvider>
  );
}
