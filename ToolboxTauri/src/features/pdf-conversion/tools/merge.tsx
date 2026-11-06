import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

export const merge: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export merges the selected PDFs in file order.",
  exportBlocked: (_options, files) => files.length <= 1,
  exportNote: (files) => (files.length < 2 ? (files.length === 1 ? "Merge needs at least 2 PDFs." : "Add at least 2 PDFs.") : null),
  OptionsPanel: ({ onSelectTool }) => (
    <>
      <div className="workspace-field">
        <span className="text-sm font-medium">Order</span>
        <p className="workspace-note">Files join top to bottom, every page of each. A file can be listed twice.</p>
      </div>
      <Card className="py-0" role="note" aria-label="Organize Pages">
        <CardContent className="flex flex-col items-start gap-2 p-3">
          <p className="workspace-note">Need only some pages, or a page order? Use Organize Pages.</p>
          <Button variant="outline" size="sm" disabled={!onSelectTool} onClick={() => onSelectTool?.("pdf-organize")}>Open Organize Pages</Button>
        </CardContent>
      </Card>
    </>
  ),
  request: (paths, outputLocation, { outputPath, runId }) => invoke<ToolResult>("merge_pdfs", { runId,
    request: { paths, outputLocation, outputPath },
  }),
};
