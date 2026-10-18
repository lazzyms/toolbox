import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  ToolWorkspaceRegistry,
  UtilityRegistry,
  toolsForWorkspace,
  workspaceForTool,
} from "../registry";
import type { ToolDefinition, ToolWorkspaceDefinition, WorkspaceId } from "../contracts";
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
import { SecurityWorkspaceView } from "./SecurityWorkspaceView";
import { PDFEditorWorkspaceView } from "./PDFEditorWorkspaceView";
import { PDFConversionWorkspaceView } from "./PDFConversionWorkspaceView";
import { ImageEditorWorkspaceView } from "./ImageEditorWorkspaceView";
import { CompressFilesWorkspaceView } from "./CompressFilesWorkspaceView";
import { MediaWorkspaceView } from "./MediaWorkspaceView";
import { PlannedToolView, UnavailableToolView } from "./PlannedToolView";
import { SettingsPage, type SettingsSection } from "./SettingsPage";
import { favoritesStorageKey, recentStorageKey } from "../savedToolIds";

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

const firstOpenableTool = (workspace: ToolWorkspaceDefinition) => {
  const tools = toolsForWorkspace(workspace);
  return tools.find((tool) => tool.capability.nativeAvailability !== "unavailable") ?? tools[0];
};

export const MainPage = () => {
  const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);
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
      .filter(Boolean) as ToolDefinition[],
    [favorites],
  );
  const normalizedSearch = search.trim().toLowerCase();
  const visibleWorkspaces = useMemo(
    () =>
      ToolWorkspaceRegistry.map((workspace) => {
        const actions = toolsForWorkspace(workspace).filter((tool) => {
          const matchesScope = filter === "all" || favorites.includes(tool.id);
          const searchText = `${tool.title} ${tool.blurb} ${tool.keywords?.join(" ") ?? ""} ${workspace.title} ${workspace.blurb}`.toLowerCase();
          return matchesScope && searchText.includes(normalizedSearch);
        });
        return { workspace, actions };
      }).filter(({ actions }) => actions.length > 0),
    [filter, normalizedSearch, favorites],
  );
  const openTool = (tool: ToolDefinition) => {
    setWorkspaceSourceAction(null);
    setSelectedTool(tool);
    const recent: string[] = JSON.parse(localStorage.getItem(recentStorageKey) || "[]");
    localStorage.setItem(recentStorageKey, JSON.stringify([tool.id, ...recent.filter((id) => id !== tool.id)].slice(0, 8)));
  };
  const goHome = () => {
    setWorkspaceSourceAction(null);
    setSelectedTool(null);
  };
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
      } else if ((id === "run" || id === "open") && !settingsSection) {
        event.preventDefault();
        pressShortcutTarget(commandCenterRef.current, id);
      }
    };

    window.addEventListener("keydown", onShortcut);
    return () => window.removeEventListener("keydown", onShortcut);
  }, [settingsSection, selectedTool, findToolFromSettings]);
  useEffect(() => {
    const openFromCompatibilityNav = (event: Event) => {
      const id = (event as CustomEvent<string>).detail;
      const tool = UtilityRegistry.find((item) => item.id === id);
      if (tool) openTool(tool);
    };
    window.addEventListener("toolbox:open", openFromCompatibilityNav);
    return () =>
      window.removeEventListener("toolbox:open", openFromCompatibilityNav);
  });
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
              actions={toolsForWorkspace(railWorkspace)}
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
              {ToolWorkspaceRegistry.map((workspace) => (
                <Button
                  className="rail-link"
                  key={workspace.id}
                  variant={selectedWorkspace?.id === workspace.id ? "secondary" : "ghost"}
                  aria-current={selectedWorkspace?.id === workspace.id ? "page" : undefined}
                  onClick={() => openTool(firstOpenableTool(workspace))}
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
                  <ViewFor utility={selectedTool} onWorkspaceSourceAction={publishWorkspaceSourceAction} />
                </CardContent>
              ) : (
                <>
                  <CardHeader className="tool-workspace-header">
                    <div className="tool-workspace-kicker">
                      <span className="card-icon"><TablerIcon name={selectedWorkspace.symbol} /></span>
                      {selectedWorkspace.title} workspace
                    </div>
                    <div className="tool-workspace-heading">
                      <CardTitle className="workspace-card-title"><h1 className="workspace-title">{selectedWorkspace.title}</h1></CardTitle>
                      <CardDescription><p>{selectedWorkspace.blurb}</p></CardDescription>
                    </div>
                    <CardAction>{favoriteButton(selectedTool)}</CardAction>
                  </CardHeader>
                  <CardContent className="tool-view">
                    <RunStateContext.Provider value={setRunning}>
                      <ViewFor utility={selectedTool} />
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
                      <Button variant="link" size="sm" className="view-open-link" aria-label={`Open ${workspace.title}`} onClick={() => openTool(firstOpenableTool(workspace))}>
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

const ViewFor = ({ utility, onWorkspaceSourceAction }: { utility: ToolDefinition; onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void }) => {
  if (utility.capability.nativeAvailability === "unavailable") {
    return <UnavailableToolView utility={utility} />;
  }
  const workspace = workspaceForTool(utility.id);
  if (!workspace) return <PlannedToolView utility={utility} />;
  if (workspace.id === "pdf-editor") return <PDFEditorWorkspaceView utility={utility} onWorkspaceSourceAction={onWorkspaceSourceAction} />;
  const View = workspaceViews[workspace.id];
  return <View utility={utility} />;
};
