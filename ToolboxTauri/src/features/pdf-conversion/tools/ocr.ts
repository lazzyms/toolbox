import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { inspectedPages, type ConversionTool } from "../tool";

export const ocr: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. OCR is unavailable in this build.",
  request: (paths, outputLocation, { runId }, _options, selection) => invoke<ToolResult>("ocr_pdf", { runId,
    request: { paths, pages: inspectedPages(selection), outputLocation },
  }),
};
