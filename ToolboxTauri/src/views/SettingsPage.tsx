import { useEffect, useRef, useState, type ComponentType } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Check, Eye, Folder, Info, List, LoaderCircle, Search, Shield, type LucideIcon } from "lucide-react";
import packageInfo from "../../package.json";
import qrCode from "../assets/BuyMeACoffeeQR.png";
import appIcon from "../../src-tauri/icons/128x128.png";
import {
  isAnalyticsOptedOut,
  setAnalyticsOptOut,
} from "../analytics";
import { useOutputSettings } from "../outputLocation";
import { shortenHomePath, useHomeDir } from "../homePath";
import type { OutputMode } from "../contracts";
import { ShortcutHint } from "../components/ShortcutHint";
import { isWindows, shortcutAria, shortcutLabel, shortcuts } from "../shortcuts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const AppearanceSection = () => {
  const [theme, setTheme] = useState<"dark" | "light">(
    () => (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark",
  );
  const chooseTheme = (next: "dark" | "light") => {
    setTheme(next);
    document.body.dataset.theme = next;
    localStorage.setItem("toolbox-theme", next);
  };
  return (
    <div className="settings-rows">
      <div className="settings-row">
        <span className="settings-row-text">
          <span id="theme-title" className="settings-row-title">Theme</span>
          <span className="settings-row-hint">Choose how Toolbox looks on this device.</span>
        </span>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          className="settings-row-control"
          aria-label="Theme"
          value={theme}
          onValueChange={(value) => {
            if (value) chooseTheme(value as "dark" | "light");
          }}
        >
          <ToggleGroupItem value="light">Light</ToggleGroupItem>
          <ToggleGroupItem value="dark">Dark</ToggleGroupItem>
        </ToggleGroup>
      </div>
    </div>
  );
};

const OutputLocationSection = () => {
  const { settings, save } = useOutputSettings();
  const home = useHomeDir();
  const [saveError, setSaveError] = useState<string | null>(null);
  const persist = async (mode: OutputMode, folder: string) => {
    try {
      await save({ mode, folder });
      setSaveError(null);
    } catch (error) {
      setSaveError(String(error));
    }
  };
  const chooseFolder = async () => {
    const picked = await open({ directory: true, multiple: false, defaultPath: settings.folder || undefined });
    if (typeof picked === "string") await persist("customFolder", picked);
  };
  const chooseMode = async (mode: OutputMode) => {
    if (mode === "customFolder" && !settings.folder) return chooseFolder();
    await persist(mode, settings.folder);
  };
  return (
    <>
      <div className="settings-rows">
        <div className="settings-row">
          <span className="settings-row-text">
            <span id="output-mode-title" className="settings-row-title">Save results</span>
            <span className="settings-row-hint">Where new files go when a tool makes one file for each original.</span>
          </span>
          <RadioGroup
            aria-labelledby="output-mode-title"
            className="settings-row-control gap-1.5"
            value={settings.mode}
            onValueChange={(value) => void chooseMode(value as OutputMode)}
          >
            <span className="settings-radio">
              <RadioGroupItem id="output-mode-alongside" value="alongsideInput" data-settings-focus={settings.mode === "alongsideInput" ? "" : undefined} />
              <Label htmlFor="output-mode-alongside">Next to each original</Label>
            </span>
            <span className="settings-radio">
              <RadioGroupItem id="output-mode-folder" value="customFolder" data-settings-focus={settings.mode === "customFolder" ? "" : undefined} />
              <Label htmlFor="output-mode-folder">Always in one folder</Label>
            </span>
          </RadioGroup>
        </div>
        <div className="settings-row">
          <span className="settings-row-text">
            <span className="settings-row-title">Folder</span>
            <span className="settings-row-hint">Used when Always in one folder is chosen.</span>
          </span>
          <span className="settings-row-control settings-folder">
            <Folder aria-hidden="true" />
            <span className="settings-folder-path" title={settings.folder}>
              {settings.folder ? shortenHomePath(settings.folder, home) : "No folder chosen"}
            </span>
            <Button variant="outline" size="xs" aria-label="Choose output folder" onClick={() => void chooseFolder()}>
              Choose…
            </Button>
          </span>
        </div>
        <div className="settings-row">
          <span className="settings-row-text">
            <span className="settings-row-title">When a file with that name exists</span>
            <span className="settings-row-hint">Toolbox never replaces files. New files get -1, -2 and so on, like report-compressed-1.pdf.</span>
          </span>
          <span className="settings-row-control settings-row-hint">Always on</span>
        </div>
      </div>
      {saveError && <p className="settings-update-status" role="alert">{saveError}</p>}
    </>
  );
};

const PrivacySection = () => {
  const [optedOut, setOptedOut] = useState<boolean>(() => isAnalyticsOptedOut(window.localStorage));
  const share = (enabled: boolean) => {
    setOptedOut(!enabled);
    setAnalyticsOptOut(window.localStorage, !enabled);
  };
  return (
    <>
      <Card className="settings-note py-0">
        <CardContent className="settings-note-content">
          <Shield aria-hidden="true" />
          <p><strong>Your files never leave this device.</strong> Every tool runs on this computer.</p>
        </CardContent>
      </Card>
      <div className="settings-rows">
        <div className="settings-row">
          <span className="settings-row-text">
            <Label htmlFor="analytics-share" className="settings-row-title">Share anonymous usage counts</Label>
            <span className="settings-row-hint">Toolbox counts installs and app opens with PostHog to gauge usage. No personal data is collected.</span>
          </span>
          <Switch id="analytics-share" className="settings-row-control" checked={!optedOut} onCheckedChange={share} />
        </div>
      </div>
    </>
  );
};

type UpdateState = "idle" | "checking" | "available" | "installing" | "current" | "failed";

const updateStatus = (state: UpdateState, version: string | null) => {
  const installed = `Version ${packageInfo.version}.`;
  switch (state) {
    case "idle": return { title: "Check for updates", detail: installed };
    case "checking": return { title: "Checking for updates", detail: installed };
    case "current": return { title: "Toolbox is up to date", detail: installed };
    case "available": return { title: `Version ${version} is available`, detail: "Check again when you want to install it." };
    case "installing": return { title: `Installing version ${version}`, detail: "Toolbox restarts when it finishes." };
    case "failed": return { title: "Could not check for updates", detail: installed };
  }
};

const UpdatesSection = () => {
  const [state, setState] = useState<UpdateState>("idle");
  const [version, setVersion] = useState<string | null>(null);
  const checkForUpdates = async () => {
    setState("checking");
    try {
      const update = await check();
      if (!update) {
        setVersion(null);
        setState("current");
        return;
      }
      setVersion(update.version);
      setState("available");
      const approved = await confirm(
        `Toolbox ${update.version} is available. Download and install it now? The app will restart.`,
        { title: "Update available", kind: "info" },
      );
      if (!approved) return;
      setState("installing");
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setState("failed");
    }
  };
  const status = updateStatus(state, version);
  const busy = state === "checking" || state === "installing";
  return (
    <Card className="settings-update py-0">
      <CardContent className="settings-update-content">
        {state === "current" && <Check aria-hidden="true" />}
        <span className="settings-row-text" role="status" aria-live="polite">
          <span className="settings-update-title">{status.title}</span>
          <span className="settings-row-hint">{status.detail}</span>
        </span>
        <Button variant="outline" size="xs" onClick={() => void checkForUpdates()} disabled={busy}>
          {state === "checking" ? "Checking…" : state === "installing" ? "Installing…" : "Check now"}
        </Button>
      </CardContent>
    </Card>
  );
};

const AboutSection = () => (
  <>
    <div className="settings-about">
      <img src={appIcon} width="56" height="56" alt="" />
      <span className="settings-row-text">
        <strong className="settings-about-name">Toolbox</strong>
        <span className="settings-row-hint">Version {packageInfo.version} for {isWindows() ? "Windows" : "macOS"}</span>
      </span>
    </div>
    <Card className="settings-support py-0">
      <CardContent className="settings-support-content">
        <img src={qrCode} width="112" height="112" alt="Buy Me a Coffee donation QR code" />
        <span className="settings-row-text">
          <strong className="settings-update-title">Support Toolbox</strong>
          <span className="settings-row-hint">Toolbox is free. If it saves you time, scan the code with your phone.</span>
        </span>
      </CardContent>
    </Card>
  </>
);

const shortcutGroups = ["Everywhere", "Batch tools"] as const;

const KeyboardShortcutsSection = () => {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const matching = shortcuts.filter((item) => item.action.toLowerCase().includes(needle));
  return (
    <>
      <div className="settings-shortcut-search">
        <Label htmlFor="settings-shortcut-search">Search shortcuts</Label>
        <Input
          id="settings-shortcut-search"
          type="search"
          placeholder="Type an action, like undo"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      {shortcutGroups.map((group) => {
        const rows = matching.filter((item) => item.group === group);
        return rows.length > 0 && (
          <section key={group} className="settings-shortcut-group" aria-label={group}>
            <h2>{group}</h2>
            <dl>
              {rows.map((item) => (
                <div key={item.id} className="settings-shortcut-row">
                  <dt>{item.action}</dt>
                  <dd><kbd className="shortcut-hint">{shortcutLabel(item.id)}</kbd></dd>
                </div>
              ))}
            </dl>
          </section>
        );
      })}
      {matching.length === 0 && <p className="settings-row-hint" role="status">No shortcuts match.</p>}
      <p className="settings-row-hint">
        Every button can be reached with Tab. Arrow keys move inside toolbars, lists and page grids. Shortcuts cannot be changed yet.
      </p>
    </>
  );
};

const sections = [
  { id: "appearance", title: "Appearance", Icon: Eye, Content: AppearanceSection },
  { id: "output-location", title: "Output location", Icon: Folder, Content: OutputLocationSection },
  { id: "privacy", title: "Privacy", Icon: Shield, Content: PrivacySection },
  { id: "updates", title: "Updates", Icon: LoaderCircle, Content: UpdatesSection },
  { id: "about-and-support", title: "About and support", Icon: Info, Content: AboutSection },
  { id: "keyboard-shortcuts", title: "Keyboard shortcuts", Icon: List, Content: KeyboardShortcutsSection, hint: "Shown for the computer you are on." },
] as const satisfies ReadonlyArray<{ id: string; title: string; Icon: LucideIcon; Content: ComponentType; hint?: string }>;

export type SettingsSection = (typeof sections)[number]["id"];

export const SettingsPage = ({
  section,
  onSectionChange,
  onClose,
  onFindTool,
}: {
  section: SettingsSection;
  onSectionChange: (section: SettingsSection) => void;
  onClose: () => void;
  onFindTool: () => void;
}) => {
  const pageRef = useRef<HTMLDivElement>(null);
  const active = sections.find((item) => item.id === section) ?? sections[0];
  useEffect(() => {
    const page = pageRef.current;
    const target = page?.querySelector<HTMLElement>(".settings-content [data-settings-focus]")
      ?? page?.querySelector<HTMLElement>(".settings-nav [aria-pressed='true']");
    target?.focus();
  }, []);
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);
  const Content = active.Content;
  return (
    <div className="settings-page" ref={pageRef}>
      <aside className="settings-sidebar" aria-label="Settings">
        <Button variant="ghost" size="sm" className="settings-nav-link" onClick={onClose}>
          <ArrowLeft aria-hidden="true" />
          All tools
        </Button>
        <p className="settings-sidebar-label">Settings</p>
        <nav className="settings-nav" aria-label="Settings sections">
          {sections.map(({ id, title, Icon }) => (
            <Button
              key={id}
              variant={id === active.id ? "secondary" : "ghost"}
              size="sm"
              className="settings-nav-link"
              aria-pressed={id === active.id}
              onClick={() => onSectionChange(id)}
            >
              <Icon aria-hidden="true" />
              {title}
            </Button>
          ))}
        </nav>
        <p className="settings-sidebar-note">Files stay on this device.</p>
      </aside>
      <main className="settings-main">
        <header className="settings-page-header" data-tauri-drag-region>
          <h1 id="settings-section-title">{active.title}</h1>
          <p>{"hint" in active ? active.hint : "Changes save as you make them."}</p>
          <span className="topbar__drag-region" aria-hidden="true" data-tauri-drag-region />
          <Button variant="ghost" size="sm" aria-keyshortcuts={shortcutAria("find")} onClick={onFindTool}>
            <Search aria-hidden="true" />
            Find a tool
            <ShortcutHint id="find" />
          </Button>
        </header>
        <div className="settings-scroll">
          <section className="settings-content" aria-labelledby="settings-section-title">
            <Content />
          </section>
        </div>
      </main>
    </div>
  );
};
