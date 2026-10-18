import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";
import { OutputLocationContext, OutputSettingsContext, outputLocationFor } from "./outputLocation";
import type { OutputSettings } from "./contracts";

const unreadableSettings: OutputSettings = { mode: "alongsideInput", folder: "" };

const parseOutputSettings = (value: unknown): OutputSettings => {
  const candidate = value as Partial<OutputSettings> | null;
  const valid = typeof candidate?.folder === "string" && (candidate.mode === "alongsideInput" || candidate.mode === "customFolder");
  return valid ? { mode: candidate.mode!, folder: candidate.folder! } : unreadableSettings;
};

export default function App() {
  const [outputSettings, setOutputSettings] = useState<OutputSettings | null>(null);

  useEffect(() => {
    void initializeInstallAnalytics();
  }, []);

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

  // Runs started before the stored setting arrives would silently use the wrong destination.
  if (!store || !outputLocation) return null;

  return (
    <OutputSettingsContext.Provider value={store}>
      <OutputLocationContext.Provider value={outputLocation}>
        <MainPage />
        <UtilityIndex />
      </OutputLocationContext.Provider>
    </OutputSettingsContext.Provider>
  );
}
