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
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { DialogClose, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

export const SettingsPanel = () => {
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
    <DialogContent className="settings-dialog" showCloseButton={false}>
        <DialogHeader className="settings-header">
          <DialogTitle>Settings</DialogTitle>
          <DialogClose asChild>
            <Button variant="ghost" size="sm" aria-label="Close settings">
              Close
            </Button>
          </DialogClose>
          <DialogDescription className="sr-only">
            Manage Toolbox appearance, privacy, and updates.
          </DialogDescription>
        </DialogHeader>
        <div className="settings-content">
          <section aria-labelledby="appearance-title">
            <h3 id="appearance-title">Appearance</h3>
            <p>Choose how Toolbox looks on this device.</p>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              className="theme-switcher"
              aria-label="Theme"
              value={theme}
              onValueChange={(value) => {
                if (value) setTheme(value as "dark" | "light");
              }}
            >
              <ToggleGroupItem value="dark">Dark</ToggleGroupItem>
              <ToggleGroupItem value="light">Light</ToggleGroupItem>
            </ToggleGroup>
          </section>
          <section aria-labelledby="app-info-title">
            <h3 id="app-info-title">App info</h3>
            <Card className="settings-info py-0">
              <CardContent className="settings-info-content grid gap-2 py-4">
              <strong>Toolbox</strong>
              <span>Version {packageInfo.version}</span>
              <p>
                Private, local-first utilities for everyday PDF and image work.
              </p>
              </CardContent>
            </Card>
          </section>
          <section aria-labelledby="support-title">
            <h3 id="support-title">Support Toolbox</h3>
            <Card className="support-info py-0">
              <CardContent className="support-info-content flex items-center gap-4 py-4">
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
              </CardContent>
            </Card>
          </section>
          <section aria-labelledby="updates-title">
            <h3 id="updates-title">Updates</h3>
            <Button variant="default" size="sm" onClick={() => void checkForUpdates()} disabled={updateState === "checking" || updateState === "installing"}>
              {updateState === "checking" ? "Checking..." : updateState === "installing" ? "Installing..." : "Check for updates"}
            </Button>
            {updateState !== "idle" && (
              <p className="settings-update-status" role="status" aria-live="polite">
                {updateState === "current" && "Toolbox is up to date."}
                {updateState === "available" && updateVersion && `Toolbox ${updateVersion} is available. Check again when you want to install it.`}
                {updateState === "installing" && updateVersion && `Installing Toolbox ${updateVersion}...`}
                {updateState === "failed" && "Could not check for updates."}
              </p>
            )}
          </section>
          <section aria-labelledby="privacy-title">
            <h3 id="privacy-title">Privacy</h3>
            <p>
              Toolbox counts installs and app opens with PostHog to gauge
              usage. No personal data is collected.
            </p>
            <div className="settings-privacy-toggle">
              <Label htmlFor="analytics-opt-out">Don’t send anonymous usage counts</Label>
              <Switch
                id="analytics-opt-out"
                checked={analyticsOptOut}
                onCheckedChange={toggleAnalyticsOptOut}
              />
            </div>
          </section>
        </div>
    </DialogContent>
  );
};
