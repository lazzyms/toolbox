import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { ocrRequest } from "../../../ocrRequest";
import { formatPageRange, inspectedPages, type ConversionTool } from "../tool";

export const ocr: ConversionTool = {
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. Export recognizes the text on each page and saves it as a text file.",
  request: (paths, outputLocation, { runId }, _options, selection) => {
    const pages = inspectedPages(selection);
    // Every page picked means no range, so each file in a batch is read in full.
    const everyPage = selection.document !== null && selection.selectedPages.length === selection.document.pages.length;
    const pageRange = pages && pages.length > 0 && !everyPage ? formatPageRange(pages) : selection.pageRange || null;
    return invoke<ToolResult>("ocr_pdf", { runId, request: ocrRequest(paths, outputLocation, pageRange) });
  },
};
