import { useEffect, useState } from "react";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";
import { ToolAvailabilityProvider } from "./ToolAvailabilityContext";
import {
  createToolAvailabilityStartup,
  type ToolAvailabilitySnapshot,
} from "./toolFlags";

const toolAvailabilityStartup = {
  platform: import.meta.env.VITE_TOOLBOX_TARGET_PLATFORM,
  isDev: import.meta.env.DEV,
};

export default function App() {
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

  if (!availability) return <div role="status" className="app-launch-loading">Loading Toolbox…</div>;

  return (
    <ToolAvailabilityProvider snapshot={availability}>
      <MainPage />
      <UtilityIndex />
    </ToolAvailabilityProvider>
  );
}
