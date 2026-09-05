import { useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { UtilityRegistry } from "../registry";
import type { ToolDefinition } from "../contracts";
import { TablerIcon } from "../components/TablerIcon";
import { PDFUnlockView } from "./PDFUnlockView";
import { PDFProtectView } from "./PDFProtectView";
import { PDFEditView } from "./PDFEditView";
import { ImageCompressView } from "./ImageCompressView";
import { ImageConvertView } from "./ImageConvertView";
import { PlannedToolView } from "./PlannedToolView";
import { PDFCropView, PDFOrganizeView, PDFSignView } from "./PDFPageToolView";
import { PDFSimpleToolView } from "./PDFSimpleToolView";
import { PDFSelectionView } from "./PDFSelectionView";
import { PDFPathsView } from "./PDFPathsView";
import { PDFConversionView } from "./PDFConversionView";
import { VisionView } from "./VisionView";
import { ImageGeometryView } from "./ImageGeometryView";
import { ImageEffectView } from "./ImageEffectView";
import { ImageFormatView } from "./ImageFormatView";
import { ImageMetadataView } from "./ImageMetadataView";
import { SettingsPanel } from "./SettingsPanel";

const views = {
  "pdf-unlock": PDFUnlockView,
  "pdf-protect": PDFProtectView,
  "pdf-edit": PDFEditView,
  "pdf-crop": PDFCropView,
  "pdf-sign": PDFSignView,
  "pdf-organize": PDFOrganizeView,
  "pdf-page-numbers": (props) => (
    <PDFSimpleToolView {...props} mode="pageNumbers" />
  ),
  "pdf-watermark": (props) => <PDFSimpleToolView {...props} mode="watermark" />,
  "pdf-compress": (props) => <PDFSimpleToolView {...props} mode="compress" />,
  "pdf-remove-pages": (props) => <PDFSelectionView {...props} mode="remove" />,
  "pdf-extract-pages": (props) => (
    <PDFSelectionView {...props} mode="extract" />
  ),
  "pdf-merge": (props) => <PDFPathsView {...props} mode="merge" />,
  "pdf-split": (props) => <PDFPathsView {...props} mode="split" />,
  "pdf-to-images": (props) => <PDFConversionView {...props} mode="to-images" />,
  "pdf-to-text": (props) => <PDFConversionView {...props} mode="to-text" />,
  "pdf-image-extract": (props) => (
    <PDFConversionView {...props} mode="extract-images" />
  ),
  "images-to-pdf": (props) => (
    <PDFConversionView {...props} mode="images-to-pdf" />
  ),
  "pdf-ocr": (props) => <VisionView {...props} mode="ocr" />,
  "image-blur-faces": (props) => <VisionView {...props} mode="faces" />,
  "image-remove-bg": (props) => <VisionView {...props} mode="background" />,
  "image-resize": (props) => <ImageGeometryView {...props} mode="resize" />,
  "image-rotate": (props) => <ImageGeometryView {...props} mode="rotate" />,
  "image-crop": (props) => <ImageGeometryView {...props} mode="crop" />,
  "image-watermark": (props) => <ImageEffectView {...props} mode="watermark" />,
  "image-tone": (props) => <ImageEffectView {...props} mode="tone" />,
  "image-icons": (props) => <ImageFormatView {...props} mode="icons" />,
  "gif-create": (props) => <ImageFormatView {...props} mode="gif-create" />,
  "gif-extract": (props) => <ImageFormatView {...props} mode="gif-extract" />,
  "tiff-pages": (props) => <ImageFormatView {...props} mode="tiff" />,
  "image-metadata": ImageMetadataView,
  "image-compress": ImageCompressView,
  "image-convert": ImageConvertView,
  planned: PlannedToolView,
} satisfies Record<
  ToolDefinition["view"],
  ComponentType<{ utility: ToolDefinition }>
>;

const designIconByToolId: Record<string, string> = {
  "pdf-unlock": "remove-password",
  "pdf-page-numbers": "page-numbers",
  "pdf-merge": "merge-pdf",
  "pdf-watermark": "watermark-pdf",
  "pdf-crop": "crop-pdf",
  "pdf-edit": "edit-pdf",
  "pdf-protect": "protect-pdf",
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

export const MainPage = () => {
  const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [filter, setFilter] = useState<
    "all" | "PDF" | "Images" | "Documents" | "favorites" | "recent"
  >("all");
  const [search, setSearch] = useState("");
  const searchInputRef = useRef<HTMLInputElement>(null);
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
  const recentPreviewTools = recentTools.slice(0, 3);
  const visibleTools = useMemo(
    () => {
      const source = filter === "recent" ? recentTools : UtilityRegistry;
      return source.filter(
        (tool) =>
          (filter === "all" ||
            filter === "recent" ||
            tool.category === filter ||
            (filter === "favorites" && favorites.includes(tool.id))) &&
          `${tool.title} ${tool.blurb}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      );
    },
    [filter, search, favorites, recentTools],
  );
  const openTool = (tool: ToolDefinition) => {
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
  const selectLibrary = (
    nextFilter: "all" | "PDF" | "Images" | "Documents" | "favorites" | "recent",
  ) => {
    setFilter(nextFilter);
    setSearch("");
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
  const workspaceDescription =
    filter === "favorites"
      ? "Your saved tools, ready whenever you need them."
      : filter === "recent"
        ? "The tools you opened most recently on this device."
        : "Private utilities for PDFs, images, documents, and everyday file work.";
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
            <button
              key={item}
              type="button"
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
            </button>
          ))}
        </nav>
        <p className="eyebrow category-label">Categories</p>
        <nav className="rail-nav" aria-label="Tool categories">
          {(["PDF", "Images", "Documents"] as const).map((item) => (
            <button
              key={item}
              type="button"
              aria-current={filter === item ? "page" : undefined}
              onClick={() => selectLibrary(item)}
              aria-label={item}
            >
              {item}
            </button>
          ))}
        </nav>
        <p className="eyebrow category-label">Quick access</p>
            <nav className="quick-tools">
              {(recentPreviewTools.length
                ? recentPreviewTools
                : UtilityRegistry.slice(0, 4)
              ).map(
                (tool) => (
              <button
                key={tool.id}
                type="button"
                onClick={() => openTool(tool)}
                aria-current={selectedTool?.id === tool.id ? "page" : undefined}
                >
                <span className="card-icon quick-tool-icon">
                  <TablerIcon name={iconName(tool)} />
                </span>
                {tool.shortTitle}
              </button>
            ),
          )}
        </nav>
        <div className="rail-spacer" />
        <button
          type="button"
          className="settings-link"
          aria-current={settingsOpen ? "page" : undefined}
          onClick={() => setSettingsOpen(true)}
        >
          Settings
        </button>
        <p className="privacy-note">
          <strong>Private by default</strong>Files stay on this device.
        </p>
      </aside>
      <main className="command-center" aria-label="Tool detail">
        <header className="topbar">
          <span className="breadcrumb">
            Toolbox <span>/ Command center</span>
          </span>
          <span className="privacy-pill">
            <i /> On-device workspace
          </span>
        </header>
        {selectedTool ? (
          <section className="tool-workspace" id="tool-detail">
            <button
              className="back-link"
              type="button"
              onClick={() => setSelectedTool(null)}
            >
              ← All tools
            </button>
            <div className="tool-workspace-card">
              <div className="tool-workspace-kicker">
                <span className="card-icon">
                  <TablerIcon name={iconName(selectedTool)} />
                </span>
                {selectedTool.category} utility
              </div>
              <div className="tool-workspace-heading">
                <div>
                  <div className="workspace-title">{selectedTool.title}</div>
                  <p>{selectedTool.blurb}</p>
                </div>
                <button
                  className="favorite-button"
                  type="button"
                  aria-pressed={favorites.includes(selectedTool.id)}
                  onClick={() => toggleFavorite(selectedTool.id)}
                >
                  {favorites.includes(selectedTool.id) ? "★ Saved" : "☆ Save"}
                </button>
              </div>
              <div className="tool-view">
                <ViewFor utility={selectedTool} />
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
                <p>{workspaceDescription}</p>
              </div>
              <label className="search-box">
                <span>⌕</span>
                <input
                  ref={searchInputRef}
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder="Find a tool or action"
                  aria-label="Search tools"
                />
                <kbd>⌘ K</kbd>
              </label>
            </div>
            {filter === "all" && recentPreviewTools.length > 0 && (
              <section className="recent-section">
                <div className="section-heading">
                  <h2>Pick up where you left off</h2>
                  <span>Stored on this device</span>
                </div>
                <div className="recent-grid">
                  {recentPreviewTools.map((tool) => (
                    <button
                      key={tool.id}
                      type="button"
                      className="recent-card"
                      onClick={() => openTool(tool)}
                    >
                      <span className="card-icon">
                        <TablerIcon name={iconName(tool)} />
                      </span>
                      <span>
                        <strong>{tool.title}</strong>
                        <small>{tool.category} utility</small>
                      </span>
                      <em>Open →</em>
                    </button>
                  ))}
                </div>
              </section>
            )}
            <section>
              <div className="section-heading">
                <h2>Tool library</h2>
                <span>{visibleTools.length} tools</span>
              </div>
              <div className="filter-row">
                {(["all", "PDF", "Images", "Documents", "favorites"] as const).map(
                  (item) => (
                    <button
                      key={item}
                      type="button"
                      aria-pressed={filter === item}
                      onClick={() => selectLibrary(item)}
                    >
                      {item === "all"
                        ? "All tools"
                        : item[0].toUpperCase() + item.slice(1)}
                    </button>
                  ),
                )}
              </div>
              <div className="tool-grid">
                {visibleTools.map((tool) => (
                  <article className="tool-card" key={tool.id}>
                    <div className="tool-card-top">
                      <span className="card-icon">
                        <TablerIcon name={iconName(tool)} />
                      </span>
                      <button
                        className="favorite-button"
                        type="button"
                        aria-label={
                          favorites.includes(tool.id)
                            ? `Remove ${tool.title} from favorites`
                            : `Add ${tool.title} to favorites`
                        }
                        aria-pressed={favorites.includes(tool.id)}
                        onClick={() => toggleFavorite(tool.id)}
                      >
                        {favorites.includes(tool.id) ? "★" : "☆"}
                      </button>
                    </div>
                    <h3>{tool.title}</h3>
                    <p>{tool.blurb}</p>
                    <footer>
                      <span>{tool.category}</span>
                      <button
                        type="button"
                        onClick={() => openTool(tool)}
                        aria-label={`Open ${tool.title}`}
                      >
                        Open tool →
                      </button>
                    </footer>
                  </article>
                ))}
              </div>
              {visibleTools.length === 0 && (
                <div className="empty-state">
                  <strong>
                    {filter === "favorites" && !search
                      ? "No favorite tools yet"
                      : filter === "recent" && !search
                        ? "No recent tools yet"
                        : "No matching tools"}
                  </strong>
                  <span>
                    {filter === "favorites" && !search
                      ? "Save a tool with the star to keep it here."
                      : filter === "recent" && !search
                        ? "Open a tool and it will appear here."
                        : "Try another term or clear the current filters."}
                  </span>
                  <button
                    type="button"
                    onClick={() => selectLibrary("all")}
                  >
                    Browse all tools
                  </button>
                </div>
              )}
            </section>
          </>
        )}
        <p className="sr-only" role="status" aria-live="polite">
          {selectedTool
            ? `${selectedTool.title} selected.`
            : "No tool selected."}
        </p>
      </main>
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </div>
  );
};

const ViewFor = ({ utility }: { utility: ToolDefinition }) => {
  const View = views[utility.view];
  return <View utility={utility} />;
};
