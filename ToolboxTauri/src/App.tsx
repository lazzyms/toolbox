import { useEffect, useState } from "react";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";
import { ToolAvailabilityProvider } from "./ToolAvailabilityContext";
import { loadToolAvailability, type ToolAvailabilitySnapshot } from "./toolFlags";

export default function App() {
  const [availability, setAvailability] = useState<ToolAvailabilitySnapshot | null>(null);

  useEffect(() => {
    void initializeInstallAnalytics();
    void loadToolAvailability({
      platform: import.meta.env.VITE_TOOLBOX_TARGET_PLATFORM,
      isDev: import.meta.env.DEV,
    })
      .then(setAvailability);
  }, []);

  if (!availability) return <div role="status" className="app-launch-loading">Loading Toolbox…</div>;

  return (
    <ToolAvailabilityProvider snapshot={availability}>
      <MainPage />
      <UtilityIndex />
    </ToolAvailabilityProvider>
  );
}
