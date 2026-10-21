import { useEffect } from "react";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";
import { OutputLocationContext } from "./outputLocation";
import type { OutputLocation } from "./contracts";

const outputLocation: OutputLocation = "alongsideInput";

export default function App() {

  useEffect(() => {
    void initializeInstallAnalytics();
  }, []);

  return (
    <OutputLocationContext.Provider value={outputLocation}>
      <MainPage />
      <UtilityIndex />
    </OutputLocationContext.Provider>
  );
}
