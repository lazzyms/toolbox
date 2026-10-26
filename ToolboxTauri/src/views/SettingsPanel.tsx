import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import packageInfo from "../../package.json";
import qrCode from "../assets/BuyMeACoffeeQR.png";

interface QuickActionSetting {
  id: string;
  title: string;
  enabled: boolean;
}

interface SettingsPanelProps {
  onClose: () => void;
}

export const SettingsPanel = ({ onClose }: SettingsPanelProps) => {
  const [theme, setTheme] = useState<"dark" | "light">(
    () => (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark",
  );
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "available" | "current" | "failed">("idle");
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [quickActions, setQuickActions] = useState<QuickActionSetting[]>([]);
  const [quickActionError, setQuickActionError] = useState<string | null>(null);
  useEffect(() => {
    invoke<QuickActionSetting[]>("get_quick_actions").then(setQuickActions, () => setQuickActionError("Could not load quick actions."));
  }, []);
  const toggleQuickAction = async (id: string) => {
    const next = quickActions.map((action) => (action.id === id ? { ...action, enabled: !action.enabled } : action));
    setQuickActions(next);
    try {
      await invoke("set_quick_actions", { ids: next.filter((action) => action.enabled).map((action) => action.id) });
      setQuickActionError(null);
    } catch {
      setQuickActionError("Could not update quick actions.");
    }
  };
  useEffect(() => {
    document.body.dataset.theme = theme;
    localStorage.setItem("toolbox-theme", theme);
  }, [theme]);
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
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setUpdateState("failed");
    }
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
          <section aria-labelledby="quick-actions-title">
            <h3 id="quick-actions-title">Quick actions</h3>
            <p>Choose the tools that appear when you right-click a file in Finder or File Explorer.</p>
            <div className="quick-action-options" role="group" aria-label="Quick actions">
              {quickActions.map((action) => (
                <label key={action.id}>
                  <input type="checkbox" checked={action.enabled} onChange={() => void toggleQuickAction(action.id)} />
                  {action.title}
                </label>
              ))}
            </div>
            {quickActionError && <p role="alert">{quickActionError}</p>}
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
            <button type="button" onClick={() => void checkForUpdates()} disabled={updateState === "checking"}>
              {updateState === "checking" ? "Checking..." : "Check for updates"}
            </button>
            <p role="status" aria-live="polite">
              {updateState === "current" && "Toolbox is up to date."}
              {updateState === "available" && updateVersion && `Installing Toolbox ${updateVersion}...`}
              {updateState === "failed" && "Could not check for updates."}
            </p>
          </section>
        </div>
      </section>
    </div>
  );
};
