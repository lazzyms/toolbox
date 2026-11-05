import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { ConversionTool } from "../tool";

export const extractPages: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. Export creates a PDF containing the selected pages.",
  OptionsPanel: ({ pageRange, choosePageRange }) => (
    <div className="workspace-field">
      <Label htmlFor="extract-pages-range">Pages or ranges</Label>
      <Input id="extract-pages-range" aria-label="Page numbers or ranges" value={pageRange} onChange={(event) => choosePageRange(event.target.value)} placeholder="1-3, 7" />
    </div>
  ),
  request: (paths, outputLocation, { runId }, _options, { selectedPages, pageRange }) => invoke<ToolResult>("extract_pdf_pages", { runId,
    request: {
      paths,
      pages: selectedPages,
      pageRanges: pageRange || null,
      outputLocation,
    },
  }),
};
