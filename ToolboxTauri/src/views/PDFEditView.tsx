import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

type EditMode = "text" | "note" | "highlight" | "shape";

export const PDFEditView = ({ utility }: { utility: ToolDefinition }) => {
  const [mode, setMode] = useState<EditMode>("text");
  const [text, setText] = useState("");
  const [pages, setPages] = useState("");
  return (
    <ToolScaffold
      utility={utility}
      onRun={(paths) =>
        invoke<ToolResult>("edit_pdf", {
          request: {
            paths,
            mode,
            text,
            pages: pages
              ? pages.split(",").map((page) => Number(page.trim()) - 1)
              : null,
            rectangle: { x: 72, y: 650, width: 240, height: 72 },
            outputLocation: "alongsideInput",
          },
        })
      }
    >
      {({ files, run, loading }) => (
        <div className="space-y-4">
          <div
            className="flex flex-wrap gap-2"
            role="group"
            aria-label="Edit mode"
          >
            {(["text", "note", "highlight", "shape"] as const).map((item) => (
              <button
                key={item}
                type="button"
                aria-pressed={mode === item}
                onClick={() => setMode(item)}
                className="rounded border px-3 py-2 text-sm"
              >
                {item[0].toUpperCase() + item.slice(1)}
              </button>
            ))}
          </div>
          {mode !== "shape" && (
            <label className="block text-sm">
              {mode === "highlight" ? "Highlighted text" : "Text or note"}
              <input
                aria-label="Edit text"
                value={text}
                onChange={(event) => setText(event.target.value)}
                className="ml-3 rounded border px-3 py-2"
                placeholder={
                  mode === "note" ? "Add a note" : "Type text to place"
                }
              />
            </label>
          )}
          <label className="block text-sm">
            Apply to pages
            <input
              aria-label="Edit pages"
              value={pages}
              onChange={(event) => setPages(event.target.value)}
              className="ml-3 rounded border px-3 py-2"
              placeholder="All pages, or 1, 3"
            />
          </label>
          <button
            type="button"
            disabled={
              loading ||
              files.length === 0 ||
              (mode !== "shape" && !text.trim())
            }
            onClick={run}
            className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50"
          >
            Edit PDF
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};
