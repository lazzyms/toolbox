import { createContext, useContext } from "react";
import type { OutputLocation, OutputSettings } from "./contracts";

export const OutputLocationContext = createContext<OutputLocation | null>(null);

export const useOutputLocation = (): OutputLocation => {
  const outputLocation = useContext(OutputLocationContext);
  if (outputLocation === null) throw new Error("useOutputLocation must be used inside the shell's OutputLocationContext provider.");
  return outputLocation;
};

export const outputLocationFor = (settings: OutputSettings): OutputLocation =>
  settings.mode === "customFolder" ? { customFolder: settings.folder } : "alongsideInput";

export interface OutputSettingsStore {
  settings: OutputSettings;
  save: (settings: OutputSettings) => Promise<void>;
}

export const OutputSettingsContext = createContext<OutputSettingsStore | null>(null);

export const useOutputSettings = (): OutputSettingsStore => {
  const store = useContext(OutputSettingsContext);
  if (store === null) throw new Error("useOutputSettings must be used inside the shell's OutputSettingsContext provider.");
  return store;
};
