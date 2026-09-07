import { useEffect } from "react";
import { initializeInstallAnalytics } from "./analytics";
import { MainPage } from "./views/MainPage";
import { UtilityIndex } from "./components/UtilityIndex";

export default function App() {

  useEffect(() => {
    void initializeInstallAnalytics();
  }, []);

  return (
    <>
      <MainPage />
      <UtilityIndex />
    </>
  );
}
