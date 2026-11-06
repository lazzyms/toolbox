import { invoke } from "@tauri-apps/api/core";
import { exampleOutputName } from "../../../components/RunDestination";
import type { OcrMode, ToolResult } from "../../../contracts";
import { ocrRequest } from "../../../ocrRequest";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { formatPageRange, inspectedPages, type ConversionTool } from "../tool";

type Options = { mode: OcrMode };

const modeHints: Record<OcrMode, string> = {
  searchablePdf: "Looks exactly like the scan, with text you can select and search.",
  text: "Plain text, page by page. Pages that already have text add it as it is, so the file covers every page in scope.",
};

export const ocr: ConversionTool<Options> = {
  initialOptions: { mode: "searchablePdf" },
  // Extract from PDF hands scans over because it couldn't read their text, so OCR keeps that ask.
  handoffOptions: { mode: "text" },
  inspectsPdf: true,
  outputPreview: ({ mode }) => `Output preview unavailable. Export recognizes the text on each page and saves it as ${mode === "text" ? "a text file" : "a searchable PDF"}.`,
  OptionsPanel: ({ options, setOptions }) => (
    <div className="workspace-field">
      <Label id="ocr-save-as-label">Save as</Label>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        aria-labelledby="ocr-save-as-label"
        value={options.mode}
        onValueChange={(value) => {
          if (value) setOptions({ ...options, mode: value as OcrMode });
        }}
      >
        <ToggleGroupItem value="searchablePdf">Searchable PDF</ToggleGroupItem>
        <ToggleGroupItem value="text">Text file</ToggleGroupItem>
      </ToggleGroup>
      <p className="workspace-panel-copy">{modeHints[options.mode]}</p>
    </div>
  ),
  exampleName: (inputPath, utility, { mode }) => exampleOutputName(inputPath, "ocr_pdf", utility.capability.outputSuffix, mode === "text" ? "txt" : "pdf"),
  request: (paths, outputLocation, { runId }, { mode }, selection) => {
    const pages = inspectedPages(selection);
    // Every page picked means no range, so each file in a batch is read in full.
    const everyPage = selection.document && pages?.length === selection.document.pages.length;
    const pageRange = !everyPage && pages?.length ? formatPageRange(pages) : selection.pageRange || null;
    return invoke<ToolResult>("ocr_pdf", { runId, request: ocrRequest(paths, outputLocation, pageRange, mode) });
  },
};
