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
import type { ToolDefinition, WorkspaceId } from "../contracts";
import { TablerIcon } from "../components/TablerIcon";
import type { WorkspaceSourceAction } from "../components/ToolScaffold";
import {
  Button,
  Card,
  EmptyState,
  IconButton,
  Pill,
  Section,
  SegmentedControl,
  StatusBar,
  TextField,
} from "../design-system";
import { SecurityWorkspaceView } from "./SecurityWorkspaceView";
import { PDFEditorWorkspaceView } from "./PDFEditorWorkspaceView";
import { PDFConversionWorkspaceView } from "./PDFConversionWorkspaceView";
import { ImageEditorWorkspaceView } from "./ImageEditorWorkspaceView";
import { MediaWorkspaceView } from "./MediaWorkspaceView";
import { PlannedToolView, UnavailableToolView } from "./PlannedToolView";
import { SettingsPanel } from "./SettingsPanel";

const workspaceViews = {
  "file-security": SecurityWorkspaceView,
  "pdf-editor": PDFEditorWorkspaceView,
  "pdf-convert": PDFConversionWorkspaceView,
  "image-editor": ImageEditorWorkspaceView,
  "media-tools": MediaWorkspaceView,
} satisfies Record<WorkspaceId, ComponentType<{ utility: ToolDefinition }>>;

const designIconByToolId: Record<string, string> = {
  "pdf-unlock": "remove-password",
  "pdf-page-numbers": "page-numbers",
  "pdf-merge": "merge-pdf",
  "pdf-watermark": "watermark-pdf",
  "pdf-crop": "crop-pdf",
  "pdf-edit": "edit-pdf",
  "pdf-protect": "protect-pdf",
  "office-protect": "protect-pdf",
  "images-to-pdf": "images-to-pdf",
  "pdf-to-images": "pdf-to-images",
  "pdf-to-text": "pdf-to-text",
  "pdf-split": "split-pdf",
  "pdf-image-extract": "extract-images",
  "pdf-sign": "signature",
  "pdf-ocr": "ocr-pdf",
  "pdf-remove-pages": "file-minus",
  "pdf-extract-pages": "extract-pages",
  "pdf-organize": "organize-pdf",
  "pdf-compress": "compress-pdf",
  "heic-convert": "convert-format",
  compress: "compress-images",
  resize: "resize-images",
  rotate: "rotate-images",
  crop: "crop-images",
  "icon-set": "generate-icons",
  "gif-create": "create-gif",
  "gif-extract": "extract-gif",
  "image-watermark": "watermark-images",
  "image-metadata": "image-metadata",
  "image-tone": "image-tone",
  "tiff-pages": "layers",
  "image-blur-faces": "face-id",
  "image-remove-bg": "wand",
};

const iconName = (tool: ToolDefinition) =>
  designIconByToolId[tool.id] ?? tool.symbol;

const libraryFilters = [
  { value: "all", label: "All tools" },
  { value: "PDF", label: "PDF" },
  { value: "Images", label: "Images" },
  { value: "Documents", label: "Documents" },
  { value: "favorites", label: "Favorites" },
  { value: "recent", label: "Recent" },
] as const;

type LibraryFilter = (typeof libraryFilters)[number]["value"];

export const MainPage = () => {
  const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);
  const [workspaceSourceAction, setWorkspaceSourceAction] = useState<WorkspaceSourceAction | null>(null);
  const publishWorkspaceSourceAction = useCallback((action: WorkspaceSourceAction | null) => {
    setWorkspaceSourceAction(() => action);
  }, []);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [filter, setFilter] = useState<LibraryFilter>("all");
  const [search, setSearch] = useState("");
  const searchInputRef = useRef<HTMLInputElement>(null);
  const commandCenterRef = useRef<HTMLElement>(null);
  const searchShortcut = navigator.platform.toLowerCase().includes("win")
    ? "Ctrl + K"
    : "⌘ + K";
  const [favorites, setFavorites] = useState<string[]>(() =>
    JSON.parse(localStorage.getItem("toolbox-favorites") || "[]"),
  );
  const [recent, setRecent] = useState<string[]>(() =>
    JSON.parse(localStorage.getItem("toolbox-recent") || "[]"),
  );
  const recentTools = useMemo(
    () =>
      recent
        .map((id) => UtilityRegistry.find((tool) => tool.id === id))
        .filter(Boolean) as ToolDefinition[],
    [recent],
  );
  const normalizedSearch = search.trim().toLowerCase();
  const recentPreviewTools = recentTools.slice(0, 3);
  const visibleWorkspaces = useMemo(
    () =>
      ToolWorkspaceRegistry.map((workspace) => {
        const actions = toolsForWorkspace(workspace).filter((tool) => {
          const matchesScope =
            filter === "all" ||
            (filter === "recent" && recent.includes(tool.id)) ||
            (filter === "favorites" && favorites.includes(tool.id)) ||
            (filter !== "recent" && filter !== "favorites" && tool.category === filter);
          const searchText = `${tool.title} ${tool.blurb} ${workspace.title} ${workspace.blurb}`.toLowerCase();
          return matchesScope && searchText.includes(normalizedSearch);
        });
        return { workspace, actions };
      }).filter(({ actions }) => actions.length > 0),
    [filter, normalizedSearch, favorites, recent],
  );
  const openTool = (tool: ToolDefinition) => {
    setWorkspaceSourceAction(null);
    setSelectedTool(tool);
    setRecent((current) => {
      const next = [tool.id, ...current.filter((id) => id !== tool.id)].slice(
        0,
        8,
      );
      localStorage.setItem("toolbox-recent", JSON.stringify(next));
      return next;
    });
  };
  const toggleFavorite = (id: string) =>
    setFavorites((current) => {
      const next = current.includes(id)
        ? current.filter((item) => item !== id)
        : [...current, id];
      localStorage.setItem("toolbox-favorites", JSON.stringify(next));
      return next;
    });
  const commandResults = () =>
    Array.from(
      commandCenterRef.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-command-result]",
      ) ?? [],
    ).filter((button) => !button.disabled);
  const handleSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key !== "ArrowDown") return;

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
  const selectLibrary = (
    nextFilter: LibraryFilter,
  ) => {
    setFilter(nextFilter);
    setSearch("");
    setWorkspaceSourceAction(null);
    setSelectedTool(null);
  };
  const workspaceTitle =
    filter === "all"
      ? "All tools"
      : filter === "favorites"
        ? "Favorites"
        : filter === "recent"
          ? "Recent"
          : `${filter} tools`;
  const selectedWorkspace = selectedTool
    ? workspaceForTool(selectedTool.id)
    : undefined;
  useEffect(() => {
    document.body.dataset.theme =
      (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark";
  }, []);
  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "k") return;

      event.preventDefault();
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    };

    window.addEventListener("keydown", focusSearch);
    return () => window.removeEventListener("keydown", focusSearch);
  }, []);
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

  return (
    <div className="app-shell">
      <aside className="app-rail" aria-label="Toolbox navigation">
        <div className="brand-lockup">
          <span className="brand-mark">T</span>
          <span>Toolbox</span>
        </div>
        <p className="eyebrow">Workspace</p>
        <nav className="rail-nav" aria-label="Workspace navigation">
          {(["all", "favorites", "recent"] as const).map((item) => (
            <Button
              className="rail-link"
              key={item}
              variant="ghost"
              aria-current={filter === item ? "page" : undefined}
              onClick={() => selectLibrary(item)}
              aria-label={
                item === "all"
                  ? "All tools"
                  : item[0].toUpperCase() + item.slice(1)
              }
            >
              {item === "all"
                ? "All tools"
                : item[0].toUpperCase() + item.slice(1)}
            </Button>
          ))}
        </nav>
        <p className="eyebrow category-label">Categories</p>
        <nav className="rail-nav" aria-label="Tool categories">
          {(["PDF", "Images", "Documents"] as const).map((item) => (
            <Button
              className="rail-link"
              key={item}
              variant="ghost"
              aria-current={filter === item ? "page" : undefined}
              onClick={() => selectLibrary(item)}
              aria-label={item}
            >
              {item}
            </Button>
          ))}
        </nav>
        <p className="eyebrow category-label">Quick access</p>
            <nav className="quick-tools">
              {(recentPreviewTools.length
                ? recentPreviewTools
                : UtilityRegistry.slice(0, 4)
              ).map(
                (tool) => (
              <Button
                className="quick-tool-link"
                key={tool.id}
                variant="ghost"
                onClick={() => openTool(tool)}
                aria-current={selectedTool?.id === tool.id ? "page" : undefined}
              >
                <span className="card-icon quick-tool-icon">
                  <TablerIcon name={iconName(tool)} />
                </span>
                {tool.shortTitle}
              </Button>
            ),
          )}
        </nav>
        <div className="rail-spacer" />
        <Button
          variant="ghost"
          className="settings-link"
          aria-current={settingsOpen ? "page" : undefined}
          onClick={() => setSettingsOpen(true)}
        >
          Settings
        </Button>
        <p className="privacy-note">
          <strong>Private by default</strong>Files stay on this device.
        </p>
      </aside>
      <main className="command-center" aria-label="Tool detail" ref={commandCenterRef}>
        <header className="topbar">
          <span className="breadcrumb">
            Toolbox <span>/ Command center</span>
          </span>
          <Pill className="privacy-pill" tone="info">
            <i aria-hidden="true" /> On-device workspace
          </Pill>
        </header>
        {selectedTool && selectedWorkspace ? (
          <section className={`tool-workspace ${selectedWorkspace.id === "pdf-editor" ? "pdf-studio" : ""}`} id="tool-detail">
            {selectedWorkspace.id === "pdf-editor" ? (
              <div className="pdf-editor-header-line">
                <button className="back-link" type="button" onClick={() => { setWorkspaceSourceAction(null); setSelectedTool(null); }}>← All tools</button>
                <h1 className="workspace-title">{selectedWorkspace.title}</h1>
                {workspaceSourceAction && <button type="button" className="pdf-editor-open-files" aria-label="Choose files to process" title="Open files" onClick={() => void workspaceSourceAction()}>Open files</button>}
                <Button
                  variant="ghost"
                  size="sm"
                  className="favorite-button"
                  aria-pressed={favorites.includes(selectedTool.id)}
                  onClick={() => toggleFavorite(selectedTool.id)}
                >
                  {favorites.includes(selectedTool.id) ? "★ Saved" : "☆ Save"}
                </Button>
              </div>
            ) : <button className="back-link" type="button" onClick={() => setSelectedTool(null)}>← All tools</button>}
            <div className="tool-workspace-card">
              <div className="tool-workspace-kicker">
                <span className="card-icon">
                  <TablerIcon name={selectedWorkspace.symbol} />
                </span>
                {selectedWorkspace.title} workspace
              </div>
              {selectedWorkspace.id !== "pdf-editor" && <div className="tool-workspace-heading">
                <div>
                  <h1 className="workspace-title">{selectedWorkspace.title}</h1>
                  <p>{selectedWorkspace.blurb}</p>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="favorite-button"
                  aria-pressed={favorites.includes(selectedTool.id)}
                  onClick={() => toggleFavorite(selectedTool.id)}
                >
                  {favorites.includes(selectedTool.id) ? "★ Saved" : "☆ Save"}
                </Button>
              </div>}
              <div className="tool-view">
                <ViewFor utility={selectedTool} onWorkspaceSourceAction={selectedWorkspace.id === "pdf-editor" ? publishWorkspaceSourceAction : undefined} />
              </div>
            </div>
          </section>
        ) : (
          <>
            <div className="workspace-toolbar">
              <div>
                <h3 className="initial-status">
                  {filter === "all" ? "Ready to process" : "Your workspace"}
                </h3>
                <h1>{workspaceTitle}</h1>
              </div>
              <TextField
                ref={searchInputRef}
                className="search-box"
                inputClassName="search-box__input"
                label="Search tools"
                labelHidden
                leading={<span aria-hidden="true">⌕</span>}
                trailing={<kbd aria-hidden="true">{searchShortcut}</kbd>}
                onKeyDown={handleSearchKeyDown}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Find a tool or action"
              />
            </div>
            <Section className="tool-library-section" title="Tool library">
              <SegmentedControl
                className="filter-row"
                label="Tool library filters"
                value={filter}
                options={libraryFilters}
                onChange={(value) => selectLibrary(value as LibraryFilter)}
              />
              <div className="tool-grid">
                {visibleWorkspaces.map(({ workspace, actions }) => (
                  <Card
                    as="article"
                    className="tool-card workspace-card"
                    key={workspace.id}
                  >
                    <div className="workspace-card-heading">
                      <span className="workspace-card-eyebrow">
                        {actions.length} {actions.length === 1 ? "action" : "actions"} · {workspace.categories.join(" + ")}
                      </span>
                    </div>
                    <h3>{workspace.title}</h3>
                    <p>{workspace.blurb}</p>
                    <div className="workspace-action-list">
                      {actions.map((tool) => (
                        <div className="workspace-action-row" key={tool.id}>
                          <Button
                            type="button"
                            className="workspace-action-button"
                            variant="ghost"
                            size="sm"
                            data-command-result={tool.id}
                            aria-label={`Open ${tool.title}`}
                            onKeyDown={handleCommandResultKeyDown}
                            onClick={() => openTool(tool)}
                          >
                            <span className="card-icon">
                              <TablerIcon name={iconName(tool)} />
                            </span>
                            <span>
                              <strong>{tool.title}</strong>
                              <small>{tool.blurb}</small>
                            </span>
                          </Button>
                          <IconButton
                            className="favorite-button"
                            aria-label={
                              favorites.includes(tool.id)
                                ? `Remove ${tool.title} from favorites`
                                : `Add ${tool.title} to favorites`
                            }
                            pressed={favorites.includes(tool.id)}
                            onClick={() => toggleFavorite(tool.id)}
                          >
                            {favorites.includes(tool.id) ? "★" : "☆"}
                          </IconButton>
                        </div>
                      ))}
                    </div>
                    <footer>
                      <span>One file surface · {actions.length} outcomes</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openTool(actions[0])}
                        aria-label={`Open ${workspace.title}`}
                      >
                        Open workspace →
                      </Button>
                    </footer>
                  </Card>
                ))}
              </div>
              {visibleWorkspaces.length === 0 && (
                <EmptyState
                  className="tool-empty-state"
                  title={
                    filter === "favorites" && !search
                      ? "No favorite tools yet"
                      : filter === "recent" && !search
                        ? "No recent tools yet"
                        : "No matching tools"
                  }
                  description={
                    filter === "favorites" && !search
                      ? "Save a tool with the star to keep it here."
                      : filter === "recent" && !search
                        ? "Open a tool and it will appear here."
                        : "Try another term or clear the current filters."
                  }
                  action={
                    <Button
                      variant="ghost"
                      onClick={() => selectLibrary("all")}
                    >
                      Browse all tools
                    </Button>
                  }
                />
              )}
            </Section>
          </>
        )}
        <p className="sr-only" role="status" aria-live="polite">
          {selectedTool
            ? `${selectedWorkspace?.title ?? selectedTool.title} workspace open.`
            : "No tool selected."}
        </p>
        {!selectedTool && (
          <StatusBar className="command-status-bar">
            <span>{UtilityRegistry.length} tools</span>
            <span>On-device processing</span>
          </StatusBar>
        )}
      </main>
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </div>
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
