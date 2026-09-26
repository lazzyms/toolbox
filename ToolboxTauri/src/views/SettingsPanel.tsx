import { useEffect, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { confirm } from "@tauri-apps/plugin-dialog";
import packageInfo from "../../package.json";
import qrCode from "../assets/BuyMeACoffeeQR.png";
import {
  isAnalyticsOptedOut,
  setAnalyticsOptOut,
} from "../analytics";

interface SettingsPanelProps {
  onClose: () => void;
}

export const SettingsPanel = ({ onClose }: SettingsPanelProps) => {
  const [theme, setTheme] = useState<"dark" | "light">(
    () => (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark",
  );
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "available" | "installing" | "current" | "failed">("idle");
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [analyticsOptOut, setAnalyticsOptOutState] = useState<boolean>(() =>
    isAnalyticsOptedOut(window.localStorage),
  );
  useEffect(() => {
    document.body.dataset.theme = theme;
    localStorage.setItem("toolbox-theme", theme);
  }, [theme]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);
  const checkForUpdates = async () => {
    setUpdateState("checking");
    try {
      const update = await check();
      if (!update) {
        setUpdateVersion(null);
        setUpdateState("current");
        return;
      }
      setUpdateVersion(update.version);
      setUpdateState("available");
      const approved = await confirm(
        `Toolbox ${update.version} is available. Download and install it now? The app will restart.`,
        { title: "Update available", kind: "info" },
      );
      if (!approved) return;
      setUpdateState("installing");
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setUpdateState("failed");
    }
  };
  const toggleAnalyticsOptOut = (optedOut: boolean) => {
    setAnalyticsOptOutState(optedOut);
    setAnalyticsOptOut(window.localStorage, optedOut);
  };
  return (
    <div className="settings-overlay" role="presentation">
      <section
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
      >
        <header className="settings-header">
          <h2 id="settings-title">Settings</h2>
          <button type="button" aria-label="Close settings" onClick={onClose}>
            Close
          </button>
        </header>
        <div className="settings-content">
          <section aria-labelledby="appearance-title">
            <h3 id="appearance-title">Appearance</h3>
            <p>Choose how Toolbox looks on this device.</p>
            <div className="theme-switcher" role="group" aria-label="Theme">
              <button
                type="button"
                aria-pressed={theme === "dark"}
                onClick={() => setTheme("dark")}
              >
                Dark
              </button>
              <button
                type="button"
                aria-pressed={theme === "light"}
                onClick={() => setTheme("light")}
              >
                Light
              </button>
            </div>
          </section>
          <section aria-labelledby="app-info-title">
            <h3 id="app-info-title">App info</h3>
            <div className="settings-info">
              <strong>Toolbox</strong>
              <span>Version {packageInfo.version}</span>
              <p>
                Private, local-first utilities for everyday PDF and image work.
              </p>
            </div>
          </section>
          <section aria-labelledby="support-title">
            <h3 id="support-title">Support Toolbox</h3>
            <div className="support-info">
              <img
                src={qrCode}
                width="112"
                height="112"
                alt="Buy Me a Coffee donation QR code"
              />
              <p>
                Enjoying Toolbox?
                <br />
                If you’d like to support development, scan the QR code.
              </p>
            </div>
          </section>
          <section aria-labelledby="updates-title">
            <h3 id="updates-title">Updates</h3>
            <button type="button" onClick={() => void checkForUpdates()} disabled={updateState === "checking" || updateState === "installing"}>
              {updateState === "checking" ? "Checking..." : updateState === "installing" ? "Installing..." : "Check for updates"}
            </button>
            <p role="status" aria-live="polite">
              {updateState === "current" && "Toolbox is up to date."}
              {updateState === "available" && updateVersion && `Toolbox ${updateVersion} is available. Check again when you want to install it.`}
              {updateState === "installing" && updateVersion && `Installing Toolbox ${updateVersion}...`}
              {updateState === "failed" && "Could not check for updates."}
            </p>
          </section>
          <section aria-labelledby="privacy-title">
            <h3 id="privacy-title">Privacy</h3>
            <p>
              Toolbox counts installs and app opens with PostHog to gauge
              usage. No personal data is collected.
            </p>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={analyticsOptOut}
                onChange={(e) => toggleAnalyticsOptOut(e.target.checked)}
              />
              Don’t send anonymous usage counts
            </label>
          </section>
        </div>
      </section>
    </div>
  );
};
