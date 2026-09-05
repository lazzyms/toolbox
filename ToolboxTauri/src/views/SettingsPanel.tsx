import { useEffect, useState } from "react";
import packageInfo from "../../package.json";
import qrCode from "../assets/BuyMeACoffeeQR.png";

interface SettingsPanelProps {
  onClose: () => void;
}

export const SettingsPanel = ({ onClose }: SettingsPanelProps) => {
  const [theme, setTheme] = useState<"dark" | "light">(
    () => (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark",
  );
  useEffect(() => {
    document.body.dataset.theme = theme;
    localStorage.setItem("toolbox-theme", theme);
  }, [theme]);
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
        </div>
      </section>
    </div>
  );
};
