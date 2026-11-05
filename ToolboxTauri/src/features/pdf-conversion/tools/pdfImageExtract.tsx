import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { inspectedPages, type ConversionTool } from "../tool";

export const pdfImageExtract: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. Export extracts embedded JPEG images from the selected pages.",
  OptionsPanel: () => (
    <p className="workspace-note">
      Embedded JPEG images are saved at their original resolution when the PDF contains them.
    </p>
  ),
  request: (paths, outputLocation, { runId }, _options, selection) => invoke<ToolResult>("extract_pdf_images", { runId,
    request: { paths, pages: inspectedPages(selection), outputLocation },
  }),
};
