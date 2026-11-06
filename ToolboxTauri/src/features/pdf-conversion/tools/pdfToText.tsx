import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { inspectedPages, type ConversionTool } from "../tool";

export const pdfToText: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. Export creates a text file from the selected pages.",
  OptionsPanel: () => (
    <p className="workspace-note">
      Selectable text is extracted in page order. Scanned PDFs are reported as needing OCR.
    </p>
  ),
  request: (paths, outputLocation, { runId }, _options, selection) => invoke<ToolResult>("pdf_to_text", { runId,
    request: { paths, pages: inspectedPages(selection), outputLocation },
  }),
};
