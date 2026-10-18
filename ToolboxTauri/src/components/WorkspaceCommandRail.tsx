import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { Button } from "@/components/ui/button";
import type { AtomicToolId, ToolDefinition } from "../contracts";
import { TablerIcon } from "./TablerIcon";

interface WorkspaceCommandRailProps {
  actions: ToolDefinition[];
  activeId: AtomicToolId;
  onSelect: (id: AtomicToolId) => void;
  label: string;
  disabled?: boolean;
}

const designIconByToolId: Record<AtomicToolId, string> = {
  "lock-unlock": "remove-password",
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

type ToolbarOrientation = "horizontal" | "vertical";

const getToolbarOrientation = (): ToolbarOrientation =>
  typeof window !== "undefined" && window.matchMedia("(max-width: 980px)").matches
    ? "horizontal"
    : "vertical";

const handleToolbarKeyDown = (
  event: KeyboardEvent<HTMLDivElement>,
  orientation: ToolbarOrientation,
) => {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const buttons = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"),
  );
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (index < 0 || buttons.length === 0) return;
  const forward = orientation === "vertical" ? "ArrowDown" : "ArrowRight";
  const backward = orientation === "vertical" ? "ArrowUp" : "ArrowLeft";
  const nextIndex = event.key === forward ? (index + 1) % buttons.length
    : event.key === backward ? (index - 1 + buttons.length) % buttons.length
      : event.key === "Home" ? 0
        : event.key === "End" ? buttons.length - 1
          : -1;
  if (nextIndex < 0) return;
  event.preventDefault();
  buttons.forEach((button, buttonIndex) => {
    button.tabIndex = buttonIndex === nextIndex ? 0 : -1;
  });
  buttons[nextIndex].focus();
  buttons[nextIndex].scrollIntoView({ block: "nearest", inline: "nearest" });
};

/**
 * Tool rail for a workspace view. Switching tools keeps the view mounted, so
 * the open files and selection stay in place. This is intentionally a
 * navigation rail, not a tab list.
 */
export const WorkspaceCommandRail = ({
  actions,
  activeId,
  onSelect,
  label,
  disabled = false,
}: WorkspaceCommandRailProps) => {
  const [orientation, setOrientation] = useState<ToolbarOrientation>(getToolbarOrientation);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const updateOrientation = () => setOrientation(getToolbarOrientation());
    window.addEventListener("resize", updateOrientation);
    return () => window.removeEventListener("resize", updateOrientation);
  }, []);

  useEffect(() => {
    listRef.current?.querySelector('[aria-pressed="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeId, orientation]);

  return (
    <nav className="workspace-command-rail" aria-label={label}>
      <div
        ref={listRef}
        className="workspace-command-list"
        role="toolbar"
        aria-label={label}
        aria-orientation={orientation}
        onKeyDown={(event) => handleToolbarKeyDown(event, orientation)}
      >
        {actions.map((tool) => {
          const unavailable = tool.capability.nativeAvailability === "unavailable";
          return (
            <Button
              variant={activeId === tool.id ? "secondary" : "ghost"}
              size="sm"
              key={tool.id}
              type="button"
              className="workspace-command"
              aria-pressed={activeId === tool.id}
              tabIndex={activeId === tool.id ? 0 : -1}
              aria-label={tool.title}
              title={unavailable ? `${tool.title} is unavailable in this build` : tool.blurb}
              disabled={unavailable || disabled}
              onClick={() => onSelect(tool.id)}
            >
              <span className="workspace-command-icon" aria-hidden="true">
                <TablerIcon name={designIconByToolId[tool.id]} />
              </span>
              <span className="workspace-command-copy">
                {tool.title}
                {unavailable && <small>Unavailable</small>}
              </span>
            </Button>
          );
        })}
      </div>
    </nav>
  );
};
