import { createContext, useContext } from "react";
import type { OutputLocation } from "./contracts";

export const OutputLocationContext = createContext<OutputLocation | null>(null);

export const useOutputLocation = (): OutputLocation => {
  const outputLocation = useContext(OutputLocationContext);
  if (outputLocation === null) throw new Error("useOutputLocation must be used inside the shell's OutputLocationContext provider.");
  return outputLocation;
};
