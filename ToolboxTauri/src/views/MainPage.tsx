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
  workspaceForTool,
} from "../registry";
import type { ToolDefinition, WorkspaceId } from "../contracts";
import { TablerIcon } from "../components/TablerIcon";
import type { WorkspaceSourceAction } from "../components/ToolScaffold";
import type { PdfEditorNavigation } from "./PDFEditorWorkspaceView";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SecurityWorkspaceView } from "./SecurityWorkspaceView";
import { PDFEditorWorkspaceView } from "./PDFEditorWorkspaceView";
import { PDFConversionWorkspaceView } from "./PDFConversionWorkspaceView";
import { ImageEditorWorkspaceView } from "./ImageEditorWorkspaceView";
import { MediaWorkspaceView } from "./MediaWorkspaceView";
import { PlannedToolView, UnavailableToolView } from "./PlannedToolView";
import { SettingsPanel } from "./SettingsPanel";
import { useShellBridge } from "../hooks/useShellBridge";
import { TOOL_DROP_EVENT, type ShellCommand, type ShellEvent } from "../hooks/useShellBridge";

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
  { value: "all", label: "All" },
  { value: "PDF", label: "PDF" },
  { value: "Images", label: "Images" },
  { value: "Documents", label: "Documents" },
  { value: "favorites", label: "Favorites" },
  { value: "recent", label: "Recent" },
] as const;

type LibraryFilter = (typeof libraryFilters)[number]["value"];

export const MainPage = () => {
  const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);
  const [initialPaths, setInitialPaths] = useState<readonly string[]>([]);
  const workspaceHeadingRef = useRef<HTMLHeadingElement>(null);
  const focusWorkspaceHeadingOnNavigation = useRef(false);
  const [fileActivation, setFileActivation] = useState<Extract<ShellEvent, { kind: "files" }> | null>(null);
  const [rejectedFiles, setRejectedFiles] = useState<Extract<ShellEvent, { kind: "rejected-files" }> | null>(null);
  const [activeDocumentPath, setActiveDocumentPath] = useState<string | null>(null);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [viewZoom, setViewZoom] = useState(() => Number(localStorage.getItem("toolbox-view-zoom") || "1"));
  const activationInFlight = useRef<string | null>(null);
  const queuedActivations = useRef<Extract<ShellEvent, { kind: "files" }>[]>([]);
  const acceptedActivationIds = useRef<string[]>([]);
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
  const openTool = useCallback((tool: ToolDefinition, paths: readonly string[] = []) => {
    setFileActivation(null);
    activationInFlight.current = null;
    queuedActivations.current = [];
    setActiveDocumentPath(paths[0] ?? null);
    setWorkspaceSourceAction(null);
    setInitialPaths([...paths]);
    setSelectedTool(tool);
    setRecent((current) => {
      const next = [tool.id, ...current.filter((id) => id !== tool.id)].slice(
        0,
        8,
      );
      localStorage.setItem("toolbox-recent", JSON.stringify(next));
      return next;
    });
  }, []);
  const closeWorkspace = useCallback(() => {
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
    if (tool) {
      focusWorkspaceHeadingOnNavigation.current = true;
      openTool(tool, paths);
    }
  }, [openTool]);
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
  const selectLibrary = (
    nextFilter: LibraryFilter,
  ) => {
    setFilter(nextFilter);
    setSearch("");
    setWorkspaceSourceAction(null);
    setSelectedTool(null);
    setFileActivation(null);
    setActiveDocumentPath(null);
    activationInFlight.current = null;
    queuedActivations.current = [];
  };
  const selectedWorkspace = selectedTool
    ? workspaceForTool(selectedTool.id)
    : undefined;
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
    const id = event.workspace === "pdf-editor" ? "pdf-edit" : "heic-convert";
    const tool = UtilityRegistry.find((item) => item.id === id);
    if (!tool) return;
    setRejectedFiles(null);
    setInitialPaths([]);
    setFileActivation(event);
    setActiveDocumentPath(event.paths[0] ?? null);
    setSelectedTool(tool);
    setWorkspaceSourceAction(null);
    setRecent((current) => {
      const next = [tool.id, ...current.filter((item) => item !== tool.id)].slice(0, 8);
      localStorage.setItem("toolbox-recent", JSON.stringify(next));
      return next;
    });
  }, []);
  const handleShellEvent = useCallback((event: ShellEvent) => {
    if (event.kind === "files") {
      if (acceptedActivationIds.current.includes(event.activationId)
        || activationInFlight.current === event.activationId
        || queuedActivations.current.some((queued) => queued.activationId === event.activationId)) return;
      if (activationInFlight.current) {
        queuedActivations.current.push(event);
        return;
      }
      activationInFlight.current = event.activationId;
      activateFile(event);
      return;
    }
    if (event.kind === "rejected-files") {
      setRejectedFiles(event);
      return;
    }
    if (event.kind === "dropped-files") {
      setRejectedFiles(null);
      if (!selectedTool) {
        setRejectedFiles({ kind: "rejected-files", paths: event.paths, reason: "Open a tool before dropping files here." });
      } else {
        window.dispatchEvent(new CustomEvent(TOOL_DROP_EVENT, { detail: event.paths }));
      }
      return;
    }
    switch (event.command) {
      case "search":
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        break;
      case "open":
        void openFilesFromMenu();
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
        setShortcutsOpen(true);
        break;
    }
  }, [activateFile, dispatchEditorCommand, openFilesFromMenu, selectedTool]);
  const acceptActivation = useCallback((activationId: string) => {
    if (activationInFlight.current !== activationId) return;
    acceptedActivationIds.current.push(activationId);
    if (acceptedActivationIds.current.length > 64) acceptedActivationIds.current.shift();
    const next = queuedActivations.current.shift() ?? null;
    activationInFlight.current = next?.activationId ?? null;
    if (next) {
      activateFile(next);
      return;
    }
    setFileActivation(null);
  }, [activateFile]);
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
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape") {
        if (settingsOpen) {
          event.preventDefault();
          setSettingsOpen(false);
          return;
        }
        if (shortcutsOpen) {
          event.preventDefault();
          setShortcutsOpen(false);
          return;
        }
        const target = event.target instanceof HTMLElement ? event.target : null;
        if (target?.closest('[role="menu"], [role="dialog"], input, textarea, [contenteditable="true"]')) return;
        if (selectedTool) {
          event.preventDefault();
          closeWorkspace();
        }
        return;
      }
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      const command = key === "k" ? "search" : key === "o" ? "open" : key === "z" ? (event.shiftKey ? "redo" : "undo") : key === "e" ? "export" : null;
      if (!command) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (key === "z" && target?.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
      if (command === "search") handleShellEvent({ kind: "command", command });
      else if (command === "open") handleShellEvent({ kind: "command", command });
      else dispatchEditorCommand(command);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [closeWorkspace, dispatchEditorCommand, handleShellEvent, selectedTool, settingsOpen, shortcutsOpen]);
  useEffect(() => {
    if (!focusWorkspaceHeadingOnNavigation.current || selectedWorkspace?.id !== "pdf-convert") return;
    focusWorkspaceHeadingOnNavigation.current = false;
    workspaceHeadingRef.current?.focus();
  }, [selectedTool, selectedWorkspace?.id]);
  useEffect(() => {
    document.body.dataset.theme =
      (localStorage.getItem("toolbox-theme") as "dark" | "light") || "dark";
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
  }, [openTool]);

  return (
    <Dialog open={settingsOpen} onOpenChange={setSettingsOpen}>
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
              variant={filter === item ? "secondary" : "ghost"}
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
              variant={filter === item ? "secondary" : "ghost"}
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
                variant={selectedTool?.id === tool.id ? "secondary" : "ghost"}
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
        <DialogTrigger asChild>
          <Button
            variant={settingsOpen ? "secondary" : "ghost"}
            className="settings-link"
            aria-current={settingsOpen ? "page" : undefined}
          >
            Settings
          </Button>
        </DialogTrigger>
        <p className="privacy-note">
          <strong>Private by default</strong>Files stay on this device.
        </p>
      </aside>
      <main className="command-center" aria-label="Tool detail" ref={commandCenterRef}>
        <header className={`topbar ${selectedTool ? "" : "topbar--home"}`}>
          <span className="breadcrumb" data-tauri-drag-region>
            Toolbox <span>/ Command center</span>
          </span>
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
                placeholder="Find a tool or action"
              />
              <kbd aria-hidden="true">{searchShortcut}</kbd>
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
            {selectedWorkspace.id === "pdf-editor" ? (
              <div className="pdf-editor-header-line">
                <Button className="back-link" variant="ghost" size="sm" onClick={closeWorkspace}>← All tools</Button>
                <h1 className="workspace-title">{selectedWorkspace.title}</h1>
                {workspaceSourceAction && <Button variant="default" size="sm" className="pdf-editor-open-files" aria-label="Choose files to process" title="Open files" onClick={() => void workspaceSourceAction()}>Open files</Button>}
                <Button
                  variant={favorites.includes(selectedTool.id) ? "secondary" : "ghost"}
                  size="icon-sm"
                  className="favorite-button"
                  aria-label={favorites.includes(selectedTool.id) ? `Remove ${selectedTool.title} from favorites` : `Add ${selectedTool.title} to favorites`}
                  aria-pressed={favorites.includes(selectedTool.id)}
                  onClick={() => toggleFavorite(selectedTool.id)}
                >
                  {favorites.includes(selectedTool.id) ? "★" : "☆"}
                </Button>
              </div>
            ) : <Button className="back-link" variant="ghost" size="sm" onClick={closeWorkspace}>← All tools</Button>}
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
                    <CardAction>
                      <Button
                        variant={favorites.includes(selectedTool.id) ? "secondary" : "ghost"}
                        size="icon-sm"
                        className="favorite-button"
                        aria-label={favorites.includes(selectedTool.id) ? `Remove ${selectedTool.title} from favorites` : `Add ${selectedTool.title} to favorites`}
                        aria-pressed={favorites.includes(selectedTool.id)}
                        onClick={() => toggleFavorite(selectedTool.id)}
                      >
                        {favorites.includes(selectedTool.id) ? "★" : "☆"}
                      </Button>
                    </CardAction>
                  </CardHeader>
                  <CardContent className="tool-view">
                    <ViewFor
                      utility={selectedTool}
                      initialPaths={initialPaths}
                      fileActivation={fileActivation?.workspace === selectedWorkspace.id ? fileActivation : undefined}
                      onActivationAccepted={acceptActivation}
                      onFilesChange={publishDocumentPath}
                      onNavigate={selectedWorkspace.id === "pdf-editor" ? navigateToPdfUtility : undefined}
                      onWorkspaceSourceAction={publishWorkspaceSourceAction}
                    />
                  </CardContent>
                </>
              )}
            </Card>
          </section>
        ) : (
          <>
            <section className="tool-library-section" aria-labelledby="tool-library-title">
              <h2 id="tool-library-title">Tool library</h2>
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                className="filter-row"
                aria-label="Tool library filters"
                value={filter}
                onValueChange={(value) => {
                  if (value) selectLibrary(value as LibraryFilter);
                }}
              >
                {libraryFilters.map((item) => (
                  <ToggleGroupItem key={item.value} value={item.value}>
                    {item.label}
                  </ToggleGroupItem>
                ))}
              </ToggleGroup>
              <div className="tool-grid">
                {visibleWorkspaces.map(({ workspace, actions }) => (
                  <Card
                    role="article"
                    className="tool-card workspace-card"
                    key={workspace.id}
                  >
                    <CardHeader className="workspace-card-header">
                      <span className="workspace-card-eyebrow">
                        {actions.length} {actions.length === 1 ? "action" : "actions"} · {workspace.categories.join(" + ")}
                      </span>
                      <CardTitle className="workspace-card-title"><h3>{workspace.title}</h3></CardTitle>
                      <CardDescription className="workspace-card-description"><p>{workspace.blurb}</p></CardDescription>
                    </CardHeader>
                    <CardContent className="workspace-action-list">
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
                          <Button
                            variant={favorites.includes(tool.id) ? "secondary" : "ghost"}
                            size="icon-sm"
                            className="favorite-button"
                            aria-label={
                              favorites.includes(tool.id)
                                ? `Remove ${tool.title} from favorites`
                                : `Add ${tool.title} to favorites`
                            }
                            aria-pressed={favorites.includes(tool.id)}
                            onClick={() => toggleFavorite(tool.id)}
                          >
                            {favorites.includes(tool.id) ? "★" : "☆"}
                          </Button>
                        </div>
                      ))}
                    </CardContent>
                    <CardFooter className="workspace-card-footer">
                      <span>One file surface · {actions.length} outcomes</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openTool(actions[0])}
                        aria-label={`Open ${workspace.title}`}
                      >
                        Open workspace →
                      </Button>
                    </CardFooter>
                  </Card>
                ))}
              </div>
              {visibleWorkspaces.length === 0 && (
                <Card className="tool-empty-state" role="status">
                  <CardHeader>
                  <CardTitle><h3>{
                    filter === "favorites" && !search
                      ? "No favorite tools yet"
                      : filter === "recent" && !search
                        ? "No recent tools yet"
                        : "No matching tools"
                  }</h3></CardTitle>
                  <CardDescription><p>{
                    filter === "favorites" && !search
                      ? "Save a tool with the star to keep it here."
                      : filter === "recent" && !search
                        ? "Open a tool and it will appear here."
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
          </>
        )}
        <p className="sr-only" role="status" aria-live="polite">
          {selectedTool
            ? `${selectedWorkspace?.title ?? selectedTool.title} workspace open.`
            : "No tool selected."}
        </p>
        {!selectedTool && (
          <footer className="command-status-bar" role="status">
            <span>{UtilityRegistry.length} tools</span>
            <span>On-device processing</span>
          </footer>
        )}
      </main>
      <Dialog open={shortcutsOpen} onOpenChange={setShortcutsOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Keyboard shortcuts</DialogTitle>
            <DialogDescription>Use these shortcuts to navigate and edit.</DialogDescription>
          </DialogHeader>
          <dl className="grid grid-cols-[1fr_auto] items-center gap-x-8 gap-y-3 text-sm">
            <dt>Search tools</dt><dd><kbd>⌘ K / Ctrl K</kbd></dd>
            <dt>Open file</dt><dd><kbd>⌘ O / Ctrl O</kbd></dd>
            <dt>Undo</dt><dd><kbd>⌘ Z / Ctrl Z</kbd></dd>
            <dt>Redo</dt><dd><kbd>⇧ ⌘ Z / Ctrl Shift Z</kbd></dd>
            <dt>Export</dt><dd><kbd>⌘ E / Ctrl E</kbd></dd>
          </dl>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShortcutsOpen(false)}>Close</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
    <SettingsPanel />
    </Dialog>
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
