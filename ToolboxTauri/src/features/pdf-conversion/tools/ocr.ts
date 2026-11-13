import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { ocrRequest } from "../../../ocrRequest";
import { formatPageRange, inspectedPages, type ConversionTool } from "../tool";

export const ocr: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. OCR is unavailable in this build.",
  request: (paths, outputLocation, { runId }, _options, selection) => {
    const pages = inspectedPages(selection);
    const pageRange = pages?.length ? formatPageRange(pages) : selection.pageRange || null;
    return invoke<ToolResult>("ocr_pdf", { runId, request: ocrRequest(paths, outputLocation, pageRange) });
  },
};
