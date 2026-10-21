import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import shellFileTypes from "../../shared/shell-file-types.json" with { type: "json" };
import {
  ToolWorkspaceRegistry,
  UtilityRegistry,
  toolsForWorkspace,
  toolsForWorkspaceId,
  workspaceForTool,
} from "../registry";
import type { AtomicToolId, ToolDefinition, WorkspaceId } from "../contracts";
import { TablerIcon } from "../components/TablerIcon";
import { RunStateContext, type WorkspaceSourceAction } from "../components/ToolScaffold";
import { ShortcutHint } from "../components/ShortcutHint";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { matchShortcut, pressShortcutTarget, shortcutAria, shortcutLabel, shortcutTarget } from "../shortcuts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import type { PdfEditorNavigation } from "./PDFEditorWorkspaceView";
import { SecurityWorkspaceView } from "./SecurityWorkspaceView";
import { PDFEditorWorkspaceView } from "./PDFEditorWorkspaceView";
import { PDFConversionWorkspaceView } from "./PDFConversionWorkspaceView";
import { ImageEditorWorkspaceView } from "./ImageEditorWorkspaceView";
import { CompressFilesWorkspaceView } from "./CompressFilesWorkspaceView";
import { MediaWorkspaceView } from "./MediaWorkspaceView";
import { PlannedToolView, UnavailableToolView } from "./PlannedToolView";
import { SettingsPage, type SettingsSection } from "./SettingsPage";
import { favoritesStorageKey, recentStorageKey } from "../savedToolIds";
import { useShellBridge } from "../hooks/useShellBridge";
import { TOOL_DROP_EVENT, type ShellCommand, type ShellEvent } from "../hooks/useShellBridge";
import { useToolAvailability } from "../ToolAvailabilityContext";

const workspaceViews = {
  "file-security": SecurityWorkspaceView,
  "pdf-editor": PDFEditorWorkspaceView,
  "pdf-convert": PDFConversionWorkspaceView,
  "image-editor": ImageEditorWorkspaceView,
  "compress-files": CompressFilesWorkspaceView,
  "media-tools": MediaWorkspaceView,
} satisfies Record<WorkspaceId, ComponentType<{ utility: ToolDefinition }>>;

const visibleToolsPerView = 6;

type LibraryFilter = "all" | "favorites";

const firstOpenableTool = (tools: readonly ToolDefinition[]) =>
  tools.find((tool) => tool.capability.nativeAvailability !== "unavailable") ?? tools[0];

export const MainPage = () => {
  const availability = useToolAvailability();
  const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);
  const [initialPaths, setInitialPaths] = useState<readonly string[]>([]);
  const workspaceHeadingRef = useRef<HTMLHeadingElement>(null);
  const focusWorkspaceHeadingOnNavigation = useRef(false);
  const [fileActivation, setFileActivation] = useState<Extract<ShellEvent, { kind: "files" }> | null>(null);
  const [rejectedFiles, setRejectedFiles] = useState<Extract<ShellEvent, { kind: "rejected-files" }> | null>(null);
  const [activeDocumentPath, setActiveDocumentPath] = useState<string | null>(null);
  const [viewZoom, setViewZoom] = useState(() => Number(localStorage.getItem("toolbox-view-zoom") || "1"));
  const activationInFlight = useRef<string | null>(null);
  const queuedActivations = useRef<Extract<ShellEvent, { kind: "files" }>[]>([]);
  const acceptedActivationIds = useRef<string[]>([]);
  const [workspaceSourceAction, setWorkspaceSourceAction] = useState<WorkspaceSourceAction | null>(null);
  const publishWorkspaceSourceAction = useCallback((action: WorkspaceSourceAction | null) => {
    setWorkspaceSourceAction(() => action);
  }, []);
  const [running, setRunning] = useState(false);
  const [settingsSection, setSettingsSection] = useState<SettingsSection | null>(null);
  const [focusAfterSettings, setFocusAfterSettings] = useState<"settings" | "search" | null>(null);
  const settingsTriggerRef = useRef<HTMLButtonElement>(null);
  const [filter, setFilter] = useState<LibraryFilter>("all");
  const [search, setSearch] = useState("");
  const [expandedViews, setExpandedViews] = useState<WorkspaceId[]>([]);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const commandCenterRef = useRef<HTMLElement>(null);
  const [favorites, setFavorites] = useState<string[]>(() =>
    JSON.parse(localStorage.getItem(favoritesStorageKey) || "[]"),
  );
  const favoriteTools = useMemo(
    () => favorites
      .map((id) => UtilityRegistry.find((tool) => tool.id === id))
      .filter((tool): tool is ToolDefinition => Boolean(tool && availability.allows(tool.id))),
    [favorites, availability],
  );
  const openableWorkspaces = useMemo(
    () => ToolWorkspaceRegistry
      .map((workspace) => ({ workspace, tools: availability.filter(toolsForWorkspace(workspace)) }))
      .filter(({ tools }) => tools.length > 0),
    [availability],
  );
  const normalizedSearch = search.trim().toLowerCase();
  const visibleWorkspaces = useMemo(
    () =>
      ToolWorkspaceRegistry.map((workspace) => {
        const actions = availability.filter(toolsForWorkspace(workspace)).filter((tool) => {
          const matchesScope = filter === "all" || favorites.includes(tool.id);
          const searchText = `${tool.title} ${tool.blurb} ${tool.keywords?.join(" ") ?? ""} ${workspace.title} ${workspace.blurb}`.toLowerCase();
          return matchesScope && searchText.includes(normalizedSearch);
        });
        return { workspace, actions };
      }).filter(({ actions }) => actions.length > 0),
    [filter, normalizedSearch, favorites, availability],
  );
  const openTool = useCallback((tool: ToolDefinition, paths: readonly string[] = []) => {
    if (!availability.allows(tool.id)) return;
    setFileActivation(null);
    activationInFlight.current = null;
    queuedActivations.current = [];
    setActiveDocumentPath(paths[0] ?? null);
    setWorkspaceSourceAction(null);
    setInitialPaths([...paths]);
    setSelectedTool(tool);
    const recent: string[] = JSON.parse(localStorage.getItem(recentStorageKey) || "[]");
    localStorage.setItem(recentStorageKey, JSON.stringify([tool.id, ...recent.filter((id) => id !== tool.id)].slice(0, 8)));
  }, [availability]);
  const goHome = useCallback(() => {
    setWorkspaceSourceAction(null);
    setSelectedTool(null);
    setInitialPaths([]);
    setFileActivation(null);
    setActiveDocumentPath(null);
    activationInFlight.current = null;
    queuedActivations.current = [];
  }, []);
  const navigateToPdfUtility: PdfEditorNavigation = useCallback(({ utilityId, initialPaths: paths }) => {
    const tool = UtilityRegistry.find((item) => item.id === utilityId);
    if (tool && availability.allows(tool.id)) {
      focusWorkspaceHeadingOnNavigation.current = true;
      openTool(tool, paths);
    }
  }, [availability, openTool]);
  const toggleFavorite = (id: string) =>
    setFavorites((current) => {
      const next = current.includes(id)
        ? current.filter((item) => item !== id)
        : [...current, id];
      localStorage.setItem(favoritesStorageKey, JSON.stringify(next));
      return next;
    });
  const commandResults = () =>
    Array.from(
      commandCenterRef.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-command-result]",
      ) ?? [],
    ).filter((button) => !button.disabled);
  const handleSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || event.key !== "ArrowDown") return;

    const firstResult = commandResults()[0];
    if (!firstResult) return;

    event.preventDefault();
    firstResult.focus();
  };
  const handleCommandResultKeyDown = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
  ) => {
    const direction = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (direction === 0) return;

    const results = commandResults();
    const currentIndex = results.indexOf(event.currentTarget);
    if (currentIndex < 0) return;

    event.preventDefault();
    const nextResult = results[currentIndex + direction];
    if (nextResult) nextResult.focus();
    else if (direction < 0) searchInputRef.current?.focus();
  };
  const selectLibrary = (nextFilter: LibraryFilter) => {
    setFilter(nextFilter);
    setSearch("");
    goHome();
  };
  const selectedWorkspace = selectedTool
    ? workspaceForTool(selectedTool.id)
    : undefined;
  const railWorkspace = selectedTool && selectedWorkspace && selectedWorkspace.id !== "pdf-editor"
    ? selectedWorkspace
    : null;
  const openFilesFromMenu = useCallback(async () => {
    const extensions = [...new Set([...shellFileTypes["pdf-editor"], ...shellFileTypes["image-editor"]])];
    const picked = await open({ multiple: false, filters: [{ name: "Supported documents", extensions }] });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (paths.length) await invoke("open_paths", { paths });
  }, []);
  const dispatchEditorCommand = useCallback((command: Exclude<ShellCommand, "search" | "open" | "zoom-in" | "zoom-out" | "zoom-reset" | "toggle-theme" | "shortcuts">) => {
    window.dispatchEvent(new CustomEvent("toolbox:editor-command", { detail: command }));
  }, []);
  const activateFile = useCallback((event: Extract<ShellEvent, { kind: "files" }>) => {
    const preferredId: AtomicToolId = event.workspace === "pdf-editor" ? "pdf-edit" : "heic-convert";
    const actions = toolsForWorkspaceId(event.workspace);
    const tool = actions.find((item) => item.id === preferredId && availability.allows(item.id))
      ?? actions.find((item) => availability.allows(item.id));
    if (!tool) {
      setRejectedFiles({ kind: "rejected-files", paths: event.paths, reason: `No enabled tools are available for ${event.workspace === "pdf-editor" ? "PDF" : "image"} files.` });
      return false;
    }
    setRejectedFiles(null);
    setInitialPaths([]);
    setFileActivation(event);
    setActiveDocumentPath(event.paths[0] ?? null);
    setSelectedTool(tool);
    setWorkspaceSourceAction(null);
    const recent: string[] = JSON.parse(localStorage.getItem(recentStorageKey) || "[]");
    localStorage.setItem(recentStorageKey, JSON.stringify([tool.id, ...recent.filter((item) => item !== tool.id)].slice(0, 8)));
    return true;
  }, [availability]);
  const startNextActivation = useCallback((event: Extract<ShellEvent, { kind: "files" }>) => {
    let next: Extract<ShellEvent, { kind: "files" }> | null = event;
    while (next) {
      if (activateFile(next)) {
        activationInFlight.current = next.activationId;
        return;
      }
      acceptedActivationIds.current.push(next.activationId);
      if (acceptedActivationIds.current.length > 64) acceptedActivationIds.current.shift();
      next = queuedActivations.current.shift() ?? null;
    }
    activationInFlight.current = null;
    setFileActivation(null);
  }, [activateFile]);
  const handleShellEvent = useCallback((event: ShellEvent) => {
    if (event.kind === "files") {
      if (acceptedActivationIds.current.includes(event.activationId)
        || activationInFlight.current === event.activationId
        || queuedActivations.current.some((queued) => queued.activationId === event.activationId)) return;
      if (activationInFlight.current) {
        queuedActivations.current.push(event);
        return;
      }
      startNextActivation(event);
      return;
    }
    if (event.kind === "rejected-files") {
      setRejectedFiles(event);
      return;
    }
    if (event.kind === "dropped-files") {
      setRejectedFiles(null);
      if (!selectedTool) {
        void invoke("open_paths", { paths: event.paths });
      } else {
        window.dispatchEvent(new CustomEvent(TOOL_DROP_EVENT, { detail: event.paths }));
      }
      return;
    }
    switch (event.command) {
      case "search":
        setSettingsSection(null);
        if (selectedTool) goHome();
        setFocusAfterSettings("search");
        break;
      case "open":
        // With a tool open, Open adds files to it. Otherwise it opens a document in an editor.
        if (selectedTool && !settingsSection && commandCenterRef.current?.querySelector('[data-shortcut="open"]:not(:disabled)')) {
          pressShortcutTarget(commandCenterRef.current, "open");
        } else {
          void openFilesFromMenu();
        }
        break;
      case "undo":
      case "redo":
      case "export":
        dispatchEditorCommand(event.command);
        break;
      case "zoom-in":
      case "zoom-out":
      case "zoom-reset":
        setViewZoom((current) => event.command === "zoom-reset" ? 1 : Math.min(1.5, Math.max(0.75, current + (event.command === "zoom-in" ? 0.1 : -0.1))));
        break;
      case "toggle-theme": {
        const theme = document.body.dataset.theme === "light" ? "dark" : "light";
        document.body.dataset.theme = theme;
        localStorage.setItem("toolbox-theme", theme);
        break;
      }
      case "shortcuts":
        setSettingsSection("keyboard-shortcuts");
        break;
    }
  }, [dispatchEditorCommand, goHome, openFilesFromMenu, selectedTool, settingsSection, startNextActivation]);
  const acceptActivation = useCallback((activationId: string) => {
    if (activationInFlight.current !== activationId) return;
    acceptedActivationIds.current.push(activationId);
    if (acceptedActivationIds.current.length > 64) acceptedActivationIds.current.shift();
    const next = queuedActivations.current.shift() ?? null;
    if (next) {
      startNextActivation(next);
      return;
    }
    activationInFlight.current = null;
    setFileActivation(null);
  }, [startNextActivation]);
  const publishDocumentPath = useCallback((paths: readonly string[]) => setActiveDocumentPath(paths[0] ?? null), []);
  useShellBridge(handleShellEvent);
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const name = activeDocumentPath?.split(/[\\/]/).pop();
    const title = selectedWorkspace ? name ? `Toolbox — ${name}` : `Toolbox — ${selectedWorkspace.title}` : "Toolbox";
    void invoke("set_document_title", { title }).catch(() => undefined);
  }, [activeDocumentPath, selectedWorkspace?.id]);
  useEffect(() => {
    document.documentElement.style.zoom = viewZoom === 1 ? "" : String(viewZoom);
    localStorage.setItem("toolbox-view-zoom", String(viewZoom));
  }, [viewZoom]);
  // The app menu owns these accelerators natively; this covers the webview when the menu does not.
  useEffect(() => {
    const onEditorKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || settingsSection || event.altKey || !(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      const command = key === "z" ? (event.shiftKey ? "redo" : "undo") : key === "e" ? "export" : null;
      if (!command) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (key === "z" && target?.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
      dispatchEditorCommand(command);
    };
    document.addEventListener("keydown", onEditorKey);
    return () => document.removeEventListener("keydown", onEditorKey);
  }, [dispatchEditorCommand, settingsSection]);
  // Escape leaves the open workspace unless something on top of it, or a running batch, owns the key.
  useEffect(() => {
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || settingsSection || !selectedTool) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (target?.closest('[role="menu"], [role="dialog"], [role="alertdialog"], input, textarea, select, [contenteditable="true"]')) return;
      if (document.querySelector('.run-controls, [role="dialog"], [role="alertdialog"], [role="menu"]')) return;
      event.preventDefault();
      goHome();
    };
    window.addEventListener("keydown", onEscape);
    return () => window.removeEventListener("keydown", onEscape);
  }, [goHome, selectedTool, settingsSection]);
  useEffect(() => {
    if (!focusWorkspaceHeadingOnNavigation.current || selectedWorkspace?.id !== "pdf-convert") return;
    focusWorkspaceHeadingOnNavigation.current = false;
    workspaceHeadingRef.current?.focus();
  }, [selectedTool, selectedWorkspace?.id]);
  useEffect(() => {
    document.body.dataset.theme =
      (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark";
  }, []);
  const closeSettings = useCallback(() => {
    setSettingsSection(null);
    setFocusAfterSettings("settings");
  }, []);
  const findToolFromSettings = useCallback(() => {
    setSettingsSection(null);
    setWorkspaceSourceAction(null);
    setSelectedTool(null);
    setFocusAfterSettings("search");
  }, []);
  useEffect(() => {
    if (settingsSection || !focusAfterSettings) return;
    if (focusAfterSettings === "settings") settingsTriggerRef.current?.focus();
    else searchInputRef.current?.focus();
    setFocusAfterSettings(null);
  }, [settingsSection, focusAfterSettings]);
  useEffect(() => {
    const onShortcut = (event: KeyboardEvent) => {
      const id = matchShortcut(event, "window");
      if (id === "find") {
        event.preventDefault();
        if (settingsSection || selectedTool) return findToolFromSettings();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      } else if (id === "settings" || id === "shortcuts") {
        event.preventDefault();
        setSettingsSection(id === "settings" ? "appearance" : "keyboard-shortcuts");
      } else if (id === "open" && !settingsSection && !selectedTool) {
        event.preventDefault();
        void openFilesFromMenu();
      } else if ((id === "run" || id === "open") && !settingsSection) {
        event.preventDefault();
        pressShortcutTarget(commandCenterRef.current, id);
      }
    };

    window.addEventListener("keydown", onShortcut);
    return () => window.removeEventListener("keydown", onShortcut);
  }, [settingsSection, selectedTool, findToolFromSettings, openFilesFromMenu]);
  useEffect(() => {
    const openFromCompatibilityNav = (event: Event) => {
      const id = (event as CustomEvent<string>).detail;
      const tool = UtilityRegistry.find((item) => item.id === id);
      if (tool) openTool(tool);
    };
    window.addEventListener("toolbox:open", openFromCompatibilityNav);
    return () =>
      window.removeEventListener("toolbox:open", openFromCompatibilityNav);
  }, [openTool]);
  useEffect(() => {
    const openSettings = (event: Event) => {
      setSettingsSection((event as CustomEvent<SettingsSection | undefined>).detail ?? "appearance");
    };
    window.addEventListener("toolbox:open-settings", openSettings);
    return () => window.removeEventListener("toolbox:open-settings", openSettings);
  }, []);

  const favoriteButton = (tool: ToolDefinition) => (
    <Button
      variant={favorites.includes(tool.id) ? "secondary" : "ghost"}
      size="icon-sm"
      className="favorite-button"
      aria-label={favorites.includes(tool.id) ? `Remove ${tool.title} from favorites` : `Add ${tool.title} to favorites`}
      aria-pressed={favorites.includes(tool.id)}
      onClick={() => toggleFavorite(tool.id)}
    >
      {favorites.includes(tool.id) ? "★" : "☆"}
    </Button>
  );

  return (
    <>
    <div className={`app-shell ${railWorkspace ? "app-shell--view" : ""}`} hidden={settingsSection !== null}>
      <aside className="app-rail" aria-label={railWorkspace ? railWorkspace.title : "Toolbox navigation"}>
        {railWorkspace && selectedTool ? (
          <>
            <Button className="rail-link back-link" variant="ghost" onClick={goHome}>← All tools</Button>
            <p className="eyebrow view-rail-heading">{railWorkspace.title}</p>
            <WorkspaceCommandRail
              actions={availability.filter(toolsForWorkspace(railWorkspace))}
              activeId={selectedTool.id}
              onSelect={(id) => {
                const tool = UtilityRegistry.find((item) => item.id === id);
                if (tool && tool.id !== selectedTool.id) openTool(tool);
              }}
              label={`${railWorkspace.title} tools`}
              disabled={running}
            />
          </>
        ) : (
          <>
            <div className="brand-lockup">
              <span className="brand-mark">T</span>
              <span>Toolbox</span>
            </div>
            <nav className="rail-nav" aria-label="Workspace navigation">
              {(["all", "favorites"] as const).map((item) => (
                <Button
                  className="rail-link"
                  key={item}
                  variant={!selectedTool && filter === item ? "secondary" : "ghost"}
                  aria-current={!selectedTool && filter === item ? "page" : undefined}
                  onClick={() => selectLibrary(item)}
                >
                  {item === "all" ? "All tools" : "Favorites"}
                </Button>
              ))}
            </nav>
            <p className="eyebrow category-label">Views</p>
            <nav className="rail-nav" aria-label="Views">
              {openableWorkspaces.map(({ workspace, tools }) => (
                <Button
                  className="rail-link"
                  key={workspace.id}
                  variant={selectedWorkspace?.id === workspace.id ? "secondary" : "ghost"}
                  aria-current={selectedWorkspace?.id === workspace.id ? "page" : undefined}
                  onClick={() => openTool(firstOpenableTool(tools))}
                >
                  <TablerIcon name={workspace.symbol} className="rail-link-icon" />
                  {workspace.title}
                </Button>
              ))}
            </nav>
          </>
        )}
        <div className="rail-spacer" />
        <Button
          ref={settingsTriggerRef}
          variant="ghost"
          className="settings-link"
          aria-keyshortcuts={shortcutAria("settings")}
          onClick={() => setSettingsSection("appearance")}
        >
          Settings
          <ShortcutHint id="settings" />
        </Button>
        <p className="privacy-note">Files stay on this device.</p>
      </aside>
      <main className="command-center" aria-label="Tool detail" ref={commandCenterRef}>
        <header className={`topbar ${selectedTool ? "" : "topbar--home"}`}>
          {selectedTool && (
            <span className="breadcrumb" data-tauri-drag-region>
              Toolbox <span>/ Command center</span>
            </span>
          )}
          <span className="topbar__drag-region" aria-hidden="true" data-tauri-drag-region />
          {selectedTool ? (
            <Badge variant="secondary">On-device workspace</Badge>
          ) : (
            <div className="search-box">
              <span className="search-box__icon" aria-hidden="true">⌕</span>
              <Input
                ref={searchInputRef}
                type="text"
                aria-label="Search tools"
                onKeyDown={handleSearchKeyDown}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Find a tool, like merge, OCR or compress"
              />
              <kbd aria-hidden="true">{shortcutLabel("find")}</kbd>
            </div>
          )}
        </header>
        {rejectedFiles && (
          <Card role="alert" aria-live="assertive" className="mx-6 my-3 border-destructive/50">
            <CardHeader>
              <CardTitle>Some files could not be opened</CardTitle>
              <CardDescription>{rejectedFiles.reason}</CardDescription>
            </CardHeader>
            <CardContent>
              <ul className="list-disc space-y-1 pl-5">
                {rejectedFiles.paths.map((path) => (
                  <li key={path}>{path.split(/[\\/]/).pop() || path}</li>
                ))}
              </ul>
            </CardContent>
            <CardFooter>
              <Button variant="outline" onClick={() => setRejectedFiles(null)}>
                Dismiss
              </Button>
            </CardFooter>
          </Card>
        )}
        {selectedTool && selectedWorkspace ? (
          <section className={`tool-workspace ${selectedWorkspace.id === "pdf-editor" ? "pdf-studio" : ""}`} id="tool-detail">
            {selectedWorkspace.id === "pdf-editor" && (
              <div className="pdf-editor-header-line">
                <Button className="back-link" variant="ghost" size="sm" onClick={goHome}>← All tools</Button>
                <h1 className="workspace-title">{selectedWorkspace.title}</h1>
                {workspaceSourceAction && <Button variant="default" size="sm" className="pdf-editor-open-files" aria-label="Choose files to process" title="Open files" {...shortcutTarget("open")} onClick={() => void workspaceSourceAction()}>Open files<ShortcutHint id="open" /></Button>}
                {favoriteButton(selectedTool)}
              </div>
            )}
            <Card className={`tool-workspace-card ${selectedWorkspace.id === "pdf-editor" ? "py-0" : ""}`}>
              {selectedWorkspace.id === "pdf-editor" ? (
                <CardContent className="tool-view py-0">
                  <ViewFor utility={selectedTool} initialPaths={initialPaths}
                    fileActivation={fileActivation?.workspace === selectedWorkspace.id ? fileActivation : undefined}
                    onActivationAccepted={acceptActivation}
                    onFilesChange={publishDocumentPath}
                    onNavigate={navigateToPdfUtility}
                    onWorkspaceSourceAction={publishWorkspaceSourceAction} />
                </CardContent>
              ) : (
                <>
                  <CardHeader className="tool-workspace-header">
                    <div className="tool-workspace-kicker">
                      <span className="card-icon"><TablerIcon name={selectedWorkspace.symbol} /></span>
                      {selectedWorkspace.title} workspace
                    </div>
                    <div className="tool-workspace-heading">
                      <CardTitle className="workspace-card-title"><h1 ref={workspaceHeadingRef} tabIndex={-1} className="workspace-title">{selectedWorkspace.title}</h1></CardTitle>
                      <CardDescription><p>{selectedWorkspace.blurb}</p></CardDescription>
                    </div>
                    <CardAction>{favoriteButton(selectedTool)}</CardAction>
                  </CardHeader>
                  <CardContent className="tool-view">
                    <RunStateContext.Provider value={setRunning}>
                      <ViewFor
                        utility={selectedTool}
                        initialPaths={initialPaths}
                        fileActivation={fileActivation?.workspace === selectedWorkspace.id ? fileActivation : undefined}
                        onActivationAccepted={acceptActivation}
                        onFilesChange={publishDocumentPath}
                        onWorkspaceSourceAction={publishWorkspaceSourceAction}
                      />
                    </RunStateContext.Provider>
                  </CardContent>
                </>
              )}
            </Card>
          </section>
        ) : (
          <section className="tool-library-section" aria-labelledby="tool-library-title">
            <h1 id="tool-library-title" className="sr-only">Tool library</h1>
            {favoriteTools.length > 0 && (
              <div className="favorite-chips" role="group" aria-label="Favorite tools">
                <span className="favorite-chips-label">Favorites</span>
                {favoriteTools.map((tool) => (
                  <Button key={tool.id} variant="outline" size="sm" className="favorite-chip" onClick={() => openTool(tool)}>
                    {tool.title}
                  </Button>
                ))}
              </div>
            )}
            <div className="view-grid">
              {visibleWorkspaces.map(({ workspace, actions }) => {
                const expanded = normalizedSearch !== "" || filter === "favorites" || expandedViews.includes(workspace.id);
                const shown = expanded ? actions : actions.slice(0, visibleToolsPerView);
                const hiddenCount = actions.length - shown.length;
                const titleId = `view-section-${workspace.id}`;
                return (
                  <section className="view-section" key={workspace.id} aria-labelledby={titleId}>
                    <Separator />
                    <div className="view-section-heading">
                      <h2 id={titleId}>{workspace.title}</h2>
                      <Button variant="link" size="sm" className="view-open-link" aria-label={`Open ${workspace.title}`} onClick={() => openTool(firstOpenableTool(actions))}>
                        Open view
                      </Button>
                    </div>
                    <div className="view-tool-list">
                      {shown.map((tool) => (
                        <Button
                          type="button"
                          key={tool.id}
                          className="view-tool-link"
                          variant="ghost"
                          size="sm"
                          data-command-result={tool.id}
                          aria-label={`Open ${tool.title}`}
                          title={tool.blurb}
                          onKeyDown={handleCommandResultKeyDown}
                          onClick={() => openTool(tool)}
                        >
                          {tool.title}
                        </Button>
                      ))}
                      {hiddenCount > 0 && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="view-tool-link view-more-tools"
                          onClick={() => setExpandedViews((current) => [...current, workspace.id])}
                        >
                          {hiddenCount} more {hiddenCount === 1 ? "tool" : "tools"}
                        </Button>
                      )}
                    </div>
                  </section>
                );
              })}
            </div>
            {visibleWorkspaces.length === 0 && (
              <Card className="tool-empty-state" role="status">
                <CardHeader>
                <CardTitle><h3>{filter === "favorites" && !search ? "No favorite tools yet" : "No matching tools"}</h3></CardTitle>
                <CardDescription><p>{
                  filter === "favorites" && !search
                    ? "Save a tool with the star in its view to keep it here."
                    : "Try another term or clear the current filters."
                }</p></CardDescription>
                </CardHeader>
                <CardFooter>
                <Button variant="ghost" onClick={() => selectLibrary("all")}>
                  Browse all tools
                </Button>
                </CardFooter>
              </Card>
            )}
          </section>
        )}
        <p className="sr-only" role="status" aria-live="polite">
          {selectedTool
            ? `${selectedWorkspace?.title ?? selectedTool.title} workspace open.`
            : "No tool selected."}
        </p>
      </main>
    </div>
    {settingsSection && (
      <SettingsPage
        section={settingsSection}
        onSectionChange={setSettingsSection}
        onClose={closeSettings}
        onFindTool={findToolFromSettings}
      />
    )}
    </>
  );
};

const ViewFor = ({ utility, initialPaths, fileActivation, onActivationAccepted, onFilesChange, onNavigate, onWorkspaceSourceAction }: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  fileActivation?: Extract<ShellEvent, { kind: "files" }>;
  onActivationAccepted?: (activationId: string) => void;
  onFilesChange?: (paths: readonly string[]) => void;
  onNavigate?: PdfEditorNavigation;
  onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
}) => {
  if (utility.capability.nativeAvailability === "unavailable") {
    return <UnavailableToolView utility={utility} />;
  }
  const workspace = workspaceForTool(utility.id);
  if (!workspace) return <PlannedToolView utility={utility} />;
  if (workspace.id === "pdf-editor") return <PDFEditorWorkspaceView utility={utility} onNavigate={onNavigate} onWorkspaceSourceAction={onWorkspaceSourceAction} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange} />;
  if (workspace.id === "image-editor") return <ImageEditorWorkspaceView utility={utility} fileActivation={fileActivation} onActivationAccepted={onActivationAccepted} onFilesChange={onFilesChange} onWorkspaceSourceAction={onWorkspaceSourceAction} />;
  if (workspace.id === "pdf-convert") return <PDFConversionWorkspaceView utility={utility} initialPaths={initialPaths} />;
  const View = workspaceViews[workspace.id];
  return <View utility={utility} />;
};
